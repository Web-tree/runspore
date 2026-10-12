//! `spec/kernel.md` section 2: workflow validation rules W01–W11.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, event_kind, WORKFLOW_FORMAT, WORKFLOW_FORMAT_0_2};
use runspore_types::reducer::FailureKind::{IncompatibleVersion, InvalidInput};
use runspore_types::reducer::{Decision, Failure};
use serde_json::{json, Map, Value};

fn check(workflow: &Value) -> Result<Decision, Failure> {
    transition(&start_request(workflow, json!({"repo": "r"})))
}

fn assert_invalid(workflow: &Value) {
    let result = check(workflow);
    assert!(result.is_err(), "accepted: {workflow}");
    assert_failure(result, InvalidInput, code::WORKFLOW_INVALID);
}

fn assert_valid(workflow: &Value) {
    if let Err(failure) = check(workflow) {
        panic!("rejected ({failure}): {workflow}");
    }
}

/// The example workflow with one member replaced.
fn edited(pointer: &str, value: Value) -> Value {
    with(review_loop(), pointer, value)
}

/// `count` complete nodes named `n000`, `n001`, ...; the first is the start.
fn many_nodes(count: usize) -> Value {
    let nodes: Map<String, Value> = (0..count)
        .map(|i| (format!("n{i:03}"), json!({"kind": "complete"})))
        .collect();
    json!({"format": WORKFLOW_FORMAT_0_2, "name": "wide", "start": "n000", "nodes": nodes})
}

/// The example workflow with its `build` node renamed, references included.
fn renamed_node(id: &str) -> Value {
    let mut workflow = review_loop();
    let node = workflow["nodes"]["build"].clone();
    unset(&mut workflow, "/nodes/build");
    workflow["nodes"][id] = node;
    workflow["start"] = json!(id);
    workflow["nodes"][id]["outcomes"]["red"] = json!(id);
    workflow["nodes"]["approve"]["outcomes"]["rejected"] = json!(id);
    workflow["nodes"]["done"]["output"] = json!(null);
    workflow
}

/// The example workflow with its `build` action renamed, the reference included.
fn renamed_action(id: &str) -> Value {
    let mut workflow = review_loop();
    let action = workflow["actions"]["build"].clone();
    workflow["actions"] = json!({ id: action });
    workflow["nodes"]["build"]["action"] = json!(id);
    workflow
}

/// A workflow whose single activity carries `mapping` as its input.
fn with_input(mapping: Value) -> Value {
    edited("/nodes/build/input", mapping)
}

#[test]
fn the_example_workflow_is_valid() {
    assert_valid(&review_loop());
}

#[test]
fn the_document_must_decode_as_a_workflow() {
    assert_invalid(&edited("/nodes/approve/kind", json!("timer")));
    assert_invalid(&edited("/nodes/approve/extra", json!(1)));
    assert_invalid(&edited("/limits/extra", json!(1)));
    assert_invalid(&edited("/limits/maxVisitsPerNode", json!("16")));
    assert_invalid(&edited("/limits/maxVisitsPerNode", json!(-1)));
    assert_invalid(&edited("/nodes/broken/error/extra", json!(1)));
    assert_invalid(&edited("/nodes/build/outcomes/ok", json!(7)));
    assert_invalid(&without(review_loop(), "/start"));
    assert_invalid(&without(review_loop(), "/name"));
    assert_invalid(&without(review_loop(), "/format"));
    assert_invalid(&without(review_loop(), "/nodes/build/action"));
    assert_invalid(&without(review_loop(), "/nodes/approve/signal"));
    assert_invalid(&without(review_loop(), "/nodes/broken/error"));
}

#[test]
fn limits_and_actions_are_optional() {
    assert_valid(&without(review_loop(), "/limits"));
    assert_valid(&edited("/limits", json!({})));
    assert_valid(&many_nodes(1));
}

#[test]
fn w01_format() {
    assert_valid(&edited("/format", json!(WORKFLOW_FORMAT)));
    assert_valid(&edited("/format", json!(WORKFLOW_FORMAT_0_2)));
    assert_invalid(&edited("/format", json!("runspore.workflow/0.3")));
    assert_invalid(&edited("/format", json!("")));
}

