//! A seeded generator of traces.
//!
//! [`generate`] builds a random valid workflow, then drives a reducer through
//! it one event at a time, choosing each event from the snapshot the reducer
//! just returned. The result is a complete trace whose expectations are what
//! that reducer did, failures included, so any other reducer can be checked
//! against it with [`run`]. The same seed, reducer behavior,
//! and step bound always give the same trace; nothing but the seed feeds the
//! random choices.
//!
//! # Workflow
//!
//! - 2 to 5 activity nodes, 0 to 2 await-signal nodes, a `complete` node, a
//!   `fail` node, and sometimes a second `complete` node.
//! - 1 to as many actions as activity nodes, so nodes may share an action.
//!   Effects cover `pure`, `read-only`, `idempotent`, and `unsafe`. Outcomes
//!   are 1 to 3 names or left to the default; retry policies are absent,
//!   partial, or complete with 1 to 4 attempts. Host fields (`kind`, `argv`)
//!   are present for the kernel to ignore.
//! - Every outcome routes to a random node, a terminal one about one time in
//!   seven, so back edges and self loops are common. Half the activity nodes
//!   also route `failed`. The start node is usually the first activity.
//! - Inputs and outputs are mappings over `input`, `nodes`, and `run`, with
//!   `$get`, `$literal`, array indexes, and now and then a path that will not
//!   resolve.
//! - Workflow limits are usually the defaults and sometimes small enough for a
//!   loop to hit them. One trace in eight narrows one reducer budget; a run
//!   that then refuses three events in a row ends its trace.
//!
//! # Events
//!
//! A run starts with `run.started` (event ID `start`, tenant [`TENANT`], run
//! ID `run_<seed as 16 hex digits>`); one trace in ten sends some noise first.
//! After that the status of the last snapshot decides:
//!
//! | Status | Likely event | Otherwise |
//! | --- | --- | --- |
//! | `running` | a result for the outstanding attempt: success with a declared, defaulted, or undeclared outcome; failure, retryable or not; unknown; expired | noise |
//! | `waiting` | the awaited signal, with a routed, unrouted, or absent outcome | noise |
//! | `needs-intervention` | a resolution of the unknown invocation: complete (declared or undeclared outcome), retry, fail | noise |
//! | terminal | 1 to 3 noise events, then the trace ends | |
//!
//! Noise is a result for a wrong attempt or another invocation, a repeat of a
//! result already sent, a signal nobody may be waiting for, a resolution that
//! does not apply, or an event the reducer must refuse: a wrong sequence
//! number, an unknown kind, a malformed or non-canonical payload, a second
//! `run.started`.
//!
//! Sequence numbers follow the last accepted event. Accepted time moves forward
//! by up to 1.5 s per event and occasionally steps back, since logical time
//! must not follow it down. Payloads carry integers at the ±(2^53 − 1) bounds,
//! escapes, and keys whose UTF-16 order differs from their code point order.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use runspore_types::canonical;
use runspore_types::digest::ids;
use runspore_types::model::{event_kind, InvocationPhase, RunStatus, State, WORKFLOW_FORMAT};
use runspore_types::reducer::Reducer;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::compile::Header;
use crate::error::Error;
use crate::format::{Event, Expect, LimitsOverride, Payload, Step, Trace};
use crate::run::{record, run, Session};

/// The step bound to use unless a test wants something else.
pub const DEFAULT_MAX_STEPS: usize = 20;

/// Tenant of every generated run.
pub const TENANT: &str = "default";

/// Traces aim for at least this many steps when the bound allows.
const MIN_STEPS: usize = 12;

/// A run that refuses this many events in a row is stuck, usually on a
/// narrowed budget, and its trace ends there.
const MAX_REFUSALS: usize = 3;

/// Builds the trace for `seed`, at most `max_steps` steps long, recording what
/// `reducer` returns for each event.
///
/// Fails only if the reducer returns bytes that are not canonical JSON.
pub fn generate(seed: u64, reducer: &dyn Reducer, max_steps: usize) -> Result<Trace, Error> {
    let mut rng = Rng(seed);
    let name = format!("generated-{seed:016x}");
    let plan = Plan::new(&mut rng, &name);
    let mut trace = Trace::new(
        &name,
        &format!("generated from seed {seed}"),
        plan.document.clone(),
    );
    trace.limits = budget(&mut rng);
    let header = Header::of(&trace)?;
    let length = if max_steps > MIN_STEPS {
        MIN_STEPS + rng.index(max_steps - MIN_STEPS + 1)
    } else {
        max_steps
    };
    let input = run_input(&mut rng);
    let now_ms = 1_000 + rng.below(9_000);
    let mut driver = Driver {
        session: Session::new(reducer, &header.identity, header.limits, &header.workflow),
        header,
        rng,
        plan,
        name,
        run_id: format!("run_{seed:016x}"),
        input,
        state: None,
        next_sequence: 1,
        now_ms,
        delivered: Vec::new(),
        invocations: Vec::new(),
        steps: Vec::new(),
    };
    driver.drive(length)?;
    trace.steps = driver.steps;
    Ok(trace)
}

/// What a batch of generated traces contained, to show it was not vacuous.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub traces: u64,
    pub steps: u64,
    /// Steps the source reducer refused with a failure.
    pub refused: u64,
}

/// The lowest seed of a batch whose trace did not replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedError {
    pub seed: u64,
    pub error: Error,
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "seed {}: {}", self.seed, self.error)
    }
}

impl std::error::Error for SeedError {}

