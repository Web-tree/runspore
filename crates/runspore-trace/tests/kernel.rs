//! The generator and the runner against the real kernel.

use std::collections::BTreeSet;

use runspore_kernel::NativeReducer;
use runspore_trace::compile::CompiledTrace;
use runspore_trace::format::{Expect, ExpectedDecision, Payload};
use runspore_trace::generate::{cross_check, DEFAULT_MAX_STEPS};
use runspore_trace::{compile, generate, run, run_compiled, Trace};

fn decisions(trace: &Trace) -> impl Iterator<Item = &ExpectedDecision> {
    trace.steps.iter().filter_map(|step| match &step.expect {
        Some(Expect::Decision(decision)) => Some(decision),
        _ => None,
    })
}

fn refusals(trace: &Trace) -> impl Iterator<Item = &str> {
    trace.steps.iter().filter_map(|step| match &step.expect {
        Some(Expect::Failure(failure)) => Some(failure.code.as_str()),
        _ => None,
    })
}

#[test]
fn generated_traces_replay_as_built_as_written_and_as_compiled() {
    for seed in 0..150 {
        let trace = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS).unwrap();
        run(&NativeReducer, &trace).unwrap_or_else(|error| panic!("seed {seed}: {error}"));

        let text = trace.to_json();
        let read = Trace::from_json(&text).unwrap();
        assert_eq!(read, trace, "seed {seed}");
        assert_eq!(read.to_json(), text, "seed {seed}");

        let compiled = serde_json::to_string(&compile(&read).unwrap()).unwrap();
        let compiled: CompiledTrace = serde_json::from_str(&compiled).unwrap();
        run_compiled(&NativeReducer, &compiled)
            .unwrap_or_else(|error| panic!("seed {seed}: {error}"));

        let again = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS).unwrap();
        assert_eq!(again, trace, "seed {seed}");
    }
}

#[test]
fn a_batch_of_seeds_cross_checks_in_parallel() {
    let totals = cross_check(
        &NativeReducer,
        &NativeReducer,
        1_000..1_500,
        DEFAULT_MAX_STEPS,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(totals.traces, 500);
    assert!(totals.steps > 3_000, "{totals:?}");
    assert!(
        totals.refused * 20 > totals.steps && totals.refused * 3 < totals.steps,
        "{totals:?}"
    );
}

#[test]
fn the_kernel_accepts_every_generated_workflow() {
    for seed in 0..600 {
        let trace = generate(seed, &NativeReducer, 3).unwrap();
        let rejected: Vec<&str> = refusals(&trace)
            .filter(|code| code.starts_with("workflow.") || code.starts_with("version."))
            .collect();
        assert!(rejected.is_empty(), "seed {seed}: {rejected:?}");
    }
}

#[test]
fn generated_runs_reach_every_part_of_the_kernel() {
    let mut statuses = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let mut refused = BTreeSet::new();
    let mut diagnostics = BTreeSet::new();
    let mut commands = BTreeSet::new();
    let mut run_errors = BTreeSet::new();
    let (mut buffered, mut backed_off, mut raw, mut steps) = (false, false, false, 0);
    for seed in 0..600 {
        let trace = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS).unwrap();
        assert!((1..=DEFAULT_MAX_STEPS).contains(&trace.steps.len()));
        steps += trace.steps.len();
        refused.extend(refusals(&trace).map(str::to_string));
        for step in &trace.steps {
            kinds.insert(step.event.kind.clone());
            raw |= matches!(step.event.payload, Payload::Raw(_));
        }
        for decision in decisions(&trace) {
            let snapshot = &decision.snapshot;
            statuses.insert(snapshot["status"].as_str().unwrap().to_string());
            buffered |= snapshot["signals"].as_array().is_some_and(|s| s.len() > 1);
            if let Some(code) = snapshot["result"]["error"]["code"].as_str() {
                run_errors.insert(code.to_string());
            }
            diagnostics.extend(decision.diagnostics.iter().map(|d| d.code.clone()));
            for command in &decision.commands {
                commands.insert(command.kind.clone());
                backed_off |= command.kind == "activity.retry"
                    && command.payload["notBeforeMs"] != snapshot["lastAcceptedAtMs"];
            }
        }
    }

    let has = |set: &BTreeSet<String>, wanted: &[&str]| {
        let missing: Vec<&&str> = wanted.iter().filter(|w| !set.contains(**w)).collect();
        assert!(missing.is_empty(), "missing {missing:?} in {set:?}");
    };
    has(
        &statuses,
        &[
            "running",
            "waiting",
            "needs-intervention",
            "completed",
            "failed",
        ],
    );
    has(
        &kinds,
        &[
            "run.started",
            "activity.result",
            "signal.received",
            "invocation.resolved",
        ],
    );
    has(
        &refused,
        &[
            "event.unknown-kind",
            "event.invalid",
            "event.out-of-order",
            "state.missing",
            "state.unexpected-start",
            "budget.microsteps",
            "budget.expression-operations",
            "budget.state-bytes",
            "budget.command-count",
        ],
    );
    has(
        &diagnostics,
        &[
            "event.ignored-terminal",
            "result.stale",
            "resolve.not-applicable",
            "signal.outcome-unrouted",
            "invocation.unknown",
            "activity.retry-scheduled",
        ],
    );
    has(&commands, &["activity.schedule", "activity.retry"]);
    has(
        &run_errors,
        &[
            "limit.visits-exceeded",
            "limit.activations-exceeded",
            "mapping.missing-path",
            "activity.undeclared-outcome",
            "activity.attempts-exhausted",
            "activity.failed",
            "gen.broken",
        ],
    );
    assert!(buffered && backed_off && raw);
    assert!(steps > 600 * 6, "{steps} steps in 600 traces");
}