/// Outside the rules that name 0.2, a 0.1 document is still accepted and judged by
/// the same rules as a 0.2 one: the same documents are refused, with the same code.
#[test]
fn a_0_1_document_follows_the_0_2_rules() {
    let refused = [
        edited("/name", json!("has space")),
        edited("/start", json!("missing")),
        edited("/limits/maxVisitsPerNode", json!(0)),
        edited("/actions/build/outcomes", json!(["ok", "failed"])),
        edited("/actions/build/effect", json!("reconcilable")),
        edited("/nodes/build/outcomes/exhausted", json!("done")),
        with_input(json!({"$get": ["elsewhere"]})),
        with_input(json!({"$get": ["input"], "other": 1})),
    ];
    for workflow in refused {
        let expected = check(&workflow).expect_err("refused as 0.2");
        let actual =
            check(&with(workflow, "/format", json!(WORKFLOW_FORMAT))).expect_err("refused as 0.1");
        assert_eq!((actual.kind, actual.code), (expected.kind, expected.code));
    }
}

#[test]
fn w02_name() {
    for name in ["", "has space", "ünï", "a/b", &"a".repeat(129)] {
        assert_invalid(&edited("/name", json!(name)));
    }
    for name in ["a", "A_z.0-9", &"a".repeat(128)] {
        assert_valid(&edited("/name", json!(name)));
    }
}

#[test]
fn w03_node_count() {
    assert_invalid(&edited("/nodes", json!({})));
    assert_valid(&many_nodes(256));
    assert_invalid(&many_nodes(257));
}

#[test]
fn w03_node_ids() {
    for id in ["", "has space", "a.b", "a/b", "ünï", &"a".repeat(65)] {
        assert_invalid(&renamed_node(id));
    }
    for id in ["a", "A_z-09", &"a".repeat(64)] {
        assert_valid(&renamed_node(id));
    }
}

#[test]
fn w04_action_ids() {
    for id in ["", "has space", "a.b", "ünï", &"a".repeat(65)] {
        assert_invalid(&renamed_action(id));
    }
    for id in ["a", "A_z-09", &"a".repeat(64)] {
        assert_valid(&renamed_action(id));
    }
}

#[test]
fn w05_start_names_a_node() {
    assert_invalid(&edited("/start", json!("nowhere")));
    assert_invalid(&edited("/start", json!("")));
}

#[test]
fn w06_graph_limits() {
    for visits in [0, 1_001] {
        assert_invalid(&edited("/limits/maxVisitsPerNode", json!(visits)));
    }
    for activations in [0, 10_001] {
        assert_invalid(&edited("/limits/maxActivations", json!(activations)));
    }
    for (visits, activations) in [(1, 1), (1_000, 10_000)] {
        assert_valid(&edited(
            "/limits",
            json!({"maxVisitsPerNode": visits, "maxActivations": activations}),
        ));
    }
}

#[test]
fn w07_each_action_parses_as_a_policy() {
    for action in [
        json!(null),
        json!("make"),
        json!([]),
        json!({}),
        json!({"outcomes": ["ok", "red"]}),
        json!({"effect": "magic", "outcomes": ["ok", "red"]}),
        json!({"effect": "pure", "outcomes": "ok"}),
        json!({"effect": "pure", "outcomes": ["ok", "red"], "retry": {"extra": 1}}),
        json!({"effect": "pure", "outcomes": ["ok", "red"], "retry": {"backoffMs": 500}}),
        json!({"effect": "pure", "outcomes": ["ok", "red"], "retry": {"maxBackoffMs": "01"}}),
        json!({"effect": "pure", "outcomes": ["ok", "red"], "retry": {"maxAttempts": "3"}}),
    ] {
        assert_invalid(&edited("/actions/build", action));
    }
}

#[test]
fn w07_outcomes() {
    let long = "a".repeat(65);
    for outcomes in [
        json!([]),
        json!(["ok", "red", "ok"]),
        json!(["ok", "red", "failed"]),
        json!(["ok", "red", "Bad"]),
        json!(["ok", "red", "-x"]),
        json!(["ok", "red", ""]),
        json!(["ok", "red", "a_b"]),
        json!(["ok", "red", long]),
    ] {
        assert_invalid(&edited("/actions/build/outcomes", outcomes));
    }
    let widest = format!("0{}", "-".repeat(63));
    let mut workflow = edited("/actions/build/outcomes", json!(["ok", "red", widest]));
    set(
        &mut workflow,
        &format!("/nodes/build/outcomes/{widest}"),
        json!("done"),
    );
    assert_valid(&workflow);
}

