//! `spec/kernel.md` 7.3–7.4: entering nodes, routing by outcome, loop limits,
//! terminal states, and mappings (section 4).

mod common;

use common::*;
use runspore_types::digest::{self, ids};
use runspore_types::model::{code, command_kind, event_kind, WORKFLOW_FORMAT};
use runspore_types::reducer::Decision;
use serde_json::{json, Value};

fn linear() -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "linear",
        "start": "a",
        "actions": {"act": {"effect": "idempotent"}},
        "nodes": {
            "a": {"kind": "activity", "action": "act",
                  "input": {"x": {"$get": ["input", "x"]}},
                  "outcomes": {"ok": "b"}},
            "b": {"kind": "activity", "action": "act",
                  "input": {"prev": {"$get": ["nodes", "a", "output"]}},
                  "outcomes": {"ok": "done"}},
            "done": {"kind": "complete",
                     "output": {"a": {"$get": ["nodes", "a", "output"]},
                                "b": {"$get": ["nodes", "b", "output", "v"]}}}
        }
    })
}

/// A self-loop on `again`, an exit on `ok`.
fn looping(max_visits: u32, max_activations: u32) -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "looping",
        "start": "ping",
        "limits": {"maxVisitsPerNode": max_visits, "maxActivations": max_activations},
        "actions": {"act": {"effect": "pure", "outcomes": ["again", "ok", "other"]}},
        "nodes": {
            "ping": {"kind": "activity", "action": "act",
                     "outcomes": {"again": "ping", "ok": "done", "other": "pong"}},
            "pong": {"kind": "activity", "action": "act",
                     "outcomes": {"again": "pong", "ok": "done", "other": "ping"}},
            "done": {"kind": "complete"}
        }
    })
}

