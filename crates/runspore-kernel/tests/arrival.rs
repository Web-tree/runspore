//! `spec/kernel.md` section 4, 0.2 only: `from` in the mapping context and `$get`
//! with a `default`.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, WORKFLOW_FORMAT_0_2};
use runspore_types::reducer::FailureKind::ResourceLimit;
use serde_json::{json, Value};

/// `work` sees why it was entered; `judge` sends it back on `red` and on `failed`;
/// `approve` sends it back on `rejected`. `done` reports the arrow into it.
fn coding_loop() -> Value {
    json!({
        "format": WORKFLOW_FORMAT_0_2,
        "name": "coding-loop",
        "start": "work",
        "actions": {
            "act": {"effect": "idempotent"},
            "judge": {"effect": "read-only", "outcomes": ["ok", "red"]}
        },
        "nodes": {
            "work": {"kind": "activity", "action": "act",
                     "input": {"from": {"$get": ["from"]},
                               "lastJudge": {"$get": ["nodes", "judge"], "default": null}},
                     "outcomes": {"ok": "judge"}},
            "judge": {"kind": "activity", "action": "judge",
                      "input": {"by": {"$get": ["from", "node"]}},
                      "outcomes": {"ok": "approve", "red": "work", "failed": "work"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done", "rejected": "work"}},
            "done": {"kind": "complete", "output": {"$get": ["from"]}}
        }
    })
}

/// The input of the activity the decision scheduled.
fn scheduled(decision: &runspore_types::reducer::Decision) -> Value {
    assert_eq!(decision.commands.len(), 1, "one schedule command");
    parse(&decision.commands[0].payload)["input"].clone()
}

#[test]
fn the_start_node_sees_null() {
    let mut run = Run::new(&coding_loop());
    let decision = run.start(json!(null));
    assert_eq!(
        scheduled(&decision),
        json!({"from": null, "lastJudge": null})
    );
}

#[test]
fn a_loop_back_sees_the_judges_outcome_and_output() {
    let mut run = Run::started(&coding_loop(), json!(null));
    let decision = run.succeed("ok", json!({"summary": "first"}), 2_000);
    assert_eq!(scheduled(&decision), json!({"by": "work"}));

    let decision = run.succeed("red", json!({"failing": 2}), 3_000);
    let judged = json!({"visit": 1, "outcome": "red", "output": {"failing": 2}});
    assert_eq!(
        scheduled(&decision),
        json!({"from": {"node": "judge", "outcome": "red", "output": {"failing": 2}},
               "lastJudge": judged})
    );
}

#[test]
fn a_failed_arrow_shows_as_failed_with_the_error() {
    let mut run = Run::started(&coding_loop(), json!(null));
    run.succeed("ok", json!(null), 2_000);
    let decision = run.fail(false, 3_000);
    let error = json!({"error": {"code": "io.broken", "message": "boom",
                                 "nodeId": "judge", "details": null}});
    assert_eq!(
        scheduled(&decision)["from"],
        json!({"node": "judge", "outcome": "failed", "output": error})
    );
}

#[test]
fn a_signal_arrow_shows_the_signal_outcome_and_data() {
    let mut run = Run::started(&coding_loop(), json!(null));
    run.succeed("ok", json!(null), 2_000);
    run.succeed("ok", json!(null), 3_000);
    let decision = run.signal(
        "approval",
        Some("rejected"),
        json!({"hint": "smaller"}),
        4_000,
    );
    assert_eq!(
        scheduled(&decision)["from"],
        json!({"node": "approve", "outcome": "rejected", "output": {"hint": "smaller"}})
    );

    // A signal buffered before its waiter: the arrow is the waiter's, not the judge's.
    run.signal("approval", Some("approved"), json!({"by": "ana"}), 5_000);
    run.succeed("ok", json!(null), 6_000);
    run.succeed("ok", json!(null), 7_000);
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["result"]["output"],
        json!({"node": "approve", "outcome": "approved", "output": {"by": "ana"}})
    );
}

#[test]
fn a_path_into_null_from_does_not_resolve() {
    let workflow = with(
        coding_loop(),
        "/nodes/work/input",
        json!({"$get": ["from", "node"]}),
    );
    let mut run = Run::new(&workflow);
    run.start(json!(null));
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(
        state["result"]["error"]["code"],
        json!(code::MAPPING_MISSING_PATH)
    );
    assert_eq!(
        state["result"]["error"]["details"],
        json!({"path": ["from", "node"]})
    );
}

#[test]
fn a_default_is_taken_unevaluated_only_when_the_path_is_missing() {
    let mapping = json!({
        "missing": {"$get": ["from", "output", "log"], "default": {"$get": ["input"]}},
        "present": {"$get": ["input", "x"], "default": "unused"}
    });
    let workflow = with(coding_loop(), "/nodes/work/input", mapping);
    let mut run = Run::new(&workflow);
    let decision = run.start(json!({"x": 7}));
    assert_eq!(
        scheduled(&decision),
        json!({"missing": {"$get": ["input"]}, "present": 7})
    );
}

/// A `$get` costs one operation and one per step; falling back costs one more.
#[test]
fn a_default_taken_costs_one_more_operation() {
    let workflow = with(
        coding_loop(),
        "/nodes/work/input",
        json!({"$get": ["input", "absent"], "default": 1}),
    );
    let mut request = start_request(&workflow, json!({}));
    request.frozen_limits.expression_operations = 3;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_EXPRESSION_OPERATIONS,
    );
    request.frozen_limits.expression_operations = 4;
    assert!(transition(&request).is_ok());

    let workflow = with(
        coding_loop(),
        "/nodes/work/input",
        json!({"$get": ["input", "here"], "default": 1}),
    );
    let mut request = start_request(&workflow, json!({"here": 0}));
    request.frozen_limits.expression_operations = 3;
    assert!(transition(&request).is_ok());
}