#[test]
fn w07_outcomes_default_to_ok() {
    let mut workflow = without(review_loop(), "/actions/build/outcomes");
    assert_invalid(&workflow);
    unset(&mut workflow, "/nodes/build/outcomes/red");
    assert_valid(&workflow);
}

#[test]
fn w07_max_attempts() {
    for attempts in [0, 101] {
        assert_invalid(&edited("/actions/build/retry/maxAttempts", json!(attempts)));
    }
    for attempts in [1, 100] {
        assert_valid(&edited("/actions/build/retry/maxAttempts", json!(attempts)));
    }
    assert_valid(&without(review_loop(), "/actions/build/retry"));
}

#[test]
fn w07_applies_to_unreferenced_actions_and_ignores_host_fields() {
    assert_invalid(&edited("/actions/spare", json!({"kind": "command"})));
    assert_valid(&edited(
        "/actions/spare",
        json!({"kind": "native", "function": "f", "effect": "unsafe", "timeoutMs": "5"}),
    ));
}

#[test]
fn w07_reconcilable_effect_is_unsupported() {
    let result = check(&edited("/actions/build/effect", json!("reconcilable")));
    assert_failure(
        result,
        IncompatibleVersion,
        code::WORKFLOW_UNSUPPORTED_EFFECT,
    );
}

#[test]
fn w07_unsupported_effect_keeps_its_place_in_the_rule_order() {
    let reconcilable = json!({"effect": "reconcilable"});
    let broken = json!({"effect": "pure", "outcomes": []});

    let earlier_rule = with(
        edited("/actions/spare", reconcilable.clone()),
        "/limits/maxActivations",
        json!(0),
    );
    assert_invalid(&earlier_rule);

    let later_rule = with(
        edited("/actions/spare", reconcilable.clone()),
        "/nodes/build/outcomes/ok",
        json!("nowhere"),
    );
    assert_failure(
        check(&later_rule),
        IncompatibleVersion,
        code::WORKFLOW_UNSUPPORTED_EFFECT,
    );

    let earlier_action = with(
        edited("/actions/a", broken.clone()),
        "/actions/b",
        reconcilable.clone(),
    );
    assert_invalid(&earlier_action);

    let later_action = with(edited("/actions/a", reconcilable), "/actions/b", broken);
    assert_failure(
        check(&later_action),
        IncompatibleVersion,
        code::WORKFLOW_UNSUPPORTED_EFFECT,
    );
}

#[test]
fn w08_activity_nodes() {
    assert_invalid(&edited("/nodes/build/action", json!("missing")));
    assert_invalid(&without(review_loop(), "/nodes/build/outcomes/red"));
    assert_invalid(&edited("/nodes/build/outcomes/extra", json!("done")));
    assert_invalid(&edited("/nodes/build/outcomes/ok", json!("nowhere")));
    assert_invalid(&edited("/nodes/build/outcomes/failed", json!("nowhere")));
    assert_invalid(&without(review_loop(), "/nodes/build/outcomes"));
    assert_valid(&without(review_loop(), "/nodes/build/outcomes/failed"));
    assert_valid(&without(review_loop(), "/nodes/build/input"));
}

#[test]
fn w09_await_signal_nodes() {
    for signal in ["", "has space", "a/b", "ünï", &"a".repeat(65)] {
        assert_invalid(&edited("/nodes/approve/signal", json!(signal)));
    }
    for signal in ["a", "A_z.0-9", &"a".repeat(64)] {
        assert_valid(&edited("/nodes/approve/signal", json!(signal)));
    }
    assert_invalid(&edited("/nodes/approve/outcomes", json!({})));
    assert_invalid(&edited("/nodes/approve/outcomes/Approved", json!("done")));
    assert_invalid(&edited("/nodes/approve/outcomes/-x", json!("done")));
    assert_invalid(&edited(
        "/nodes/approve/outcomes/approved",
        json!("nowhere"),
    ));
    assert_valid(&edited("/nodes/approve/outcomes/failed", json!("broken")));
    assert_valid(&edited("/nodes/approve/outcomes/received", json!("done")));
}

