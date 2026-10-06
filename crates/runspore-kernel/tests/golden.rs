//! The golden traces of `conformance/traces` against the native kernel
//! (`spec/kernel.md` section 9).
//!
//! `RUNSPORE_BLESS=1 cargo test -p runspore-kernel --test golden` records what the
//! kernel returns instead of checking it. Read the diff before committing it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use runspore_kernel::NativeReducer;
use runspore_trace::{load, run_dir, Expect};
use runspore_types::model::code;
use serde_json::Value;

/// Files a checkout must hold; more may be added.
const MINIMUM_TRACES: usize = 48;

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
