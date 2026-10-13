//! `spec/kernel.md` 3 and 7.4: loop limits, which only a 0.2 document has. The golden
//! traces `15*` pin the decisions; these tests pin K10 and the snapshot checks.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, event_kind, WORKFLOW_FORMAT_0_2};
use runspore_types::reducer::FailureKind::InvalidInput;
use serde_json::{json, Value};

/// `test` judges a loop back to `implement`: `red` at most `limit` times in a row, then
/// the `exhausted` route to a person at `stuck`, who sends the run back.
fn coding_loop(limit: u32) -> Value {
    json!({
        "format": WORKFLOW_FORMAT_0_2,
        "name": "coding-loop",
        "start": "implement",
        "limits": {"maxVisitsPerNode": 1000, "maxActivations": 10000},
        "actions": {
            "work": {"effect": "idempotent"},
            "test": {"effect": "read-only", "outcomes": ["ok", "red", "flaky"]}
        },
        "nodes": {
            "implement": {"kind": "activity", "action": "work", "outcomes": {"ok": "test"}},
            "test": {"kind": "activity", "action": "test", "loopLimits": {"red": limit},
                     "outcomes": {"ok": "done", "red": "implement", "flaky": "implement",
                                  "exhausted": "stuck"}},
            "stuck": {"kind": "await-signal", "signal": "stuck",
                      "outcomes": {"keep-going": "implement"}},
            "done": {"kind": "complete"}
        }
    })
}

fn position(run: &Run) -> String {
    run.state()["position"]["nodeId"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// K10: whatever `test` returns, `red` is never routed back to `implement` more than
/// the limit times in a row; the next `red` goes to `stuck`, and the count starts over
/// after any other outcome or after the exhausted route.
#[test]
fn a_limited_outcome_never_routes_more_than_its_limit_in_a_row() {
    for limit in 1..=4 {
        let mut run = Run::started(&coding_loop(limit), json!(null));
        let mut in_a_row = 0;
        let mut at = 2_000;
        // A fixed, irregular pattern of results; `flaky` breaks a streak of `red`.
        let pattern = "rrrrrrfrrrrfrrrrrrrrrfrr";
        for verdict in pattern.chars().cycle().take(120) {
            at += 10;
            if position(&run) == "stuck" {
                run.signal("stuck", Some("keep-going"), json!(null), at);
                continue;
            }
            run.succeed("ok", json!(null), at); // implement
            at += 10;
            let outcome = if verdict == 'r' { "red" } else { "flaky" };
            let decision = run.succeed(outcome, json!(null), at);
            let exhausted = codes(&decision) == [code::LOOP_EXHAUSTED];
            match (outcome, position(&run).as_str()) {
                ("red", "implement") => in_a_row += 1,
                ("red", "stuck") => {
                    assert!(exhausted, "only a used-up limit leads to stuck");
                    assert_eq!(in_a_row, limit, "the limit is used up exactly");
                    in_a_row = 0;
                }
                ("flaky", "implement") => in_a_row = 0,
                other => panic!("unexpected {other:?}"),
            }
            assert!(in_a_row <= limit, "K10: {in_a_row} > {limit}");
            let state = run.state();
            assert_eq!(
                state["nodes"]["test"]["outcome"], outcome,
                "the real result"
            );
            match state.get("streaks") {
                None => assert_eq!(in_a_row, 0),
                Some(streaks) => {
                    assert_eq!(
                        streaks["test"],
                        json!({"outcome": "red", "count": in_a_row})
                    );
                }
            }
        }
    }
}

/// A `failed` route counts like any routed outcome.
#[test]
fn a_failed_route_may_be_limited() {
    let mut workflow = coding_loop(1);
    set(
        &mut workflow,
        "/nodes/test/loopLimits",
        json!({"failed": 1}),
    );
    set(
        &mut workflow,
        "/nodes/test/outcomes/failed",
        json!("implement"),
    );
    let mut run = Run::started(&workflow, json!(null));
    run.succeed("ok", json!(null), 2_000);
    run.fail(false, 2_100);
    assert_eq!(position(&run), "implement");
    run.succeed("ok", json!(null), 2_200);
    let decision = run.fail(false, 2_300);
    assert_eq!(codes(&decision), [code::LOOP_EXHAUSTED]);
    assert_eq!(position(&run), "stuck");
    assert_eq!(run.state()["nodes"]["test"]["outcome"], "failed");
}

/// A snapshot whose streak §3 does not allow is not a state this kernel wrote.
#[test]
fn a_streak_the_loop_limits_do_not_allow_is_invalid() {
    let mut run = Run::started(&coding_loop(2), json!(null));
    run.succeed("ok", json!(null), 2_000);
    run.succeed("red", json!(null), 2_100);
    let good = run.state();
    assert_eq!(
        good["streaks"],
        json!({"test": {"outcome": "red", "count": 1}})
    );
    let next = run.result("success");

    let mut allowed = good.clone();
    set(&mut allowed, "/streaks/test/count", json!(2));
    let mut request = run.next(event_kind::ACTIVITY_RESULT, &next, 2_200);
    request.snapshot = Some(canon(&allowed));
    assert!(transition(&request).is_ok());

    for streak in [
        json!({"implement": {"outcome": "ok", "count": 1}}),
        json!({"test": {"outcome": "flaky", "count": 1}}),
        json!({"test": {"outcome": "red", "count": 0}}),
        json!({"test": {"outcome": "red", "count": 3}}),
        json!({"test": {"outcome": "red", "count": 1, "extra": 1}}),
    ] {
        let mut state = good.clone();
        set(&mut state, "/streaks", streak.clone());
        let mut request = run.next(event_kind::ACTIVITY_RESULT, &next, 2_200);
        request.snapshot = Some(canon(&state));
        let result = transition(&request);
        assert!(result.is_err(), "{streak} was accepted");
        assert_failure(result, InvalidInput, code::STATE_INVALID);
    }
}