/// `first` runs, then `second` is entered with `mapping` as its input.
fn mapped(mapping: Value) -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "mapped",
        "start": "first",
        "actions": {"act": {"effect": "pure"}},
        "nodes": {
            "first": {"kind": "activity", "action": "act", "outcomes": {"ok": "second"}},
            "second": {"kind": "activity", "action": "act", "input": mapping,
                       "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn run_input() -> Value {
    json!({"x": 7, "list": [10, 20, {"deep": "value"}], "obj": {"k": "v"}, "nil": null})
}

/// Enters `second` with `mapping` evaluated against a fixed context.
fn evaluate(mapping: Value) -> (Run, Decision) {
    let mut run = Run::started(&mapped(mapping), run_input());
    let decision = run.succeed("ok", json!({"k": [1, 2]}), 2_000);
    (run, decision)
}

/// The input the mapping produced, read from the schedule command.
fn evaluated(mapping: Value) -> Value {
    let (_, decision) = evaluate(mapping);
    assert_eq!(decision.commands.len(), 1, "mapping did not resolve");
    parse(&decision.commands[0].payload)["input"].clone()
}

fn assert_missing(mapping: Value, path: Value) {
    let (run, decision) = evaluate(mapping);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(
        state["result"],
        json!({"error": {
            "code": code::MAPPING_MISSING_PATH,
            "message": "mapping path does not resolve",
            "nodeId": "second",
            "details": {"path": path}
        }})
    );
    assert_eq!(state["visits"], json!({"first": 1, "second": 1}));
    assert_eq!(state["activations"], json!(2));
}

#[test]
fn run_started_enters_the_start_node() {
    let mut run = Run::new(&linear());
    let decision = run.start(json!({"x": 7}));
    let invocation = ids::invocation(TENANT, RUN_ID, "a/1");

    assert_eq!(
        parse(&decision.snapshot),
        json!({
            "format": "runspore.state/0.1",
            "semantics": "0.1",
            "tenant": "default",
            "runId": "run_1",
            "status": "running",
            "input": {"x": 7},
            "nodes": {},
            "visits": {"a": 1},
            "activations": 1,
            "position": {"nodeId": "a", "activationId": "a/1", "visit": 1},
            "invocation": {
                "invocationId": invocation,
                "activationId": "a/1",
                "nodeId": "a",
                "actionId": "act",
                "attempt": 1,
                "state": "scheduled",
                "inputDigest": digest::body(br#"{"x":7}"#)
            },
            "signals": [],
            "result": null,
            "lastSequence": "1",
            "lastAcceptedAtMs": "1000"
        })
    );
    assert_eq!(decision.snapshot_digest, digest::state(&decision.snapshot));
    assert!(decision.diagnostics.is_empty());

    assert_eq!(decision.commands.len(), 1);
    let command = &decision.commands[0];
    assert_eq!(command.command_id, ids::command(TENANT, RUN_ID, "a/1", 1));
    assert_eq!(command.activation_id, "a/1");
    assert_eq!(command.kind, command_kind::ACTIVITY_SCHEDULE);
    assert_eq!(
        parse(&command.payload),
        json!({
            "invocationId": invocation,
            "activationId": "a/1",
            "nodeId": "a",
            "actionId": "act",
            "effectKey": invocation,
            "attempt": 1,
            "notBeforeMs": "1000",
            "input": {"x": 7}
        })
    );
}

#[test]
fn a_linear_workflow_runs_to_completion() {
    let mut run = Run::started(&linear(), json!({"x": 7}));

    let decision = run.succeed("ok", json!({"sum": 8}), 2_000);
    let state = run.state();
    assert_eq!(state["status"], json!("running"));
    assert_eq!(
        state["nodes"],
        json!({"a": {"visit": 1, "outcome": "ok", "output": {"sum": 8}}})
    );
    assert_eq!(
        state["position"],
        json!({"nodeId": "b", "activationId": "b/1", "visit": 1})
    );
    assert_eq!(state["lastSequence"], json!("2"));
    assert_eq!(state["lastAcceptedAtMs"], json!("2000"));
    assert_eq!(decision.commands.len(), 1);
    let payload = parse(&decision.commands[0].payload);
    assert_eq!(payload["nodeId"], json!("b"));
    assert_eq!(payload["input"], json!({"prev": {"sum": 8}}));
    assert_eq!(payload["notBeforeMs"], json!("2000"));
    assert_eq!(
        state["invocation"]["inputDigest"],
        json!(digest::body(br#"{"prev":{"sum":8}}"#))
    );

    let decision = run.succeed("ok", json!({"v": [1, 2]}), 3_000);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(
        state["result"],
        json!({"output": {"a": {"sum": 8}, "b": [1, 2]}})
    );
    assert_eq!(state["visits"], json!({"a": 1, "b": 1, "done": 1}));
    assert_eq!(state["activations"], json!(3));
}

#[test]
fn a_success_without_an_outcome_is_ok_and_output_defaults_to_null() {
    let mut run = Run::started(&linear(), json!({"x": 7}));
    let payload = run.result("success");
    run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(
        run.state()["nodes"]["a"],
        json!({"visit": 1, "outcome": "ok", "output": null})
    );
}

#[test]
fn an_absent_mapping_evaluates_to_null() {
    let mut run = Run::new(&single_activity("pure", json!({}), false));
    let decision = run.start(json!({"ignored": true}));
    assert_eq!(parse(&decision.commands[0].payload)["input"], json!(null));
    assert_eq!(
        run.state()["invocation"]["inputDigest"],
        json!(digest::body(b"null"))
    );
}

#[test]
fn the_outcome_selects_the_route() {
    let mut run = Run::started(&review_loop(), json!({"repo": "r"}));
    run.succeed("red", json!("first try"), 2_000);
    let state = run.state();
    assert_eq!(
        state["position"],
        json!({"nodeId": "build", "activationId": "build/2", "visit": 2})
    );
    assert_eq!(
        state["nodes"]["build"],
        json!({"visit": 1, "outcome": "red", "output": "first try"})
    );

    run.succeed("ok", json!("second try"), 3_000);
    let state = run.state();
    assert_eq!(state["status"], json!("waiting"));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(
        state["position"],
        json!({"nodeId": "approve", "activationId": "approve/1", "visit": 1})
    );
    assert_eq!(
        state["nodes"]["build"],
        json!({"visit": 2, "outcome": "ok", "output": "second try"})
    );

    run.signal("approval", Some("approved"), json!({"by": "ana"}), 4_000);
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(state["result"], json!({"output": "second try"}));
}

/// K4: each visit has its own activation and invocation; the effect key follows.
#[test]
fn each_visit_has_distinct_activation_and_invocation_ids() {
    let mut run = Run::new(&review_loop());
    let first = run.start(json!({"repo": "r"}));
    let second = run.succeed("red", json!(null), 2_000);
    let first = parse(&first.commands[0].payload);
    let second_command = &second.commands[0];
    let second = parse(&second_command.payload);

    assert_eq!(first["activationId"], json!("build/1"));
    assert_eq!(second["activationId"], json!("build/2"));
    assert_eq!(
        second["invocationId"],
        json!(ids::invocation(TENANT, RUN_ID, "build/2"))
    );
    assert_ne!(first["invocationId"], second["invocationId"]);
    assert_eq!(second["effectKey"], second["invocationId"]);
    assert_eq!(second["attempt"], json!(1));
    assert_eq!(
        second_command.command_id,
        ids::command(TENANT, RUN_ID, "build/2", 1)
    );
}

#[test]
fn an_undeclared_outcome_fails_the_invocation() {
    for outcome in ["blue", "failed"] {
        let mut run = Run::started(&single_activity("pure", json!({}), true), json!(null));
        let decision = run.succeed(outcome, json!("ignored"), 2_000);
        assert!(decision.commands.is_empty());
        let state = run.state();
        assert_eq!(state["status"], json!("completed"));
        assert_eq!(
            state["result"],
            json!({"output": {"visit": 1, "outcome": "failed", "output": {"error": {
                "code": code::ACTIVITY_UNDECLARED_OUTCOME,
                "message": "activity reported an undeclared outcome",
                "nodeId": "work",
                "details": {"outcome": outcome}
            }}}})
        );
        assert_eq!(state["visits"], json!({"work": 1, "recover": 1}));
    }
}

#[test]
fn an_undeclared_outcome_without_a_failed_route_ends_the_run() {
    let mut run = Run::started(&single_activity("pure", json!({}), false), json!(null));
    run.succeed("blue", json!("ignored"), 2_000);
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(state["nodes"], json!({}));
    assert_eq!(
        state["result"],
        json!({"error": {
            "code": code::ACTIVITY_UNDECLARED_OUTCOME,
            "message": "activity reported an undeclared outcome",
            "nodeId": "work",
            "details": {"outcome": "blue"}
        }})
    );
}

#[test]
fn a_back_edge_stops_at_max_visits_per_node() {
    let mut run = Run::started(&looping(3, 256), json!(null));
    run.succeed("again", json!(1), 2_000);
    let third = run.succeed("again", json!(2), 3_000);
    assert_eq!(third.commands[0].activation_id, "ping/3");
    assert_eq!(run.status(), "running");

    let decision = run.succeed("again", json!(3), 4_000);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(
        state["result"],
        json!({"error": {
            "code": code::LIMIT_VISITS_EXCEEDED,
            "message": "node visit limit exceeded",
            "nodeId": "ping",
            "details": null
        }})
    );
    assert_eq!(state["visits"], json!({"ping": 3}));
    assert_eq!(state["activations"], json!(3));
    assert_eq!(
        state["nodes"]["ping"],
        json!({"visit": 3, "outcome": "again", "output": 3})
    );
}

#[test]
fn a_run_stops_at_max_activations() {
    let mut run = Run::started(&looping(1_000, 3), json!(null));
    run.succeed("other", json!(null), 2_000);
    run.succeed("other", json!(null), 3_000);
    assert_eq!(run.state()["visits"], json!({"ping": 2, "pong": 1}));

    let decision = run.succeed("other", json!(null), 4_000);
    assert!(decision.commands.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(
        state["result"],
        json!({"error": {
            "code": code::LIMIT_ACTIVATIONS_EXCEEDED,
            "message": "run activation limit exceeded",
            "nodeId": "pong",
            "details": null
        }})
    );
    assert_eq!(state["visits"], json!({"ping": 2, "pong": 1}));
    assert_eq!(state["activations"], json!(3));
}

#[test]
fn the_visit_limit_is_checked_before_the_activation_limit() {
    let mut run = Run::started(&looping(1, 1), json!(null));
    run.succeed("again", json!(null), 2_000);
    assert_eq!(
        run.state()["result"]["error"]["code"],
        json!(code::LIMIT_VISITS_EXCEEDED)
    );

    let mut run = Run::started(&looping(1, 1), json!(null));
    run.succeed("ok", json!(null), 2_000);
    assert_eq!(
        run.state()["result"]["error"],
        json!({
            "code": code::LIMIT_ACTIVATIONS_EXCEEDED,
            "message": "run activation limit exceeded",
            "nodeId": "done",
            "details": null
        })
    );
}

#[test]
fn a_fail_node_ends_the_run_with_its_error() {
    let mut workflow = single_activity("pure", json!({}), true);
    set(
        &mut workflow,
        "/nodes/recover",
        json!({"kind": "fail", "error": {"code": "work.gave-up", "message": "no luck",
                                         "details": {"hint": 1}}}),
    );
    let mut run = Run::started(&workflow, json!(null));
    let decision = run.fail(false, 2_000);
    assert!(decision.commands.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(
        state["result"],
        json!({"error": {"code": "work.gave-up", "message": "no luck", "nodeId": "recover",
                         "details": {"hint": 1}}})
    );
    assert_eq!(state["visits"], json!({"work": 1, "recover": 1}));
}

#[test]
fn a_start_node_may_be_terminal() {
    let workflow = json!({
        "format": WORKFLOW_FORMAT, "name": "instant", "start": "done",
        "nodes": {"done": {"kind": "complete", "output": {"echo": {"$get": ["input"]}}}}
    });
    let mut run = Run::new(&workflow);
    let decision = run.start(json!([1, 2]));
    assert!(decision.commands.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(state["result"], json!({"output": {"echo": [1, 2]}}));
    assert_eq!(state["visits"], json!({"done": 1}));
}

/// K3: a terminal state consumes every event and never changes status.
#[test]
fn events_after_a_terminal_state_are_ignored() {
    for terminal in ["ok", "blue"] {
        let mut run = Run::started(&single_activity("pure", json!({}), false), json!(null));
        let invocation = run.invocation_id();
        run.succeed(terminal, json!(1), 2_000);
        let before = run.state();
        assert!(before["status"] == json!("completed") || before["status"] == json!("failed"));

        let events = [
            (
                event_kind::ACTIVITY_RESULT,
                result_for(&invocation, 1, "success"),
            ),
            (event_kind::SIGNAL_RECEIVED, json!({"name": "anything"})),
            (
                event_kind::INVOCATION_RESOLVED,
                json!({"invocationId": invocation, "resolution": {"action": "retry"}}),
            ),
        ];
        for (index, (kind, payload)) in events.iter().enumerate() {
            let sequence = 3 + index as u64;
            let decision = run.step(kind, payload, 3_000 + index as u64);
            assert!(decision.commands.is_empty());
            assert_eq!(
                diagnostics(&decision),
                json!([{"code": code::EVENT_IGNORED_TERMINAL, "nodeId": null,
                        "details": {"kind": kind}}])
            );
            let mut expected = before.clone();
            set(&mut expected, "/lastSequence", json!(sequence.to_string()));
            set(
                &mut expected,
                "/lastAcceptedAtMs",
                json!((3_000 + index as u64).to_string()),
            );
            assert_eq!(run.state(), expected);
        }
    }
}

#[test]
fn logical_time_never_decreases() {
    let mut run = Run::started(&linear(), json!({"x": 7}));
    let decision = run.succeed("ok", json!(null), 400);
    assert_eq!(run.state()["lastAcceptedAtMs"], json!("1000"));
    assert_eq!(
        parse(&decision.commands[0].payload)["notBeforeMs"],
        json!("1000")
    );
    run.signal("noise", None, json!(null), 5_000);
    assert_eq!(run.state()["lastAcceptedAtMs"], json!("5000"));
    run.signal("noise", None, json!(null), 4_999);
    assert_eq!(run.state()["lastAcceptedAtMs"], json!("5000"));
    assert_eq!(run.state()["lastSequence"], json!("4"));
}

#[test]
fn get_projects_from_the_run_input() {
    assert_eq!(evaluated(json!({"$get": ["input"]})), run_input());
    assert_eq!(evaluated(json!({"$get": ["input", "x"]})), json!(7));
    assert_eq!(evaluated(json!({"$get": ["input", "list", 1]})), json!(20));
    assert_eq!(
        evaluated(json!({"$get": ["input", "list", 2, "deep"]})),
        json!("value")
    );
    assert_eq!(evaluated(json!({"$get": ["input", "nil"]})), json!(null));
}

#[test]
fn get_projects_from_node_results() {
    let first = json!({"visit": 1, "outcome": "ok", "output": {"k": [1, 2]}});
    assert_eq!(
        evaluated(json!({"$get": ["nodes"]})),
        json!({"first": first})
    );
    assert_eq!(evaluated(json!({"$get": ["nodes", "first"]})), first);
    assert_eq!(
        evaluated(json!({"$get": ["nodes", "first", "visit"]})),
        json!(1)
    );
    assert_eq!(
        evaluated(json!({"$get": ["nodes", "first", "outcome"]})),
        json!("ok")
    );
    assert_eq!(
        evaluated(json!({"$get": ["nodes", "first", "output", "k", 0]})),
        json!(1)
    );
}

#[test]
fn get_projects_from_the_run_identity() {
    assert_eq!(
        evaluated(json!({"$get": ["run"]})),
        json!({"id": RUN_ID, "tenant": TENANT})
    );
    assert_eq!(evaluated(json!({"$get": ["run", "id"]})), json!(RUN_ID));
    assert_eq!(evaluated(json!({"$get": ["run", "tenant"]})), json!(TENANT));
}

#[test]
fn literal_is_not_evaluated() {
    let inner = json!({"$get": ["input", "absent"], "$anything": [{"$literal": 1}]});
    assert_eq!(evaluated(json!({"$literal": inner})), inner);
    assert_eq!(evaluated(json!({"$literal": null})), json!(null));
}

#[test]
fn objects_and_arrays_are_evaluated_member_by_member() {
    let mapping = json!({
        "a": [1, {"b": {"$get": ["input", "x"]}}, [{"$get": ["run", "id"]}]],
        "c": "text",
        "d": null,
        "e": true,
        "f": {"$literal": {"$get": ["input"]}},
        "g": {},
        "h": []
    });
    assert_eq!(
        evaluated(mapping),
        json!({
            "a": [1, {"b": 7}, [RUN_ID]],
            "c": "text",
            "d": null,
            "e": true,
            "f": {"$get": ["input"]},
            "g": {},
            "h": []
        })
    );
}

#[test]
fn an_unresolved_path_fails_the_run() {
    let paths = [
        json!(["input", "absent"]),
        json!(["input", "list", 3]),
        json!(["input", "list", "0"]),
        json!(["input", "obj", 0]),
        json!(["input", "x", "y"]),
        json!(["input", "nil", "x"]),
        json!(["input", "list", 9007199254740991u64]),
        json!(["nodes", "second"]),
        json!(["nodes", "done", "output"]),
        json!(["nodes", 0]),
        json!(["nodes", "first", "other"]),
        json!(["nodes", "first", 0]),
        json!(["nodes", "first", "output", "k", 2]),
        json!(["nodes", "first", "visit", "x"]),
        json!(["run", "other"]),
        json!(["run", 0]),
        json!(["run", "id", "x"]),
    ];
    for path in paths {
        assert_missing(json!({"wrapped": [{"$get": path}]}), path);
    }
}

/// Members are evaluated in canonical key order, so the first failure is stable.
#[test]
fn the_first_unresolved_path_in_canonical_order_is_reported() {
    let mapping = json!({
        "b": {"$get": ["input", "missing-b"]},
        "a": [{"$get": ["input", "x"]}, {"$get": ["input", "missing-a"]}],
        "\u{fb33}": {"$get": ["input", "missing-high"]},
        "\u{1f600}": {"$get": ["input", "missing-astral"]}
    });
    assert_missing(mapping, json!(["input", "missing-a"]));

    let mapping = json!({
        "\u{fb33}": {"$get": ["input", "missing-high"]},
        "\u{1f600}": {"$get": ["input", "missing-astral"]}
    });
    assert_missing(mapping, json!(["input", "missing-astral"]));
}

#[test]
fn an_unresolved_path_in_a_complete_node_fails_the_run() {
    let mut run = Run::started(&review_loop(), json!({"repo": "r"}));
    run.signal("approval", Some("approved"), json!(null), 1_500);
    let mut payload = run.result("success");
    set(&mut payload, "/output", json!({"only": "this"}));
    run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(run.status(), "completed");

    let workflow = with(
        review_loop(),
        "/nodes/done/output",
        json!({"$get": ["nodes", "approve", "output", "by"]}),
    );
    let mut run = Run::started(&workflow, json!({"repo": "r"}));
    run.signal("approval", Some("approved"), json!(null), 1_500);
    run.succeed("ok", json!(null), 2_000);
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(
        state["result"],
        json!({"error": {
            "code": code::MAPPING_MISSING_PATH,
            "message": "mapping path does not resolve",
            "nodeId": "done",
            "details": {"path": ["nodes", "approve", "output", "by"]}
        }})
    );
    assert_eq!(state["position"], json!(null));
    assert_eq!(
        state["visits"],
        json!({"build": 1, "approve": 1, "done": 1})
    );
}
