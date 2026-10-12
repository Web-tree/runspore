//! The golden traces of `conformance/traces` against the native kernel
//! (`spec/kernel.md` section 9).
//!
//! `RUNSPORE_BLESS=1 cargo test -p runspore-kernel --test golden` records what the
//! kernel returns instead of checking it. Read the diff before committing it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use runspore_kernel::NativeReducer;
use runspore_trace::run::bless_requested;
use runspore_trace::{load, run, run_dir, Expect};
use runspore_types::model::{code, WORKFLOW_FORMAT, WORKFLOW_FORMAT_0_2};
use serde_json::{json, Value};

/// Files a checkout must hold; more may be added.
const MINIMUM_TRACES: usize = 59;

/// Every failure code, diagnostic code and kernel-produced error code. Each must
/// be expected by at least one trace, so no part of the vocabulary goes unpinned.
const VOCABULARY: &[&str] = &[
    code::WORKFLOW_INVALID,
    code::EVENT_INVALID,
    code::EVENT_OUT_OF_ORDER,
    code::EVENT_UNKNOWN_KIND,
    code::STATE_INVALID,
    code::STATE_MISSING,
    code::STATE_UNEXPECTED_START,
    code::VERSION_SEMANTICS,
    code::VERSION_CODEC,
    code::VERSION_STATE_FORMAT,
    code::WORKFLOW_UNSUPPORTED_EFFECT,
    code::BUDGET_MICROSTEPS,
    code::BUDGET_EXPRESSION_OPERATIONS,
    code::BUDGET_STATE_BYTES,
    code::BUDGET_COMMAND_COUNT,
    code::EVENT_IGNORED_TERMINAL,
    code::RESULT_STALE,
    code::RESOLVE_NOT_APPLICABLE,
    code::SIGNAL_BUFFER_OVERFLOW,
    code::SIGNAL_OUTCOME_UNROUTED,
    code::INVOCATION_UNKNOWN,
    code::INVOCATION_STUCK,
    code::ACTIVITY_RETRY_SCHEDULED,
    code::LIMIT_VISITS_EXCEEDED,
    code::LIMIT_ACTIVATIONS_EXCEEDED,
    code::MAPPING_MISSING_PATH,
    code::ACTIVITY_UNDECLARED_OUTCOME,
    code::ACTIVITY_ATTEMPTS_EXHAUSTED,
    code::ACTIVITY_FAILED,
    runspore_kernel::code::BUDGET_VALUE_DEPTH,
    runspore_kernel::code::BUDGET_COUNTER_RANGE,
];

fn traces() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/traces")
}

/// The error codes a snapshot holds: the run's result and every failed node result.
fn error_codes(snapshot: &Value, seen: &mut BTreeSet<String>) {
    let mut note = |error: &Value| {
        if let Some(code) = error["code"].as_str() {
            seen.insert(code.to_string());
        }
    };
    note(&snapshot["result"]["error"]);
    if let Some(nodes) = snapshot["nodes"].as_object() {
        for result in nodes.values() {
            note(&result["output"]["error"]);
        }
    }
}

/// One test, so blessing has finished before the files are read again.
#[test]
fn golden_traces_hold_and_cover_the_vocabulary() {
    let count = run_dir(&NativeReducer, traces()).unwrap_or_else(|errors| panic!("{errors}"));
    assert!(
        count >= MINIMUM_TRACES,
        "{count} golden traces, expected at least {MINIMUM_TRACES}"
    );

    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(traces()).expect("trace directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let trace = load(&path).unwrap_or_else(|error| panic!("{error}"));
        assert!(
            names.insert(trace.name.clone()),
            "two traces are named {}",
            trace.name
        );
        assert!(
            !trace.description.is_empty(),
            "{} lacks a description",
            trace.name
        );
        for step in &trace.steps {
            match step
                .expect
                .as_ref()
                .expect("a checked trace has every expect block")
            {
                Expect::Failure(failure) => {
                    seen.insert(failure.code.clone());
                }
                Expect::Decision(decision) => {
                    seen.extend(decision.diagnostics.iter().map(|d| d.code.clone()));
                    error_codes(&decision.snapshot, &mut seen);
                }
            }
        }
    }
    let missing: Vec<&str> = VOCABULARY
        .iter()
        .copied()
        .filter(|code| !seen.contains(*code))
        .collect();
    assert!(missing.is_empty(), "no golden trace expects {missing:?}");
}

/// Traces of a 0.1 document where a step gives up with no `failed` route. Under 0.2
/// that step is stuck instead (`spec/kernel.md` 7.4); the `14*` traces pin it.
const STUCK_UNDER_0_2: &[&str] = &[
    "expired-idempotent-exhausted",
    "failure-without-error",
    "retry-exhausted-no-route",
];

/// Traces of a 0.2 document that pin a rule naming 0.2 (`from`, `$get` `default`,
/// the stuck step): a 0.1 document is refused or runs differently there, by design.
const RULES_NAMING_0_2: &[&str] = &[
    "from-arrows",
    "get-default",
    "stuck-expiries-exhausted",
    "stuck-non-retryable-failure",
    "stuck-retries-exhausted",
    "stuck-undeclared-outcome",
];

/// Outside the rules of `spec/kernel.md` that name a format, a document runs exactly
/// as the same document marked with the other format: every trace holds unchanged
/// with its format switched between 0.1 and 0.2, except the ones that pin such a
/// rule, which must change.
#[test]
fn every_trace_holds_in_the_other_format() {
    if bless_requested() {
        return;
    }
    let mut replayed = 0;
    let mut differ = BTreeSet::new();
    for entry in fs::read_dir(traces()).expect("trace directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let mut trace = load(&path).unwrap_or_else(|error| panic!("{error}"));
        let other = if trace.workflow["format"] == WORKFLOW_FORMAT {
            WORKFLOW_FORMAT_0_2
        } else if trace.workflow["format"] == WORKFLOW_FORMAT_0_2 {
            WORKFLOW_FORMAT
        } else {
            continue;
        };
        trace.workflow["format"] = json!(other);
        if run(&NativeReducer, &trace).is_err() {
            differ.insert(trace.name.clone());
        }
        replayed += 1;
    }
    let expected: BTreeSet<String> = STUCK_UNDER_0_2
        .iter()
        .chain(RULES_NAMING_0_2)
        .map(|n| n.to_string())
        .collect();
    assert_eq!(differ, expected, "traces that differ in the other format");
    // Every trace but the W01 one, whose format no version defines.
    assert!(
        replayed >= MINIMUM_TRACES - 1,
        "{replayed} traces replayed in the other format"
    );
}
