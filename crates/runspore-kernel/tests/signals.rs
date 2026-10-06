//! `spec/kernel.md` 7.3–7.4: the signal buffer and the Consume procedure.

mod common;

use common::*;
use runspore_types::model::{code, command_kind, MAX_BUFFERED_SIGNALS, WORKFLOW_FORMAT};
use serde_json::{json, Value};

/// `prep` runs first; then two waiters on the same signal name, then `after`.
fn gated() -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "gated",
        "start": "prep",
        "actions": {"act": {"effect": "pure"}},
        "nodes": {
            "prep": {"kind": "activity", "action": "act", "outcomes": {"ok": "gate1"}},
            "gate1": {"kind": "await-signal", "signal": "go",
                      "outcomes": {"a": "gate2", "b": "gate2", "received": "gate2"}},
            "gate2": {"kind": "await-signal", "signal": "go",
                      "outcomes": {"a": "after", "b": "after", "back": "gate1"}},
            "after": {"kind": "activity", "action": "act",
                      "input": {"first": {"$get": ["nodes", "gate1"]},
                                "second": {"$get": ["nodes", "gate2"]}},
                      "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn buffered(run: &Run) -> Vec<String> {
    run.state()["signals"]
        .as_array()
        .expect("signals is an array")
        .iter()
        .map(|signal| signal["eventId"].as_str().expect("event id").to_string())
        .collect()
}

/// K5 (C08): signals accepted before their waiter are consumed once, earliest first.
#[test]
fn buffered_signals_are_consumed_once_earliest_first() {
    let mut run = Run::started(&gated(), json!(null));
    run.signal("go", Some("a"), json!(1), 1_100);
    run.signal("other", Some("a"), json!("unrelated"), 1_200);
    run.signal("go", Some("b"), json!(2), 1_300);
    run.signal("go", Some("a"), json!(3), 1_400);
    assert_eq!(buffered(&run), ["e2", "e3", "e4", "e5"]);
    assert_eq!(run.status(), "running");

    let decision = run.succeed("ok", json!(null), 2_000);
    assert!(decision.diagnostics.is_empty());
    assert_eq!(decision.commands.len(), 1);
    let command = &decision.commands[0];
    assert_eq!(command.kind, command_kind::ACTIVITY_SCHEDULE);
    assert_eq!(command.activation_id, "after/1");
    assert_eq!(
        parse(&command.payload)["input"],
        json!({
            "first": {"visit": 1, "outcome": "a", "output": 1},
            "second": {"visit": 1, "outcome": "b", "output": 2}
        })
    );
    assert_eq!(buffered(&run), ["e3", "e5"]);
    let state = run.state();
    assert_eq!(state["status"], json!("running"));
    assert_eq!(
        state["visits"],
        json!({"prep": 1, "gate1": 1, "gate2": 1, "after": 1})
    );
    assert_eq!(state["activations"], json!(4));
}

#[test]
fn a_waiter_without_a_matching_signal_parks() {
    let mut run = Run::started(&gated(), json!(null));
    run.signal("other", None, json!(null), 1_100);
    let decision = run.succeed("ok", json!(null), 2_000);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("waiting"));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(
        state["position"],
        json!({"nodeId": "gate1", "activationId": "gate1/1", "visit": 1})
    );
    assert_eq!(buffered(&run), ["e2"]);
}

#[test]
fn a_signal_for_the_waiter_is_consumed_on_arrival() {
    let mut run = Run::started(&gated(), json!(null));
    run.succeed("ok", json!(null), 2_000);

    let decision = run.signal("other", Some("a"), json!(null), 2_100);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    assert_eq!(run.status(), "waiting");
    assert_eq!(buffered(&run), ["e3"]);

    let decision = run.signal("go", Some("b"), json!({"n": 1}), 2_200);
    assert!(decision.commands.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("waiting"));
    assert_eq!(
        state["position"],
        json!({"nodeId": "gate2", "activationId": "gate2/1", "visit": 1})
    );
    assert_eq!(
        state["nodes"]["gate1"],
        json!({"visit": 1, "outcome": "b", "output": {"n": 1}})
    );
    assert_eq!(buffered(&run), ["e3"]);

    let decision = run.signal("go", Some("a"), json!({"n": 2}), 2_300);
    assert_eq!(decision.commands.len(), 1);
    assert_eq!(run.status(), "running");
    assert_eq!(
        parse(&decision.commands[0].payload)["notBeforeMs"],
        json!("2300")
    );
    assert_eq!(buffered(&run), ["e3"]);
}

#[test]
fn a_signal_without_an_outcome_is_received() {
    let mut run = Run::started(&gated(), json!(null));
    run.succeed("ok", json!(null), 2_000);
    run.signal("go", None, json!("payload"), 2_100);
    let state = run.state();
    assert_eq!(
        state["nodes"]["gate1"],
        json!({"visit": 1, "outcome": "received", "output": "payload"})
    );
    assert_eq!(state["position"]["nodeId"], json!("gate2"));
}