/// Generates the trace of every seed in `seeds` from `source` and replays it
/// against `target`, on all available cores. Passing the same reducer twice
/// checks that it replays its own runs.
pub fn cross_check(
    source: &dyn Reducer,
    target: &dyn Reducer,
    seeds: Range<u64>,
    max_steps: usize,
) -> Result<Totals, SeedError> {
    let next = AtomicU64::new(seeds.start);
    let work = || {
        let mut totals = Totals::default();
        loop {
            let seed = next.fetch_add(1, Ordering::Relaxed);
            if seed >= seeds.end {
                return Ok(totals);
            }
            let check = generate(seed, source, max_steps).and_then(|trace| {
                run(target, &trace)?;
                Ok(trace)
            });
            let trace = check.map_err(|error| SeedError { seed, error })?;
            totals.traces += 1;
            totals.steps += trace.steps.len() as u64;
            totals.refused += trace
                .steps
                .iter()
                .filter(|step| matches!(step.expect, Some(Expect::Failure(_))))
                .count() as u64;
        }
    };
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let results: Vec<Result<Totals, SeedError>> = std::thread::scope(|scope| {
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
    // Seeds are handed out in order and a worker stops at its first error, so
    // the lowest failing seed is always among the reported ones.
    let mut totals = Totals::default();
    let mut first: Option<SeedError> = None;
    for result in results {
        match result {
            Ok(part) => {
                totals.traces += part.traces;
                totals.steps += part.steps;
                totals.refused += part.refused;
            }
            Err(error) if first.as_ref().is_none_or(|f| error.seed < f.seed) => first = Some(error),
            Err(_) => {}
        }
    }
    first.map_or(Ok(totals), Err)
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform enough in `0..bound`; `bound` must not be zero.
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// `items` must not be empty.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }
}

const ACTIVITY_STEMS: [&str; 5] = ["fetch", "Build", "test", "Review", "deploy"];
const SEPARATORS: [&str; 3] = ["", "_", "-"];
const SIGNALS: [&str; 3] = ["approval", "go", "data.ready"];
const SIGNAL_OUTCOMES: [&str; 3] = ["approved", "rejected", "received"];
const EXTRA_OUTCOMES: [&str; 4] = ["red", "again", "skip", "2nd"];
const KEYS: [&str; 9] = [
    "a",
    "b",
    "Z",
    "10",
    "9",
    "",
    "\u{e9}",
    "\u{1f600}",
    "\u{fb33}",
];
const TEXTS: [&str; 8] = [
    "",
    "ok",
    "line\nbreak",
    "quote\" and back\\slash",
    "\u{e9}t\u{e9}",
    "\u{1f600}",
    "\u{fb33}\u{1}",
    "</script>",
];
const INTEGERS: [i64; 7] = [
    0,
    1,
    -1,
    42,
    canonical::MAX_SAFE_INT,
    -canonical::MAX_SAFE_INT,
    1_700_000_000_000,
];

/// The generated workflow and what the driver needs to know about it.
struct Plan {
    document: Value,
    /// Declared outcomes per action ID, with the default applied.
    outcomes: BTreeMap<String, Vec<String>>,
    /// Signal name and routed outcomes per await-signal node ID.
    waits: BTreeMap<String, (String, Vec<String>)>,
    /// Signal names some node waits for.
    signals: Vec<String>,
}

impl Plan {
    fn new(rng: &mut Rng, name: &str) -> Plan {
        let activity_count = 2 + rng.index(4);
        let activities: Vec<String> = (0..activity_count)
            .map(|i| format!("{}{}{i}", ACTIVITY_STEMS[i], rng.pick(&SEPARATORS)))
            .collect();
        let wait_nodes: Vec<String> = (0..rng.index(3)).map(|i| format!("wait_{i}")).collect();
        let mut terminals = vec!["done".to_string(), "broken".to_string()];
        if rng.chance(30) {
            terminals.push("done-alt".to_string());
        }
        let inner: Vec<String> = activities.iter().chain(&wait_nodes).cloned().collect();
        let start = if rng.chance(75) {
            inner[0].clone()
        } else {
            rng.pick(&inner).clone()
        };
        let target = |rng: &mut Rng| {
            let pool = if rng.chance(15) { &terminals } else { &inner };
            json!(rng.pick(pool))
        };

        let action_count = 1 + rng.index(activity_count);
        let mut actions = Map::new();
        let mut outcomes = BTreeMap::new();
        for k in 0..action_count {
            let id = format!("act_{k}");
            let (action, declared) = action(rng, &id);
            actions.insert(id.clone(), action);
            outcomes.insert(id, declared);
        }

        let mut nodes = Map::new();
        for (i, id) in activities.iter().enumerate() {
            let action = format!(
                "act_{}",
                if i < action_count {
                    i
                } else {
                    rng.index(action_count)
                }
            );
            let mut routes = Map::new();
            for outcome in &outcomes[&action] {
                routes.insert(outcome.clone(), target(rng));
            }
            if rng.chance(50) {
                routes.insert("failed".to_string(), target(rng));
            }
            let mut node = Map::new();
            node.insert("kind".to_string(), json!("activity"));
            node.insert("action".to_string(), json!(action));
            if rng.chance(85) {
                let input = mapping(rng, &inner, &start, *id == start);
                node.insert("input".to_string(), input);
            }
            node.insert("outcomes".to_string(), Value::Object(routes));
            nodes.insert(id.clone(), Value::Object(node));
        }

        let mut waits = BTreeMap::new();
        let mut signals = Vec::new();
        for id in &wait_nodes {
            let signal = rng.pick(&SIGNALS).to_string();
            let mut routed: Vec<String> = SIGNAL_OUTCOMES
                .iter()
                .filter(|_| rng.chance(55))
                .map(|outcome| outcome.to_string())
                .collect();
            if routed.is_empty() {
                routed.push(rng.pick(&SIGNAL_OUTCOMES).to_string());
            }
            let routes: Map<String, Value> = routed
                .iter()
                .map(|outcome| (outcome.clone(), target(rng)))
                .collect();
            nodes.insert(
                id.clone(),
                json!({"kind": "await-signal", "signal": signal, "outcomes": routes}),
            );
            if !signals.contains(&signal) {
                signals.push(signal.clone());
            }
            waits.insert(id.clone(), (signal, routed));
        }

        let mut done = Map::new();
        done.insert("kind".to_string(), json!("complete"));
        if rng.chance(80) {
            let output = mapping(rng, &inner, &start, false);
            done.insert("output".to_string(), output);
        }
        nodes.insert("done".to_string(), Value::Object(done));
        let mut error = Map::new();
        error.insert("code".to_string(), json!("gen.broken"));
        error.insert("message".to_string(), json!("generated failure"));
        if rng.chance(30) {
            error.insert("details".to_string(), random_value(rng, 1));
        }
        nodes.insert(
            "broken".to_string(),
            json!({"kind": "fail", "error": error}),
        );
        if terminals.len() > 2 {
            nodes.insert("done-alt".to_string(), json!({"kind": "complete"}));
        }

        let mut document = Map::new();
        document.insert("format".to_string(), json!(WORKFLOW_FORMAT));
        document.insert("name".to_string(), json!(name));
        document.insert("start".to_string(), json!(start));
        if rng.chance(40) {
            let mut limits = Map::new();
            if rng.chance(70) {
                limits.insert("maxVisitsPerNode".to_string(), json!(1 + rng.below(4)));
            }
            if rng.chance(50) {
                limits.insert("maxActivations".to_string(), json!(3 + rng.below(10)));
            }
            document.insert("limits".to_string(), Value::Object(limits));
        }
        document.insert("actions".to_string(), Value::Object(actions));
        document.insert("nodes".to_string(), Value::Object(nodes));

        Plan {
            document: Value::Object(document),
            outcomes,
            waits,
            signals,
        }
    }
}

/// An action binding and the outcomes it declares.
fn action(rng: &mut Rng, id: &str) -> (Value, Vec<String>) {
    let mut action = Map::new();
    action.insert("kind".to_string(), json!("command"));
    action.insert("argv".to_string(), json!(["tool", id]));
    let effect = match rng.below(100) {
        0..=34 => "idempotent",
        35..=64 => "unsafe",
        65..=84 => "read-only",
        _ => "pure",
    };
    action.insert("effect".to_string(), json!(effect));

    let mut declared = vec!["ok".to_string()];
    if rng.chance(75) {
        declared.clear();
        if rng.chance(80) {
            declared.push("ok".to_string());
        }
        for extra in EXTRA_OUTCOMES {
            if declared.len() < 3 && rng.chance(30) {
                declared.push(extra.to_string());
            }
        }
        if declared.is_empty() {
            declared.push(rng.pick(&EXTRA_OUTCOMES).to_string());
        }
        if rng.chance(50) {
            declared.reverse();
        }
        action.insert("outcomes".to_string(), json!(declared));
    }

    if rng.chance(60) {
        let mut retry = Map::new();
        if rng.chance(85) {
            retry.insert("maxAttempts".to_string(), json!(1 + rng.below(4)));
        }
        if rng.chance(70) {
            let backoff = rng.pick(&["0", "100", "500", "30000"]);
            retry.insert("backoffMs".to_string(), json!(backoff));
        }
        if rng.chance(30) {
            retry.insert("maxBackoffMs".to_string(), json!("800"));
        }
        action.insert("retry".to_string(), Value::Object(retry));
    }
    (Value::Object(action), declared)
}

/// A well-formed mapping. Most resolve against any run; some depend on which
/// nodes have run, and a few never resolve. The start node only gets mappings
/// that resolve, so a run is not lost on its first event.
fn mapping(rng: &mut Rng, nodes: &[String], start: &str, at_start: bool) -> Value {
    match rng.below(100) {
        0..=19 => json!({"$get": ["input"]}),
        20..=41 => json!({
            "repo": {"$get": ["input", "repo"]},
            "quoted": {"$literal": {"$get": ["not", "evaluated"]}},
        }),
        42..=59 => json!({
            "first": {"$get": ["input", "list", 0]},
            "run": {"$get": ["run", "id"]},
            "tenant": {"$get": ["run", "tenant"]},
        }),
        60..=71 => json!([{"$get": ["input", "x"]}, "fixed", 7, null]),
        72..=88 if !at_start => {
            let node = if rng.chance(75) {
                start
            } else {
                rng.pick(nodes)
            };
            json!({
                "previous": {"$get": ["nodes", node, "output"]},
                "outcome": {"$get": ["nodes", node, "outcome"]},
            })
        }
        89..=91 if !at_start => json!({"$get": ["input", "missing"]}),
        _ => random_value(rng, 2),
    }
}

/// One narrowed reducer budget, for one trace in eight.
fn budget(rng: &mut Rng) -> Option<LimitsOverride> {
    if !rng.chance(12) {
        return None;
    }
    let mut limits = LimitsOverride::default();
    match rng.below(12) {
        0..=3 => limits.microsteps = Some(1 + rng.below(3) as u32),
        4..=6 => limits.expression_operations = Some(3 + rng.below(12) as u32),
        7..=10 => limits.max_state_bytes = Some(600 + rng.below(1_200) as u32),
        _ => limits.max_command_count = Some(0),
    }
    Some(limits)
}

fn run_input(rng: &mut Rng) -> Value {
    let mut input = Map::new();
    input.insert(
        "repo".to_string(),
        json!(format!("repo-{}", rng.below(100))),
    );
    input.insert("x".to_string(), json!(rng.below(1_000) as i64 - 500));
    let list: Vec<Value> = (0..1 + rng.index(3)).map(|_| scalar(rng)).collect();
    input.insert("list".to_string(), Value::Array(list));
    if rng.chance(40) {
        input.insert(rng.pick(&KEYS).to_string(), random_value(rng, 2));
    }
    Value::Object(input)
}

fn scalar(rng: &mut Rng) -> Value {
    match rng.below(6) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(false),
        3 => json!(rng.pick(&INTEGERS)),
        4 => json!(rng.below(2_000) as i64 - 1_000),
        _ => json!(rng.pick(&TEXTS)),
    }
}

/// A value in the canonical domain, nested at most `depth` containers deep.
fn random_value(rng: &mut Rng, depth: u32) -> Value {
    if depth == 0 || rng.chance(45) {
        return scalar(rng);
    }
    let len = rng.index(4);
    if rng.chance(50) {
        Value::Array((0..len).map(|_| random_value(rng, depth - 1)).collect())
    } else {
        Value::Object(
            (0..len)
                .map(|_| (rng.pick(&KEYS).to_string(), random_value(rng, depth - 1)))
                .collect(),
        )
    }
}

fn error_info(rng: &mut Rng) -> Value {
    let mut error = Map::new();
    let code = rng.pick(&["gen.transient", "gen.rejected", "io.timeout"]);
    error.insert("code".to_string(), json!(code));
    error.insert("message".to_string(), json!("generated error"));
    match rng.below(10) {
        0 => error.insert("nodeId".to_string(), json!("elsewhere")),
        1 | 2 => error.insert("nodeId".to_string(), Value::Null),
        _ => None,
    };
    if rng.chance(30) {
        error.insert("details".to_string(), random_value(rng, 1));
    }
    Value::Object(error)
}

/// An `activity.result` payload for one attempt. Optional members are left out
/// about as often as they are written.
fn result_payload(rng: &mut Rng, invocation_id: &str, attempt: u32, declared: &[String]) -> Value {
    let attempt_id = ids::attempt(invocation_id, attempt);
    let roll = rng.below(100);
    if roll >= 87 {
        // The body every store appends on lease expiry (`ActivityResult::expired`).
        return json!({
            "invocationId": invocation_id, "attemptId": attempt_id, "attempt": attempt,
            "status": "expired", "outcome": null, "output": null, "error": null,
            "retryable": true,
        });
    }
    let mut result = Map::new();
    result.insert("invocationId".to_string(), json!(invocation_id));
    result.insert("attemptId".to_string(), json!(attempt_id));
    result.insert("attempt".to_string(), json!(attempt));
    match roll {
        0..=54 => {
            result.insert("status".to_string(), json!("success"));
            let outcome = if declared.is_empty() || rng.chance(8) {
                Some(rng.pick(&["bogus", "failed"]).to_string())
            } else {
                let picked = rng.pick(declared).clone();
                (picked != "ok" || rng.chance(50)).then_some(picked)
            };
            if let Some(outcome) = outcome {
                result.insert("outcome".to_string(), json!(outcome));
            }
            if rng.chance(70) {
                result.insert("output".to_string(), random_value(rng, 2));
            }
        }
        55..=74 => {
            result.insert("status".to_string(), json!("failure"));
            match rng.below(100) {
                0..=39 => {}
                40..=74 => {
                    result.insert("retryable".to_string(), json!(true));
                }
                _ => {
                    result.insert("retryable".to_string(), json!(false));
                }
            }
            if rng.chance(60) {
                result.insert("error".to_string(), error_info(rng));
            }
        }
        _ => {
            result.insert("status".to_string(), json!("unknown"));
            if rng.chance(30) {
                result.insert("error".to_string(), error_info(rng));
            }
        }
    }
    Value::Object(result)
}

fn resolution(rng: &mut Rng, declared: &[String]) -> Value {
    match rng.below(100) {
        0..=44 => {
            let outcome = if declared.is_empty() || rng.chance(20) {
                "bogus".to_string()
            } else {
                rng.pick(declared).clone()
            };
            let mut complete = Map::new();
            complete.insert("action".to_string(), json!("complete"));
            complete.insert("outcome".to_string(), json!(outcome));
            if rng.chance(60) {
                complete.insert("output".to_string(), random_value(rng, 2));
            }
            Value::Object(complete)
        }
        45..=74 => json!({"action": "retry"}),
        _ => json!({"action": "fail", "error": error_info(rng)}),
    }
}

struct Driver<'a> {
    session: Session<'a>,
    header: Header,
    rng: Rng,
    plan: Plan,
    name: String,
    run_id: String,
    input: Value,
    /// The last snapshot, if the reducer has produced one this driver can read.
    state: Option<State>,
    next_sequence: u64,
    now_ms: u64,
    /// Results sent for an outstanding attempt, as (event ID, payload).
    delivered: Vec<(String, Value)>,
    /// Invocation IDs seen in snapshots, oldest first.
    invocations: Vec<String>,
    steps: Vec<Step>,
}

