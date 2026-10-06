//! Properties of `spec/kernel.md` section 8, checked on every step of 10,000
//! generated traces against the native kernel.
//!
//! Each trace is generated once and then replayed from sequence 1 by this
//! file's own loop, which sends every request twice. The checks per step:
//!
//! - the same request gives the same result, bytes and failure text included;
//! - K1: the replay reproduces every snapshot, command, and diagnostic byte
//!   for byte, and every refusal;
//! - K2: a result or resolution that does not apply changes nothing but the
//!   sequence and the clock, one that applies is applied once, and each
//!   `(node, visit)` record is written at most once;
//! - K3: a terminal state changes in nothing but the sequence and the clock;
//! - K4: every activation has its own invocation ID, retries keep it, and no
//!   command ID repeats; all are the derivations of `digest::ids`;
//! - K6: a decision that leaves the run in `needs-intervention` has no command;
//! - everything emitted is canonical JSON and `snapshotDigest` is its digest.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use runspore_kernel::NativeReducer;
use runspore_trace::compile::{CompiledExpect, CompiledTrace};
use runspore_trace::format::Payload;
use runspore_trace::generate::DEFAULT_MAX_STEPS;
use runspore_trace::{bless, compile, generate, Event, Step, Trace};
use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::model::{
    code, command_kind, event_kind, ActionPolicy, ActivityResult, InvocationPhase,
    InvocationResolved, Resolution, RunStatus, State, Workflow, MAX_BUFFERED_SIGNALS,
};
use runspore_types::reducer::{Decision, Envelope, Identity, Limits, Reducer, TransitionRequest};
use runspore_types::Value;

const SEEDS: u64 = 10_000;

type Counts = BTreeMap<String, u64>;

fn count(counts: &mut Counts, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

/// Returns an error naming what failed instead of panicking, so the caller can
/// add the seed and the step.
macro_rules! ensure {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}

/// What the checks remember about one run.
#[derive(Default)]
struct Run {
    state: Option<State>,
    /// Invocation ID per activation ID, from `activity.schedule` commands.
    invocations: BTreeMap<String, String>,
    command_ids: BTreeSet<String>,
    /// `(node, visit)` records written to `nodes` so far.
    records: BTreeSet<(String, u32)>,
    /// `(invocation, attempt)` pairs whose result was applied.
    results: BTreeSet<(String, u32)>,
    /// `(invocation, attempt)` pairs resolved by an operator.
    resolutions: BTreeSet<(String, u32)>,
    /// Result payloads sent so far, applied or not.
    result_payloads: BTreeSet<String>,
}

fn requests(trace: &CompiledTrace) -> impl Iterator<Item = TransitionRequest> + '_ {
    trace.steps.iter().map(|step| TransitionRequest {
        identity: Identity {
            package_digest: trace.identity.package_digest.clone(),
            kernel_digest: trace.identity.kernel_digest.clone(),
            semantics_version: trace.identity.semantics_version.clone(),
            codec_version: trace.identity.codec_version.clone(),
        },
        graph: trace.workflow.as_bytes().to_vec(),
        snapshot: None,
        input_event: Envelope {
            event_id: step.event.event_id.clone(),
            sequence: step.event.sequence,
            accepted_at_ms: step.event.accepted_at_ms,
            kind: step.event.kind.clone(),
            payload: step.event.payload.as_bytes().to_vec(),
        },
        frozen_limits: Limits {
            microsteps: trace.limits.microsteps,
            expression_operations: trace.limits.expression_operations,
            max_state_bytes: trace.limits.max_state_bytes,
            max_command_count: trace.limits.max_command_count,
        },
    })
}

fn check_seed(seed: u64, counts: &mut Counts) -> Result<(), String> {
    let trace = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS).map_err(|e| e.to_string())?;
    check_trace(&trace, counts).map_err(|message| format!("seed {seed}, {message}"))
}