#[test]
fn w10_mappings_are_well_formed() {
    for mapping in [
        json!({"$get": ["input"], "other": 1}),
        json!({"$literal": 1, "other": 1}),
        json!({"$get": ["input"], "$literal": 1}),
        json!({"$unknown": 1}),
        json!({"plain": 1, "$unknown": 1}),
        json!({"$": 1}),
        json!({"$get": "input"}),
        json!({"$get": null}),
        json!({"$get": {"path": ["input"]}}),
        json!({"$get": []}),
        json!({"$get": ["elsewhere", "x"]}),
        json!({"$get": [0, "x"]}),
        json!({"$get": ["input", -1]}),
        json!({"$get": ["input", true]}),
        json!({"$get": ["input", null]}),
        json!({"$get": ["input", ["x"]]}),
        json!({"$get": ["input", {"x": 1}]}),
        json!({"a": {"b": [1, {"$get": []}]}}),
        json!([[{"$oops": null}]]),
    ] {
        assert_invalid(&with_input(mapping.clone()));
        assert_invalid(&edited("/nodes/done/output", mapping));
    }
}

#[test]
fn w10_accepts_every_documented_form() {
    for mapping in [
        json!(null),
        json!(true),
        json!(7),
        json!("$not-a-key"),
        json!([]),
        json!({}),
        json!({"$get": ["input"]}),
        json!({"$get": ["nodes"]}),
        json!({"$get": ["run", "id"]}),
        json!({"$get": ["input", "repo", 0, "x"]}),
        json!({"$literal": {"$get": 5, "$unknown": [{"$literal": 1, "x": 2}]}}),
        json!({"$literal": null}),
        json!({"a": [1, "two", {"b": {"$literal": "$x"}}], "dollar$inside": 1}),
    ] {
        assert_valid(&edited("/nodes/done/output", mapping));
    }
}

/// W10 in 0.2: `from` may start a path, and `$get` may carry `default` and no other
/// key. A 0.1 document refuses both.
#[test]
fn w10_a_0_2_document_admits_from_and_default() {
    let as_0_2 = |workflow: Value| with(workflow, "/format", json!(WORKFLOW_FORMAT_0_2));
    let as_0_1 = |workflow: Value| with(workflow, "/format", json!(WORKFLOW_FORMAT));
    for mapping in [
        json!({"$get": ["from"]}),
        json!({"$get": ["from", "output", "log", 0]}),
        json!({"$get": ["nodes", "test"], "default": null}),
        json!({"$get": ["from", "node"], "default": {"$get": 5, "$unknown": 1}}),
        json!({"default": 1}),
    ] {
        assert_valid(&as_0_2(with_input(mapping.clone())));
        assert_valid(&as_0_2(edited("/nodes/done/output", mapping.clone())));
        if mapping.get("default") != Some(&json!(1)) {
            assert_invalid(&as_0_1(with_input(mapping)));
        }
    }
    for mapping in [
        json!({"$get": ["input"], "default": 1, "other": 1}),
        json!({"$get": ["input"], "fallback": 1}),
        json!({"$literal": 1, "default": 1}),
        json!({"$get": ["From"]}),
        json!({"$get": [], "default": 1}),
        json!({"$get": ["from", -1], "default": 1}),
    ] {
        assert_invalid(&as_0_2(with_input(mapping)));
    }
}

#[test]
fn w11_fail_nodes() {
    for bad in ["", "Bad", "-x", "a_b", "has space", &"a".repeat(65)] {
        assert_invalid(&edited("/nodes/broken/error/code", json!(bad)));
    }
    for good in ["a", "0", "a.b-c.9", &"a".repeat(64)] {
        assert_valid(&edited("/nodes/broken/error/code", json!(good)));
    }
    assert_invalid(&edited("/nodes/broken/error/nodeId", json!("broken")));
    assert_valid(&edited("/nodes/broken/error/nodeId", json!(null)));
    assert_valid(&edited(
        "/nodes/broken/error/details",
        json!({"hint": [1, 2]}),
    ));
}

#[test]
fn validation_runs_on_every_transition() {
    let mut run = Run::started(&review_loop(), json!({"repo": "r"}));
    run.graph = canon(&edited("/start", json!("nowhere")));
    let result = run.apply(
        event_kind::SIGNAL_RECEIVED,
        &json!({"name": "approval"}),
        2_000,
    );
    assert_failure(result, InvalidInput, code::WORKFLOW_INVALID);
}