impl Driver<'_> {
    fn drive(&mut self, length: usize) -> Result<(), Error> {
        let mut after_terminal: Option<usize> = None;
        let mut refused = 0;
        while self.steps.len() < length && refused < MAX_REFUSALS {
            match after_terminal.as_mut() {
                Some(0) => break,
                Some(left) => *left -= 1,
                None => {}
            }
            let event = self.next_event();
            refused = if self.apply(event)? { 0 } else { refused + 1 };
            let terminal = self
                .state
                .as_ref()
                .is_some_and(|state| state.status.is_terminal());
            if terminal && after_terminal.is_none() {
                after_terminal = Some(1 + self.rng.index(3));
            }
        }
        Ok(())
    }

    /// Sends one event and records the step. Returns whether the reducer
    /// produced a decision.
    fn apply(&mut self, event: Event) -> Result<bool, Error> {
        let index = self.steps.len();
        let mut step = Step::new(event);
        let request = self.header.step(index, &step)?;
        let outcome = self.session.apply(&request.event, None, None);
        let expect = record(&self.name, index, &step.event.event_id, &outcome)?;
        if let Expect::Decision(decision) = &expect {
            self.next_sequence = step.event.sequence.saturating_add(1);
            self.state = State::deserialize(&decision.snapshot).ok();
            let invocation = self
                .state
                .as_ref()
                .and_then(|state| state.invocation.as_ref());
            if let Some(invocation) = invocation {
                if !self.invocations.contains(&invocation.invocation_id) {
                    self.invocations.push(invocation.invocation_id.clone());
                }
            }
        }
        step.expect = Some(expect);
        self.steps.push(step);
        Ok(outcome.is_ok())
    }

    fn next_event(&mut self) -> Event {
        let Some(state) = self.state.clone() else {
            return if self.rng.chance(90) {
                self.start()
            } else {
                self.noise(None)
            };
        };
        let likely = match state.status {
            RunStatus::Running if self.rng.chance(72) => self.result(&state),
            RunStatus::Waiting if self.rng.chance(60) => self.awaited_signal(&state),
            RunStatus::NeedsIntervention if self.rng.chance(65) => self.resolve(&state),
            _ => None,
        };
        likely.unwrap_or_else(|| self.noise(Some(&state)))
    }

    /// An event at the next sequence number and the next accepted time.
    fn event(&mut self, event_id: String, kind: &str, payload: Value) -> Event {
        let accepted_at_ms = if self.rng.chance(6) {
            self.now_ms.saturating_sub(self.rng.below(500))
        } else {
            self.now_ms += self.rng.below(1_500);
            self.now_ms
        };
        Event {
            event_id,
            sequence: self.next_sequence,
            accepted_at_ms,
            kind: kind.to_string(),
            payload: Payload::Json(payload),
        }
    }

    /// An event ID no other step of the trace has.
    fn unique(&self, base: &str) -> String {
        format!("{base}~{}", self.steps.len())
    }

    fn start(&mut self) -> Event {
        let used = self
            .steps
            .iter()
            .any(|step| step.event.event_id == ids::EVENT_STARTED);
        let event_id = if used {
            self.unique(ids::EVENT_STARTED)
        } else {
            ids::EVENT_STARTED.to_string()
        };
        let payload = json!({"tenant": TENANT, "runId": self.run_id, "input": self.input});
        self.event(event_id, event_kind::RUN_STARTED, payload)
    }

    fn result(&mut self, state: &State) -> Option<Event> {
        let invocation = state.invocation.as_ref()?;
        let declared = self.plan.outcomes.get(&invocation.action_id)?;
        let payload = result_payload(
            &mut self.rng,
            &invocation.invocation_id,
            invocation.attempt,
            declared,
        );
        let event_id =
            ids::event_for_attempt(&ids::attempt(&invocation.invocation_id, invocation.attempt));
        self.delivered.push((event_id.clone(), payload.clone()));
        Some(self.event(event_id, event_kind::ACTIVITY_RESULT, payload))
    }

    fn awaited_signal(&mut self, state: &State) -> Option<Event> {
        let (name, routed) = self.plan.waits.get(&state.position.as_ref()?.node_id)?;
        let outcome = match self.rng.below(100) {
            0..=69 => {
                let picked = self.rng.pick(routed).clone();
                (picked != "received" || self.rng.chance(50)).then_some(picked)
            }
            70..=84 => Some("nope".to_string()),
            _ => None,
        };
        let name = name.clone();
        Some(self.signal(&name, outcome))
    }

    fn signal(&mut self, name: &str, outcome: Option<String>) -> Event {
        let mut payload = Map::new();
        payload.insert("name".to_string(), json!(name));
        match outcome {
            Some(outcome) => {
                payload.insert("outcome".to_string(), json!(outcome));
            }
            None if self.rng.chance(30) => {
                payload.insert("outcome".to_string(), Value::Null);
            }
            None => {}
        }
        if self.rng.chance(60) {
            payload.insert("data".to_string(), random_value(&mut self.rng, 2));
        }
        let event_id = ids::event_for_signal(&format!("m{}", self.steps.len()));
        self.event(
            event_id,
            event_kind::SIGNAL_RECEIVED,
            Value::Object(payload),
        )
    }

    fn resolve(&mut self, state: &State) -> Option<Event> {
        let invocation = state.invocation.as_ref()?;
        let declared = self.plan.outcomes.get(&invocation.action_id)?;
        let resolution = resolution(&mut self.rng, declared);
        Some(self.resolved(&invocation.invocation_id, resolution))
    }

    fn resolved(&mut self, invocation_id: &str, resolution: Value) -> Event {
        let event_id = ids::event_for_resolution(&format!("r{}", self.steps.len()));
        let payload = json!({"invocationId": invocation_id, "resolution": resolution});
        self.event(event_id, event_kind::INVOCATION_RESOLVED, payload)
    }

    fn noise(&mut self, state: Option<&State>) -> Event {
        match self.rng.below(100) {
            0..=21 => self.stale_result(state),
            22..=33 => self.repeated_result(state),
            34..=61 => self.stray_signal(),
            62..=75 => self.misplaced_resolution(state),
            76..=83 => self.out_of_order(),
            84..=88 => self.unknown_kind(),
            89..=95 => self.invalid_payload(),
            _ => self.restart(),
        }
    }

    /// A result the outstanding attempt is not waiting for.
    fn stale_result(&mut self, state: Option<&State>) -> Event {
        let current = state.and_then(|state| state.invocation.as_ref());
        let (invocation_id, attempt) = match current {
            Some(invocation) if self.rng.chance(55) => {
                let attempt = match invocation.state {
                    InvocationPhase::Unknown if self.rng.chance(50) => invocation.attempt,
                    _ if invocation.attempt > 1 && self.rng.chance(50) => invocation.attempt - 1,
                    _ => invocation.attempt + 1,
                };
                (invocation.invocation_id.clone(), attempt)
            }
            _ => (self.foreign_invocation(state), 1 + self.rng.below(3) as u32),
        };
        let payload = result_payload(&mut self.rng, &invocation_id, attempt, &["ok".to_string()]);
        let event_id = self.unique(&ids::event_for_attempt(&ids::attempt(
            &invocation_id,
            attempt,
        )));
        self.event(event_id, event_kind::ACTIVITY_RESULT, payload)
    }

    fn repeated_result(&mut self, state: Option<&State>) -> Event {
        if self.delivered.is_empty() {
            return self.stale_result(state);
        }
        let (event_id, payload) = self.rng.pick(&self.delivered).clone();
        let event_id = self.unique(&event_id);
        self.event(event_id, event_kind::ACTIVITY_RESULT, payload)
    }

    fn stray_signal(&mut self) -> Event {
        let name = if !self.plan.signals.is_empty() && self.rng.chance(70) {
            self.rng.pick(&self.plan.signals).clone()
        } else {
            "noise.sig".to_string()
        };
        let outcome = match self.rng.below(5) {
            0 => None,
            1 => Some("nope"),
            n => Some(SIGNAL_OUTCOMES[n as usize - 2]),
        };
        self.signal(&name, outcome.map(str::to_string))
    }

    /// A resolution for an invocation that is not in the unknown phase.
    fn misplaced_resolution(&mut self, state: Option<&State>) -> Event {
        let scheduled = state
            .and_then(|state| state.invocation.as_ref())
            .filter(|invocation| invocation.state == InvocationPhase::Scheduled);
        let invocation_id = match scheduled {
            Some(invocation) if self.rng.chance(60) => invocation.invocation_id.clone(),
            _ => self.foreign_invocation(state),
        };
        let resolution = resolution(&mut self.rng, &["ok".to_string()]);
        self.resolved(&invocation_id, resolution)
    }

    /// An earlier invocation of this run, or an ID no run has.
    fn foreign_invocation(&mut self, state: Option<&State>) -> String {
        let current = state
            .and_then(|state| state.invocation.as_ref())
            .map(|invocation| &invocation.invocation_id);
        let earlier: Vec<&String> = self
            .invocations
            .iter()
            .filter(|id| Some(*id) != current)
            .collect();
        if !earlier.is_empty() && self.rng.chance(60) {
            (*self.rng.pick(&earlier)).clone()
        } else {
            format!("inv_{:032x}", self.rng.next())
        }
    }

    fn out_of_order(&mut self) -> Event {
        let mut event = self.stray_signal();
        event.sequence = if event.sequence > 1 && self.rng.chance(50) {
            event.sequence - 1
        } else {
            event.sequence + 1 + self.rng.below(3)
        };
        event
    }

    fn unknown_kind(&mut self) -> Event {
        let event_id = self.unique("evt");
        let kind = *self
            .rng
            .pick(&["timer.fired", "run.cancelled", "Activity.Result"]);
        let payload = random_value(&mut self.rng, 1);
        self.event(event_id, kind, payload)
    }

    /// A known kind whose payload has the wrong shape or is not canonical.
    fn invalid_payload(&mut self) -> Event {
        let signal = event_kind::SIGNAL_RECEIVED;
        let (kind, payload) = match self.rng.below(9) {
            0 => (signal, Payload::Raw(r#"{"name" : "approval"}"#.to_string())),
            1 => (
                signal,
                Payload::Raw(r#"{"name":"approval","data":null}"#.to_string()),
            ),
            2 => (
                signal,
                Payload::Raw(r#"{"name":"approval","name":"go"}"#.to_string()),
            ),
            3 => (
                signal,
                Payload::Raw(r#"{"data":1.5,"name":"approval"}"#.to_string()),
            ),
            4 => (signal, Payload::Raw(String::new())),
            5 => (signal, Payload::Json(json!({"name": 7}))),
            6 => (
                signal,
                Payload::Json(json!({"name": "approval", "extra": true})),
            ),
            7 => (
                event_kind::ACTIVITY_RESULT,
                Payload::Json(json!({"invocationId": "inv_incomplete"})),
            ),
            _ => (
                event_kind::INVOCATION_RESOLVED,
                Payload::Json(json!({"invocationId": "inv_x", "resolution": {"action": "shrug"}})),
            ),
        };
        let event_id = self.unique("evt");
        let mut event = self.event(event_id, kind, Value::Null);
        event.payload = payload;
        event
    }

    /// A second `run.started`.
    fn restart(&mut self) -> Event {
        let mut event = self.start();
        if self.rng.chance(50) {
            event.sequence = 1;
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use runspore_types::model::{ActionPolicy, Effect, Node, Workflow};

    use runspore_types::reducer::{Decision, Failure, FailureKind, TransitionRequest};

    use super::*;
    use crate::testing::{Altered, Toy};

    #[test]
    fn splitmix64_matches_the_reference_sequence() {
        let mut rng = Rng(1_234_567);
        let outputs: Vec<u64> = (0..5).map(|_| rng.next()).collect();
        assert_eq!(
            outputs,
            [
                6_457_827_717_110_365_317,
                3_203_168_211_198_807_973,
                9_817_491_932_198_370_423,
                4_593_380_528_125_082_431,
                16_408_922_859_458_223_821
            ]
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_trace() {
        let mut texts = BTreeSet::new();
        for seed in 0..40 {
            let text = generate(seed, &Toy, DEFAULT_MAX_STEPS).unwrap().to_json();
            assert_eq!(
                generate(seed, &Toy, DEFAULT_MAX_STEPS).unwrap().to_json(),
                text
            );
            texts.insert(text);
        }
        assert_eq!(texts.len(), 40);
        let trace = generate(u64::MAX, &Toy, DEFAULT_MAX_STEPS).unwrap();
        assert_eq!(trace.name, "generated-ffffffffffffffff");
        assert_eq!(trace.workflow["name"], "generated-ffffffffffffffff");
        let start = trace
            .steps
            .iter()
            .find(|step| step.event.event_id == "start")
            .unwrap();
        assert_eq!(
            (start.event.sequence, start.event.kind.as_str()),
            (1, "run.started")
        );
        let Payload::Json(payload) = &start.event.payload else {
            panic!("the start payload is plain JSON");
        };
        assert_eq!(payload["tenant"], TENANT);
        assert_eq!(payload["runId"], "run_ffffffffffffffff");
        assert!(payload["input"]["repo"].is_string());
    }

    fn matches(text: &str, max: usize, first: fn(char) -> bool, rest: fn(char) -> bool) -> bool {
        let mut chars = text.chars();
        chars.next().is_some_and(first) && chars.all(rest) && text.len() <= max
    }

    fn identifier(text: &str, max: usize, punctuation: &str) -> bool {
        !text.is_empty()
            && text.len() <= max
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || punctuation.contains(c))
    }

    fn outcome_name(text: &str) -> bool {
        let head = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
        matches(text, 64, head, |c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
        })
    }

    /// Rules W01 to W09 and W11 of `spec/kernel.md`.
    fn assert_well_formed(document: &Value) {
        let workflow: Workflow = serde_json::from_value(document.clone()).unwrap();
        assert_eq!(workflow.format, WORKFLOW_FORMAT);
        assert!(identifier(&workflow.name, 128, "_.-"));
        assert!((1..=256).contains(&workflow.nodes.len()));
        assert!(workflow.nodes.contains_key(&workflow.start));
        assert!((1..=1_000).contains(&workflow.limits.max_visits_per_node));
        assert!((1..=10_000).contains(&workflow.limits.max_activations));

        let policy = |id: &String| -> ActionPolicy {
            serde_json::from_value(workflow.actions[id].clone()).unwrap()
        };
        for id in workflow.actions.keys() {
            assert!(identifier(id, 64, "_-"));
            let policy = policy(id);
            assert!(!policy.outcomes.is_empty());
            let distinct: BTreeSet<&String> = policy.outcomes.iter().collect();
            assert_eq!(distinct.len(), policy.outcomes.len());
            assert!(policy
                .outcomes
                .iter()
                .all(|outcome| outcome_name(outcome) && outcome != "failed"));
            assert!((1..=100).contains(&policy.retry.max_attempts));
            assert_ne!(policy.effect, Effect::Reconcilable);
        }
        for (id, node) in &workflow.nodes {
            assert!(identifier(id, 64, "_-"));
            match node {
                Node::Activity {
                    action, outcomes, ..
                } => {
                    let declared = policy(action).outcomes;
                    assert!(declared.iter().all(|o| outcomes.contains_key(o)));
                    for (outcome, target) in outcomes {
                        assert!(outcome == "failed" || declared.contains(outcome));
                        assert!(workflow.nodes.contains_key(target));
                    }
                }
                Node::AwaitSignal { signal, outcomes } => {
                    assert!(identifier(signal, 64, "_.-"));
                    assert!(!outcomes.is_empty());
                    for (outcome, target) in outcomes {
                        assert!(outcome_name(outcome));
                        assert!(workflow.nodes.contains_key(target));
                    }
                }
                Node::Complete { .. } => {}
                Node::Fail { error } => {
                    let head = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
                    assert!(matches(&error.code, 64, head, |c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'
                    }));
                    assert_eq!(error.node_id, None);
                }
            }
        }
    }

    #[test]
    fn generated_workflows_are_well_formed_and_varied() {
        let mut effects = BTreeSet::new();
        let mut shapes = BTreeSet::new();
        for seed in 0..500 {
            let mut rng = Rng(seed);
            let plan = Plan::new(&mut rng, "generated");
            assert_well_formed(&plan.document);
            let workflow: Workflow = serde_json::from_value(plan.document).unwrap();
            for action in workflow.actions.values() {
                effects.insert(action["effect"].as_str().unwrap().to_string());
                shapes.insert(match action.get("retry") {
                    Some(_) => "retry",
                    None => "no retry",
                });
                if action.get("outcomes").is_none() {
                    shapes.insert("default outcomes");
                }
            }
            for (id, node) in &workflow.nodes {
                match node {
                    Node::Activity { outcomes, .. } => {
                        if outcomes.values().any(|target| target == id) {
                            shapes.insert("self loop");
                        }
                        if outcomes.contains_key("failed") {
                            shapes.insert("failed route");
                        }
                    }
                    Node::AwaitSignal { .. } => {
                        shapes.insert("await-signal");
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            effects.into_iter().collect::<Vec<_>>(),
            ["idempotent", "pure", "read-only", "unsafe"]
        );
        assert_eq!(shapes.len(), 6, "{shapes:?}");
    }

    #[test]
    fn generated_traces_replay_within_the_step_bound() {
        for seed in 0..300 {
            let trace = generate(seed, &Toy, 16).unwrap();
            assert!((1..=16).contains(&trace.steps.len()), "seed {seed}");
            assert!(trace.steps.iter().all(|step| step.expect.is_some()));
            run(&Toy, &trace).unwrap_or_else(|error| panic!("seed {seed}: {error}"));
        }
        assert_eq!(generate(7, &Toy, 0).unwrap().steps.len(), 0);
        assert_eq!(generate(7, &Toy, 2).unwrap().steps.len(), 2);
    }

    #[test]
    fn cross_check_covers_every_seed_and_reports_the_lowest_failing_one() {
        let totals = cross_check(&Toy, &Toy, 0..64, 12).unwrap();
        let traces: Vec<Trace> = (0..64)
            .map(|seed| generate(seed, &Toy, 12).unwrap())
            .collect();
        assert_eq!(totals.traces, 64);
        assert_eq!(
            totals.steps,
            traces.iter().map(|t| t.steps.len() as u64).sum::<u64>()
        );
        assert!(0 < totals.refused && totals.refused < totals.steps);
        assert_eq!(cross_check(&Toy, &Toy, 9..9, 12), Ok(Totals::default()));

        // A target that traps on every run from seed 17 up; the seed is in the
        // workflow name.
        let trapping = Altered(
            |request: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                let graph = String::from_utf8_lossy(&request.graph);
                let at = graph.find("generated-").unwrap() + "generated-".len();
                if u64::from_str_radix(&graph[at..at + 16], 16).unwrap() >= 17 {
                    *outcome = Err(Failure {
                        kind: FailureKind::InvariantViolation,
                        code: "host.trap".to_string(),
                        details: String::new(),
                    });
                }
            },
        );
        let error = cross_check(&Toy, &trapping, 0..64, 12).unwrap_err();
        assert_eq!(error.seed, 17);
        assert!(matches!(&error.error, Error::Mismatch(mismatch) if mismatch.step == 0));
        assert!(error
            .to_string()
            .starts_with("seed 17: trace `generated-0000000000000011`, step 0 "));
    }

    #[test]
    fn the_event_mix_reaches_every_status_kind_and_refusal() {
        let mut statuses = BTreeSet::new();
        let mut kinds = BTreeSet::new();
        let mut refusals = BTreeSet::new();
        let mut diagnostics = BTreeSet::new();
        let mut commands = BTreeSet::new();
        let (mut raw, mut clock_steps_back, mut budgets) = (false, false, 0);
        for seed in 0..400 {
            let trace = generate(seed, &Toy, DEFAULT_MAX_STEPS).unwrap();
            budgets += usize::from(trace.limits.is_some());
            let mut last_sequence = 0;
            let mut last_time = 0;
            for step in &trace.steps {
                kinds.insert(step.event.kind.clone());
                raw |= matches!(step.event.payload, Payload::Raw(_));
                clock_steps_back |= step.event.accepted_at_ms < last_time;
                last_time = step.event.accepted_at_ms;
                match step.expect.as_ref().unwrap() {
                    Expect::Failure(failure) => {
                        refusals.insert(failure.code.clone());
                    }
                    Expect::Decision(decision) => {
                        assert_eq!(step.event.sequence, last_sequence + 1, "seed {seed}");
                        last_sequence = step.event.sequence;
                        statuses.insert(decision.snapshot["status"].as_str().unwrap().to_string());
                        diagnostics.extend(decision.diagnostics.iter().map(|d| d.code.clone()));
                        commands.extend(decision.commands.iter().map(|c| c.kind.clone()));
                    }
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
                "timer.fired",
            ],
        );
        has(
            &refusals,
            &[
                "event.out-of-order",
                "event.unknown-kind",
                "event.invalid",
                "state.unexpected-start",
                "state.missing",
                "budget.state-bytes",
            ],
        );
        has(
            &diagnostics,
            &[
                "result.stale",
                "resolve.not-applicable",
                "event.ignored-terminal",
                "invocation.unknown",
                "activity.retry-scheduled",
                "signal.outcome-unrouted",
            ],
        );
        has(&commands, &["activity.schedule", "activity.retry"]);
        assert!(raw && clock_steps_back);
        assert!((20..=90).contains(&budgets), "{budgets}");
    }
}