/// Replays a trace recorded from the native kernel and checks every step.
fn check_trace(trace: &Trace, counts: &mut Counts) -> Result<(), String> {
    let trace = compile(trace).map_err(|e| e.to_string())?;
    let workflow: Workflow =
        canonical::decode(trace.workflow.as_bytes()).map_err(|e| e.to_string())?;
    let mut run = Run::default();
    let mut snapshot: Option<Vec<u8>> = None;
    count(counts, "traces");
    for (index, (step, mut request)) in trace.steps.iter().zip(requests(&trace)).enumerate() {
        request.snapshot = snapshot.clone();
        let outcome = NativeReducer.transition(&request);
        let checked = (|| {
            ensure!(
                NativeReducer.transition(&request) == outcome,
                "the same request gave two results"
            );
            count(counts, "steps");
            count(counts, &format!("event {}", request.input_event.kind));
            let decision = match (&step.expect, &outcome) {
                (CompiledExpect::Failure(expected), Err(failure)) => {
                    ensure!(
                        expected.failure.kind == failure.kind
                            && expected.failure.code == failure.code,
                        "replay refused with {failure}, generation with {:?}",
                        expected.failure
                    );
                    count(counts, &format!("refused {}", failure.code));
                    return Ok(None);
                }
                (CompiledExpect::Decision(expected), Ok(decision)) => {
                    reproduces(expected, decision)?;
                    decision
                }
                _ => return Err("replay and generation disagree on decision or failure".into()),
            };
            is_canonical(decision)?;
            let next: State = canonical::decode(&decision.snapshot).map_err(|e| e.to_string())?;
            check_step(
                &workflow,
                &mut run,
                &request.input_event,
                decision,
                &next,
                counts,
            )?;
            run.state = Some(next);
            Ok(Some(decision.snapshot.clone()))
        })()
        .map_err(|message: String| {
            format!("step {index} (event `{}`): {message}", step.event.event_id)
        })?;
        if let Some(accepted) = checked {
            snapshot = Some(accepted);
        }
    }
    Ok(())
}

/// K1: the replayed decision is the generated one, byte for byte.
fn reproduces(
    expected: &runspore_trace::compile::CompiledDecision,
    actual: &Decision,
) -> Result<(), String> {
    ensure!(
        expected.snapshot.as_bytes() == actual.snapshot,
        "snapshot bytes differ on replay"
    );
    ensure!(
        expected.snapshot_digest == actual.snapshot_digest,
        "snapshot digest differs on replay"
    );
    ensure!(
        expected.decision_digest == digest::decision(actual),
        "decision digest differs on replay"
    );
    ensure!(
        expected.commands.len() == actual.commands.len()
            && expected
                .commands
                .iter()
                .zip(&actual.commands)
                .all(|(e, a)| {
                    e.command_id == a.command_id
                        && e.activation_id == a.activation_id
                        && e.kind == a.kind
                        && e.payload.as_bytes() == a.payload
                }),
        "commands differ on replay"
    );
    ensure!(
        expected.diagnostics.len() == actual.diagnostics.len()
            && expected
                .diagnostics
                .iter()
                .zip(&actual.diagnostics)
                .all(|(e, a)| {
                    e.code == a.code && e.node_id == a.node_id && e.details.as_bytes() == a.details
                }),
        "diagnostics differ on replay"
    );
    Ok(())
}

fn is_canonical(decision: &Decision) -> Result<(), String> {
    let canonical = |what: &str, bytes: &[u8]| -> Result<Value, String> {
        let value = canonical::parse_canonical(bytes).map_err(|e| format!("{what}: {e}"))?;
        ensure!(
            canonical::to_vec(&value).is_ok_and(|again| again == bytes),
            "{what} does not round-trip"
        );
        Ok(value)
    };
    canonical("snapshot", &decision.snapshot)?;
    ensure!(
        decision.snapshot_digest == digest::state(&decision.snapshot),
        "snapshotDigest is not digest::state(snapshot)"
    );
    for command in &decision.commands {
        canonical("command payload", &command.payload)?;
    }
    for diagnostic in &decision.diagnostics {
        canonical("diagnostic details", &diagnostic.details)?;
    }
    Ok(())
}