#[test]
fn an_unrouted_outcome_is_dropped_and_the_next_signal_is_tried() {
    let mut run = Run::started(&gated(), json!(null));
    run.signal("go", Some("zzz"), json!(1), 1_100);
    run.signal("go", None, json!(2), 1_200);
    run.signal("go", Some("received"), json!(3), 1_300);
    run.signal("go", Some("a"), json!(4), 1_400);

    let decision = run.succeed("ok", json!(null), 2_000);
    assert_eq!(
        diagnostics(&decision),
        json!([
            {"code": code::SIGNAL_OUTCOME_UNROUTED, "nodeId": "gate1",
             "details": {"eventId": "e2", "outcome": "zzz"}},
            {"code": code::SIGNAL_OUTCOME_UNROUTED, "nodeId": "gate2",
             "details": {"eventId": "e4", "outcome": "received"}}
        ])
    );
    assert_eq!(decision.commands.len(), 1);
    assert_eq!(
        parse(&decision.commands[0].payload)["input"],
        json!({
            "first": {"visit": 1, "outcome": "received", "output": 2},
            "second": {"visit": 1, "outcome": "a", "output": 4}
        })
    );
    assert!(buffered(&run).is_empty());
}

#[test]
fn an_unrouted_signal_on_arrival_leaves_the_waiter_parked() {
    let mut run = Run::started(&gated(), json!(null));
    run.succeed("ok", json!(null), 2_000);
    let before = run.state();
    let decision = run.signal("go", Some("zzz"), json!(1), 2_100);
    assert!(decision.commands.is_empty());
    assert_eq!(
        diagnostics(&decision),
        json!([{"code": code::SIGNAL_OUTCOME_UNROUTED, "nodeId": "gate1",
                "details": {"eventId": "e3", "outcome": "zzz"}}])
    );
    let mut expected = before;
    set(&mut expected, "/lastSequence", json!("3"));
    set(&mut expected, "/lastAcceptedAtMs", json!("2100"));
    assert_eq!(run.state(), expected);
}

#[test]
fn a_waiter_may_be_visited_again_through_a_back_edge() {
    let mut run = Run::started(&gated(), json!(null));
    run.succeed("ok", json!(null), 2_000);
    run.signal("go", Some("a"), json!(1), 2_100);
    run.signal("go", Some("back"), json!(2), 2_200);
    let state = run.state();
    assert_eq!(
        state["position"],
        json!({"nodeId": "gate1", "activationId": "gate1/2", "visit": 2})
    );
    assert_eq!(
        state["nodes"]["gate2"],
        json!({"visit": 1, "outcome": "back", "output": 2})
    );
    run.signal("go", Some("b"), json!(3), 2_300);
    assert_eq!(
        run.state()["nodes"]["gate1"],
        json!({"visit": 2, "outcome": "b", "output": 3})
    );
    assert_eq!(run.state()["position"]["activationId"], json!("gate2/2"));
}

#[test]
fn the_buffer_holds_sixty_four_signals() {
    let mut run = Run::started(&gated(), json!(null));
    for index in 0..MAX_BUFFERED_SIGNALS {
        let decision = run.signal("other", None, json!(index), 1_100);
        assert!(decision.diagnostics.is_empty());
    }
    assert_eq!(buffered(&run).len(), 64);
    let before = run.state();

    let decision = run.signal("go", Some("a"), json!("dropped"), 1_200);
    assert!(decision.commands.is_empty());
    assert_eq!(
        diagnostics(&decision),
        json!([{"code": code::SIGNAL_BUFFER_OVERFLOW, "nodeId": null,
                "details": {"eventId": "e66"}}])
    );
    let mut expected = before;
    set(&mut expected, "/lastSequence", json!("66"));
    set(&mut expected, "/lastAcceptedAtMs", json!("1200"));
    assert_eq!(run.state(), expected);
}

/// The overflow rule comes before consumption, so a full buffer drops even a
/// signal the parked waiter would have taken.
#[test]
fn a_full_buffer_drops_a_signal_the_waiter_wants() {
    let mut run = Run::started(&gated(), json!(null));
    for index in 0..MAX_BUFFERED_SIGNALS {
        run.signal("other", None, json!(index), 1_100);
    }
    run.succeed("ok", json!(null), 2_000);
    assert_eq!(run.status(), "waiting");

    let decision = run.signal("go", Some("a"), json!("dropped"), 2_100);
    assert_eq!(
        diagnostics(&decision),
        json!([{"code": code::SIGNAL_BUFFER_OVERFLOW, "nodeId": null,
                "details": {"eventId": "e67"}}])
    );
    assert_eq!(run.status(), "waiting");
    assert_eq!(run.state()["position"]["nodeId"], json!("gate1"));
    assert_eq!(buffered(&run).len(), 64);
}

#[test]
fn consumed_signals_free_buffer_space() {
    let mut run = Run::started(&gated(), json!(null));
    for _ in 0..MAX_BUFFERED_SIGNALS {
        run.signal("go", Some("zzz"), json!(null), 1_100);
    }
    let decision = run.succeed("ok", json!(null), 2_000);
    assert_eq!(decision.diagnostics.len(), 64);
    assert!(buffered(&run).is_empty());
    assert_eq!(run.status(), "waiting");

    let decision = run.signal("go", Some("a"), json!(null), 2_100);
    assert!(decision.diagnostics.is_empty());
    assert_eq!(run.state()["position"]["nodeId"], json!("gate2"));
}