/// `prev` with the sequence and the clock of `next`: what a state must equal
/// after an event that changed nothing.
fn only_clock_moved(prev: &State, next: &State) -> bool {
    let mut moved = prev.clone();
    moved.last_sequence = next.last_sequence;
    moved.last_accepted_at_ms = next.last_accepted_at_ms;
    moved == *next
}

fn codes(decision: &Decision) -> Vec<&str> {
    decision
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn check_step(
    workflow: &Workflow,
    run: &mut Run,
    event: &Envelope,
    decision: &Decision,
    next: &State,
    counts: &mut Counts,
) -> Result<(), String> {
    count(counts, &format!("status {}", next.status.as_str()));
    for code in codes(decision) {
        count(counts, &format!("diagnostic {code}"));
    }
    ensure!(
        next.last_sequence == event.sequence,
        "lastSequence is not the event's"
    );
    let prev = run.state.as_ref();
    ensure!(
        prev.is_none_or(|prev| next.last_accepted_at_ms >= prev.last_accepted_at_ms),
        "logical time went back"
    );

    // K3
    if let Some(prev) = prev.filter(|prev| prev.status.is_terminal()) {
        ensure!(
            only_clock_moved(prev, next),
            "a terminal state changed (K3)"
        );
        ensure!(
            decision.commands.is_empty(),
            "a terminal state emitted a command"
        );
        ensure!(
            codes(decision) == [code::EVENT_IGNORED_TERMINAL],
            "an event after terminal was not reported as ignored"
        );
        count(counts, "category: event after terminal");
        return Ok(());
    }

    // K6
    if next.status == RunStatus::NeedsIntervention {
        ensure!(
            decision.commands.is_empty(),
            "needs-intervention with a command (K6)"
        );
    }

    // K4
    for command in &decision.commands {
        count(counts, &format!("command {}", command.kind));
        let payload: Value =
            canonical::parse_canonical(&command.payload).map_err(|e| e.to_string())?;
        let invocation_id = payload["invocationId"].as_str().unwrap_or_default();
        let attempt = payload["attempt"].as_u64().unwrap_or_default() as u32;
        let activation_id = &command.activation_id;
        ensure!(
            run.command_ids.insert(command.command_id.clone()),
            "command ID {} repeats (K4)",
            command.command_id
        );
        ensure!(
            command.command_id == ids::command(&next.tenant, &next.run_id, activation_id, attempt),
            "command ID is not ids::command for attempt {attempt}"
        );
        if command.kind == command_kind::ACTIVITY_SCHEDULE {
            ensure!(attempt == 1, "a schedule command for attempt {attempt}");
            ensure!(
                invocation_id == ids::invocation(&next.tenant, &next.run_id, activation_id),
                "invocation ID is not ids::invocation"
            );
            ensure!(
                !run.invocations.values().any(|known| known == invocation_id),
                "invocation ID {invocation_id} serves two activations (K4)"
            );
            ensure!(
                run.invocations
                    .insert(activation_id.clone(), invocation_id.to_string())
                    .is_none(),
                "activation {activation_id} was scheduled twice (K4)"
            );
        } else {
            ensure!(
                attempt > 1
                    && run.invocations.get(activation_id).map(String::as_str)
                        == Some(invocation_id),
                "a retry changed the invocation ID of {activation_id} (K4)"
            );
        }
    }
    if let Some(invocation) = &next.invocation {
        ensure!(
            run.invocations.get(&invocation.activation_id) == Some(&invocation.invocation_id),
            "the outstanding invocation was never scheduled"
        );
    }
    if let Some(position) = &next.position {
        ensure!(
            position.activation_id == ids::activation(&position.node_id, position.visit),
            "activation ID is not node/visit"
        );
    }

    // K2
    for (node, record) in &next.nodes {
        let unchanged = prev.is_some_and(|prev| prev.nodes.get(node) == Some(record));
        if !unchanged {
            ensure!(
                run.records.insert((node.clone(), record.visit)),
                "the record of {node}/{} was written twice (K2)",
                record.visit
            );
        }
    }
    let Some(prev) = prev else {
        return Ok(());
    };
    let outstanding = prev.invocation.as_ref();
    if event.kind == event_kind::ACTIVITY_RESULT {
        let result: ActivityResult =
            canonical::decode(&event.payload).map_err(|e| e.to_string())?;
        let repeated = !run
            .result_payloads
            .insert(String::from_utf8_lossy(&event.payload).into_owned());
        let applies = outstanding.is_some_and(|invocation| {
            invocation.invocation_id == result.invocation_id
                && invocation.attempt == result.attempt
                && invocation.state == InvocationPhase::Scheduled
        });
        if applies {
            ensure!(
                run.results.insert((result.invocation_id, result.attempt)),
                "a result was applied twice (K2)"
            );
            ensure!(
                !only_clock_moved(prev, next),
                "an applied result changed nothing"
            );
            count(counts, "category: result applied");
        } else {
            ensure!(
                only_clock_moved(prev, next) && decision.commands.is_empty(),
                "a result that does not apply changed the run (K2)"
            );
            ensure!(
                codes(decision) == [code::RESULT_STALE],
                "a result that does not apply was not reported stale"
            );
            count(
                counts,
                if repeated {
                    "category: duplicate result"
                } else {
                    "category: stale result"
                },
            );
        }
    } else if event.kind == event_kind::INVOCATION_RESOLVED {
        let resolved: InvocationResolved =
            canonical::decode(&event.payload).map_err(|e| e.to_string())?;
        let unknown = outstanding.filter(|invocation| {
            invocation.invocation_id == resolved.invocation_id
                && invocation.state == InvocationPhase::Unknown
        });
        let applies = unknown.is_some_and(|invocation| match &resolved.resolution {
            Resolution::Complete { outcome, .. } => workflow
                .actions
                .get(&invocation.action_id)
                .and_then(|action| serde_json::from_value::<ActionPolicy>(action.clone()).ok())
                .is_some_and(|policy| policy.outcomes.contains(outcome)),
            _ => true,
        });
        match unknown {
            Some(invocation) if applies => {
                ensure!(
                    run.resolutions
                        .insert((resolved.invocation_id, invocation.attempt)),
                    "a resolution was applied twice (K2)"
                );
                ensure!(
                    !only_clock_moved(prev, next),
                    "an applied resolution changed nothing"
                );
                count(counts, "category: resolution applied");
            }
            _ => {
                ensure!(
                    only_clock_moved(prev, next) && decision.commands.is_empty(),
                    "a resolution that does not apply changed the run (K2)"
                );
                ensure!(
                    codes(decision) == [code::RESOLVE_NOT_APPLICABLE],
                    "a resolution that does not apply was not reported"
                );
                count(counts, "category: resolution not applied");
            }
        }
    } else if event.kind == event_kind::SIGNAL_RECEIVED {
        for code in codes(decision) {
            if code == code::SIGNAL_OUTCOME_UNROUTED {
                count(counts, "category: unrouted signal");
            } else if code == code::SIGNAL_BUFFER_OVERFLOW {
                ensure!(
                    only_clock_moved(prev, next) && prev.signals.len() == MAX_BUFFERED_SIGNALS,
                    "a signal was dropped from a buffer that was not full, or changed the run"
                );
                count(counts, "category: signal buffer overflow");
            }
        }
        ensure!(
            next.signals.len() <= MAX_BUFFERED_SIGNALS,
            "more than {MAX_BUFFERED_SIGNALS} signals are buffered"
        );
        if next.signals.len() > prev.signals.len() {
            count(counts, "category: signal buffered");
        } else if next.nodes != prev.nodes {
            count(counts, "category: signal consumed");
        }
    }
    Ok(())
}

#[test]
fn kernel_properties_hold_on_every_step_of_ten_thousand_traces() {
    let started = Instant::now();
    let next = AtomicU64::new(0);
    let work = || {
        let mut counts = Counts::new();
        loop {
            let seed = next.fetch_add(1, Ordering::Relaxed);
            if seed >= SEEDS {
                return Ok(counts);
            }
            check_seed(seed, &mut counts)?;
        }
    };
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let results: Vec<Result<Counts, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });

    let mut counts = Counts::new();
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(part) => {
                for (key, n) in part {
                    *counts.entry(key).or_default() += n;
                }
            }
            Err(failure) => failures.push(failure),
        }
    }
    failures.sort();
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    println!(
        "{SEEDS} traces on {workers} threads in {:.1?}",
        started.elapsed()
    );
    for (key, n) in &counts {
        println!("{n:>8}  {key}");
    }
    assert_eq!(counts["traces"], SEEDS);
    assert!(counts["steps"] > SEEDS * 6, "{}", counts["steps"]);
    for category in [
        "category: result applied",
        "category: stale result",
        "category: duplicate result",
        "category: resolution applied",
        "category: resolution not applied",
        "category: unrouted signal",
        "category: signal buffered",
        "category: signal consumed",
        "category: event after terminal",
        "command activity.schedule",
        "command activity.retry",
        "status running",
        "status waiting",
        "status needs-intervention",
        "status completed",
        "status failed",
        "refused event.out-of-order",
        "refused event.unknown-kind",
        "refused event.invalid",
        "refused state.missing",
        "refused state.unexpected-start",
        "refused budget.microsteps",
        "refused budget.expression-operations",
        "refused budget.state-bytes",
        "refused budget.command-count",
    ] {
        assert!(
            counts.get(category).is_some_and(|n| *n >= 50),
            "{category}: {:?}",
            counts.get(category)
        );
    }
}

/// No generated trace is long enough to fill the signal buffer, so this one is
/// built by hand: a generated workflow, its start event, and more signals that
/// no node waits for than the buffer holds.
#[test]
fn kernel_properties_hold_when_the_signal_buffer_overflows() {
    let generated = (0..)
        .map(|seed| generate(seed, &NativeReducer, 1).unwrap())
        .find(|trace| {
            trace.limits.is_none()
                && matches!(
                    trace.steps[0].expect,
                    Some(runspore_trace::Expect::Decision(_))
                )
        })
        .unwrap();
    let mut trace = Trace::new("signal-overflow", "", generated.workflow.clone());
    trace.steps = generated.steps.clone();
    let start = trace.steps[0].event.accepted_at_ms;
    for n in 0..(MAX_BUFFERED_SIGNALS as u64 + 6) {
        let mut signal = Event::new(
            &format!("sig:overflow-{n}"),
            n + 2,
            start + n,
            event_kind::SIGNAL_RECEIVED,
            serde_json::json!({"name": "nobody.waits", "data": n}),
        );
        if n % 7 == 3 {
            signal.payload = Payload::Raw(r#"{"name":"nobody.waits"}"#.to_string());
        }
        trace.steps.push(Step::new(signal));
    }
    let trace = bless(&NativeReducer, &trace).unwrap();

    let mut counts = Counts::new();
    check_trace(&trace, &mut counts).unwrap_or_else(|message| panic!("{message}"));
    assert_eq!(
        counts["category: signal buffered"],
        MAX_BUFFERED_SIGNALS as u64
    );
    assert_eq!(counts["category: signal buffer overflow"], 6);
    assert_eq!(counts["steps"], MAX_BUFFERED_SIGNALS as u64 + 7);
}
