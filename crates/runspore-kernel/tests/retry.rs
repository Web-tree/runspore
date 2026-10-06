//! `spec/kernel.md` 7.3–7.4: delivery retries, unknown outcomes, operator
//! resolutions, and stale or duplicate results.

mod common;

use common::*;
use runspore_types::digest::ids;
use runspore_types::model::{code, command_kind, event_kind};
use runspore_types::reducer::Decision;
use serde_json::{json, Value};

fn backoff() -> Value {
    json!({"maxAttempts": 3, "backoffMs": "500", "maxBackoffMs": "800"})
}

fn io_error(node: &str) -> Value {
    json!({"code": "io.broken", "message": "boom", "nodeId": node, "details": null})
}

fn retry_command(decision: &Decision) -> Value {
    assert_eq!(decision.commands.len(), 1);
    let command = &decision.commands[0];
    assert_eq!(command.kind, command_kind::ACTIVITY_RETRY);
    parse(&command.payload)
}

fn retry_diagnostic(attempt: u32) -> Value {
    json!([{"code": code::ACTIVITY_RETRY_SCHEDULED, "nodeId": "work",
            "details": {"attempt": attempt}}])
}

/// A run whose only invocation is `unknown` on a non-repeatable action.
fn needs_intervention(retry: Value, failed_route: bool) -> Run {
    let mut run = Run::started(&single_activity("unsafe", retry, failed_route), json!(null));
    run.lose("unknown", 2_000);
    assert_eq!(run.status(), "needs-intervention");
    run
}

/// K4: a retry keeps the invocation and changes the command.
#[test]
fn a_retryable_failure_is_retried_with_backoff() {
    let mut run = Run::new(&single_activity("idempotent", backoff(), false));
    let schedule = run.start(json!(null));
    let invocation = run.invocation_id();
    assert_eq!(invocation, ids::invocation(TENANT, RUN_ID, "work/1"));

    let decision = run.fail(true, 2_000);
    assert_eq!(
        retry_command(&decision),
        json!({"invocationId": invocation, "attempt": 2, "notBeforeMs": "2500"})
    );
    let command = &decision.commands[0];
    assert_eq!(
        command.command_id,
        ids::command(TENANT, RUN_ID, "work/1", 2)
    );
    assert_eq!(command.activation_id, "work/1");
    assert_ne!(command.command_id, schedule.commands[0].command_id);
    assert_eq!(diagnostics(&decision), retry_diagnostic(2));
    let state = run.state();
    assert_eq!(state["status"], json!("running"));
    assert_eq!(state["invocation"]["invocationId"], json!(invocation));
    assert_eq!(state["invocation"]["attempt"], json!(2));
    assert_eq!(state["invocation"]["state"], json!("scheduled"));
    assert_eq!(state["visits"], json!({"work": 1}));
    assert_eq!(state["activations"], json!(1));
    assert_eq!(state["nodes"], json!({}));

    let decision = run.fail(true, 3_000);
    assert_eq!(
        retry_command(&decision),
        json!({"invocationId": invocation, "attempt": 3, "notBeforeMs": "3800"})
    );
    assert_eq!(
        decision.commands[0].command_id,
        ids::command(TENANT, RUN_ID, "work/1", 3)
    );
    assert_eq!(diagnostics(&decision), retry_diagnostic(3));

    let decision = run.succeed("ok", json!("third time"), 4_000);
    assert!(decision.commands.is_empty());
    assert_eq!(run.status(), "completed");
    assert_eq!(
        run.state()["result"],
        json!({"output": {"visit": 1, "outcome": "ok", "output": "third time"}})
    );
}

#[test]
fn backoff_starts_from_logical_now() {
    let mut run = Run::started(
        &single_activity("idempotent", backoff(), false),
        json!(null),
    );
    run.signal("noise", None, json!(null), 9_000);
    let decision = run.fail(true, 2_000);
    assert_eq!(retry_command(&decision)["notBeforeMs"], json!("9500"));
}

#[test]
fn exhausted_retries_take_the_failed_route() {
    let mut run = Run::started(&single_activity("idempotent", backoff(), true), json!(null));
    run.fail(true, 2_000);
    run.fail(true, 3_000);
    let decision = run.fail(true, 4_000);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(state["invocation"], json!(null));
    let recorded = json!({"visit": 1, "outcome": "failed", "output": {"error": io_error("work")}});
    assert_eq!(state["nodes"]["work"], recorded);
    assert_eq!(state["result"], json!({"output": recorded}));
    assert_eq!(state["visits"], json!({"work": 1, "recover": 1}));
}

#[test]
fn exhausted_retries_without_a_failed_route_end_the_run() {
    let mut run = Run::started(
        &single_activity("idempotent", backoff(), false),
        json!(null),
    );
    run.fail(true, 2_000);
    run.fail(true, 3_000);
    let decision = run.fail(true, 4_000);
    assert!(decision.commands.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["position"], json!(null));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(state["nodes"], json!({}));
    assert_eq!(state["result"], json!({"error": io_error("work")}));
}

#[test]
fn a_non_retryable_failure_skips_the_retry_budget() {
    let mut run = Run::started(&single_activity("idempotent", backoff(), true), json!(null));
    let decision = run.fail(false, 2_000);
    assert!(decision.commands.is_empty());
    assert_eq!(
        run.state()["nodes"]["work"]["output"],
        json!({"error": io_error("work")})
    );

    let mut run = Run::started(
        &single_activity("idempotent", backoff(), false),
        json!(null),
    );
    run.fail(false, 2_000);
    assert_eq!(run.state()["status"], json!("failed"));
    assert_eq!(run.state()["result"], json!({"error": io_error("work")}));
}

#[test]
fn one_attempt_is_the_default_budget() {
    let mut run = Run::started(
        &single_activity("idempotent", json!({}), false),
        json!(null),
    );
    let decision = run.fail(true, 2_000);
    assert!(decision.commands.is_empty());
    assert_eq!(run.status(), "failed");
}

#[test]
fn a_failure_without_an_error_is_activity_failed() {
    let mut run = Run::started(&single_activity("pure", json!({}), false), json!(null));
    let payload = run.result("failure");
    run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(
        run.state()["result"],
        json!({"error": {"code": code::ACTIVITY_FAILED, "message": "activity failed",
                         "nodeId": "work", "details": null}})
    );
}

#[test]
fn the_failing_node_replaces_the_reported_node_id() {
    let mut run = Run::started(&single_activity("pure", json!({}), false), json!(null));
    let mut payload = run.result("failure");
    set(
        &mut payload,
        "/error",
        json!({"code": "x.y", "message": "m", "nodeId": "elsewhere", "details": {"k": 1}}),
    );
    run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(
        run.state()["result"],
        json!({"error": {"code": "x.y", "message": "m", "nodeId": "work",
                         "details": {"k": 1}}})
    );
}

#[test]
fn an_unknown_or_expired_attempt_of_a_repeatable_action_is_retried() {
    for effect in ["pure", "read-only", "idempotent"] {
        for status in ["unknown", "expired"] {
            let mut run = Run::started(&single_activity(effect, backoff(), false), json!(null));
            let invocation = run.invocation_id();
            let decision = run.lose(status, 2_000);
            assert_eq!(
                retry_command(&decision),
                json!({"invocationId": invocation, "attempt": 2, "notBeforeMs": "2500"})
            );
            assert_eq!(diagnostics(&decision), retry_diagnostic(2));
            assert_eq!(run.status(), "running");
        }
    }
}

#[test]
fn a_repeatable_action_out_of_attempts_is_exhausted() {
    for status in ["unknown", "expired"] {
        let mut run = Run::started(
            &single_activity("idempotent", backoff(), false),
            json!(null),
        );
        run.lose(status, 2_000);
        run.lose(status, 3_000);
        let decision = run.lose(status, 4_000);
        assert!(decision.commands.is_empty());
        assert!(decision.diagnostics.is_empty());
        let state = run.state();
        assert_eq!(state["status"], json!("failed"));
        assert_eq!(
            state["result"],
            json!({"error": {
                "code": code::ACTIVITY_ATTEMPTS_EXHAUSTED,
                "message": "activity attempts exhausted",
                "nodeId": "work",
                "details": {"lastStatus": status}
            }})
        );
    }
}

#[test]
fn exhaustion_takes_the_failed_route_when_there_is_one() {
    let mut run = Run::started(&single_activity("pure", json!({}), true), json!(null));
    run.lose("expired", 2_000);
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["nodes"]["work"]["output"]["error"]["code"],
        json!(code::ACTIVITY_ATTEMPTS_EXHAUSTED)
    );
}

/// K6: an unknown outcome on a non-repeatable action never produces a command.
#[test]
fn an_unknown_or_expired_attempt_of_an_unsafe_action_needs_intervention() {
    for status in ["unknown", "expired"] {
        let mut run = Run::started(&single_activity("unsafe", backoff(), true), json!(null));
        let invocation = run.invocation_id();
        let before = run.state();
        let decision = run.lose(status, 2_000);
        assert!(decision.commands.is_empty());
        assert_eq!(
            diagnostics(&decision),
            json!([{"code": code::INVOCATION_UNKNOWN, "nodeId": "work",
                    "details": {"invocationId": invocation, "attempt": 1, "status": status}}])
        );
        let mut expected = before;
        set(&mut expected, "/status", json!("needs-intervention"));
        set(&mut expected, "/invocation/state", json!("unknown"));
        set(&mut expected, "/lastSequence", json!("2"));
        set(&mut expected, "/lastAcceptedAtMs", json!("2000"));
        assert_eq!(run.state(), expected);
    }
}

#[test]
fn a_failure_of_an_unsafe_action_is_still_retried() {
    let mut run = Run::started(&single_activity("unsafe", backoff(), false), json!(null));
    let decision = run.fail(true, 2_000);
    assert_eq!(retry_command(&decision)["attempt"], json!(2));
    assert_eq!(run.status(), "running");
}

#[test]
fn a_result_while_intervention_is_needed_is_stale() {
    let mut run = needs_intervention(backoff(), false);
    let invocation = run.invocation_id();
    let before = run.state();
    let decision = run.step(
        event_kind::ACTIVITY_RESULT,
        &result_for(&invocation, 1, "success"),
        3_000,
    );
    assert!(decision.commands.is_empty());
    assert_eq!(
        diagnostics(&decision),
        json!([{"code": code::RESULT_STALE, "nodeId": "work",
                "details": {"invocationId": invocation, "attempt": 1}}])
    );
    let mut expected = before;
    set(&mut expected, "/lastSequence", json!("3"));
    set(&mut expected, "/lastAcceptedAtMs", json!("3000"));
    assert_eq!(run.state(), expected);
}

#[test]
fn resolving_complete_continues_as_a_success() {
    let mut run = needs_intervention(json!({}), true);
    let invocation = run.invocation_id();
    let decision = run.resolve(
        &invocation,
        json!({"action": "complete", "outcome": "ok", "output": {"by": "operator"}}),
        3_000,
    );
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["nodes"]["work"],
        json!({"visit": 1, "outcome": "ok", "output": {"by": "operator"}})
    );
    assert_eq!(state["visits"], json!({"work": 1, "done": 1}));
}

#[test]
fn resolving_complete_with_an_undeclared_outcome_changes_nothing() {
    let mut run = needs_intervention(json!({}), true);
    let invocation = run.invocation_id();
    let before = run.state();
    for outcome in ["blue", "failed"] {
        let decision = run.resolve(
            &invocation,
            json!({"action": "complete", "outcome": outcome}),
            3_000,
        );
        assert!(decision.commands.is_empty());
        assert_eq!(
            diagnostics(&decision),
            json!([{"code": code::RESOLVE_NOT_APPLICABLE, "nodeId": "work",
                    "details": {"invocationId": invocation, "reason": "undeclared-outcome"}}])
        );
        assert_eq!(run.status(), "needs-intervention");
        assert_eq!(run.state()["invocation"], before["invocation"]);
        assert_eq!(run.state()["nodes"], before["nodes"]);
    }
}

#[test]
fn resolving_retry_authorizes_a_new_attempt_without_delay() {
    let mut run = needs_intervention(backoff(), false);
    let invocation = run.invocation_id();
    let decision = run.resolve(&invocation, json!({"action": "retry"}), 3_000);
    assert_eq!(
        retry_command(&decision),
        json!({"invocationId": invocation, "attempt": 2, "notBeforeMs": "3000"})
    );
    assert_eq!(
        decision.commands[0].command_id,
        ids::command(TENANT, RUN_ID, "work/1", 2)
    );
    assert_eq!(decision.commands[0].activation_id, "work/1");
    assert_eq!(diagnostics(&decision), retry_diagnostic(2));
    let state = run.state();
    assert_eq!(state["status"], json!("running"));
    assert_eq!(state["invocation"]["state"], json!("scheduled"));
    assert_eq!(state["invocation"]["attempt"], json!(2));
}

#[test]
fn resolving_retry_ignores_max_attempts() {
    let mut run = needs_intervention(json!({"maxAttempts": 1}), false);
    for attempt in 2..=4u32 {
        let invocation = run.invocation_id();
        let at = 1_000 * u64::from(attempt);
        let decision = run.resolve(&invocation, json!({"action": "retry"}), at);
        assert_eq!(retry_command(&decision)["attempt"], json!(attempt));
        assert_eq!(run.attempt(), attempt);
        run.lose("expired", at + 500);
        assert_eq!(run.status(), "needs-intervention");
    }
}

#[test]
fn resolving_fail_fails_the_invocation() {
    let error = json!({"code": code::OPERATOR_FAILED, "message": "gave up",
                       "nodeId": "elsewhere", "details": {"ticket": 7}});
    let stored = json!({"code": code::OPERATOR_FAILED, "message": "gave up",
                        "nodeId": "work", "details": {"ticket": 7}});

    let mut run = needs_intervention(json!({}), true);
    let invocation = run.invocation_id();
    let decision = run.resolve(
        &invocation,
        json!({"action": "fail", "error": error}),
        3_000,
    );
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["nodes"]["work"],
        json!({"visit": 1, "outcome": "failed", "output": {"error": stored}})
    );

    let mut run = needs_intervention(json!({}), false);
    let invocation = run.invocation_id();
    run.resolve(
        &invocation,
        json!({"action": "fail", "error": error}),
        3_000,
    );
    let state = run.state();
    assert_eq!(state["status"], json!("failed"));
    assert_eq!(state["invocation"], json!(null));
    assert_eq!(state["result"], json!({"error": stored}));
}

#[test]
fn a_resolution_needs_a_current_unknown_invocation() {
    let resolutions = [
        json!({"action": "retry"}),
        json!({"action": "complete", "outcome": "ok"}),
        json!({"action": "fail", "error": {"code": "x", "message": "m"}}),
    ];

    let mut running = Run::started(&single_activity("unsafe", json!({}), false), json!(null));
    let current = running.invocation_id();
    let mut intervention = needs_intervention(json!({}), false);
    let mut waiting = Run::started(&review_loop(), json!({"repo": "r"}));
    waiting.succeed("ok", json!(null), 2_000);
    assert_eq!(waiting.status(), "waiting");

    for resolution in resolutions {
        let cases: [(&mut Run, &str, Value, &str); 3] = [
            (&mut running, current.as_str(), json!("work"), "not-unknown"),
            (&mut intervention, "inv_other", json!(null), "not-current"),
            (&mut waiting, current.as_str(), json!(null), "not-current"),
        ];
        for (run, invocation, node, reason) in cases {
            let before = run.state();
            let sequence = run.sequence + 1;
            let decision = run.resolve(invocation, resolution.clone(), 5_000);
            assert!(decision.commands.is_empty());
            assert_eq!(
                diagnostics(&decision),
                json!([{"code": code::RESOLVE_NOT_APPLICABLE, "nodeId": node,
                        "details": {"invocationId": invocation, "reason": reason}}])
            );
            let mut expected = before;
            set(&mut expected, "/lastSequence", json!(sequence.to_string()));
            set(&mut expected, "/lastAcceptedAtMs", json!("5000"));
            assert_eq!(run.state(), expected);
        }
    }
}

/// K2: a result is applied at most once.
#[test]
fn a_duplicate_result_is_stale() {
    let mut run = Run::started(&review_loop(), json!({"repo": "r"}));
    let first = run.invocation_id();
    run.succeed("red", json!(1), 2_000);
    let before = run.state();

    let mut duplicate = result_for(&first, 1, "success");
    set(&mut duplicate, "/outcome", json!("ok"));
    for (index, status) in ["success", "failure", "unknown", "expired"]
        .iter()
        .enumerate()
    {
        set(&mut duplicate, "/status", json!(status));
        let decision = run.step(event_kind::ACTIVITY_RESULT, &duplicate, 2_500);
        assert!(decision.commands.is_empty());
        assert_eq!(
            diagnostics(&decision),
            json!([{"code": code::RESULT_STALE, "nodeId": null,
                    "details": {"invocationId": first, "attempt": 1}}])
        );
        let mut expected = before.clone();
        set(
            &mut expected,
            "/lastSequence",
            json!((3 + index).to_string()),
        );
        set(&mut expected, "/lastAcceptedAtMs", json!("2500"));
        assert_eq!(run.state(), expected);
    }
}

#[test]
fn a_result_for_another_attempt_is_stale() {
    let mut run = Run::started(
        &single_activity("idempotent", backoff(), false),
        json!(null),
    );
    let invocation = run.invocation_id();
    run.fail(true, 2_000);
    assert_eq!(run.attempt(), 2);
    let before = run.state();

    for (index, attempt) in [1u32, 3, 0].iter().enumerate() {
        let decision = run.step(
            event_kind::ACTIVITY_RESULT,
            &result_for(&invocation, *attempt, "success"),
            2_500,
        );
        assert!(decision.commands.is_empty());
        assert_eq!(
            diagnostics(&decision),
            json!([{"code": code::RESULT_STALE, "nodeId": "work",
                    "details": {"invocationId": invocation, "attempt": attempt}}])
        );
        let mut expected = before.clone();
        set(
            &mut expected,
            "/lastSequence",
            json!((3 + index).to_string()),
        );
        set(&mut expected, "/lastAcceptedAtMs", json!("2500"));
        assert_eq!(run.state(), expected);
    }

    run.succeed("ok", json!("late but current"), 3_000);
    assert_eq!(run.status(), "completed");
}

#[test]
fn a_result_without_an_outstanding_invocation_is_stale() {
    let mut run = Run::started(&review_loop(), json!({"repo": "r"}));
    let invocation = run.invocation_id();
    run.succeed("ok", json!(null), 2_000);
    assert_eq!(run.status(), "waiting");
    let decision = run.step(
        event_kind::ACTIVITY_RESULT,
        &result_for(&invocation, 1, "success"),
        3_000,
    );
    assert_eq!(
        diagnostics(&decision),
        json!([{"code": code::RESULT_STALE, "nodeId": null,
                "details": {"invocationId": invocation, "attempt": 1}}])
    );
    assert_eq!(run.status(), "waiting");
}

#[test]
fn a_signal_while_running_is_buffered_and_nothing_else_changes() {
    let mut run = Run::started(&single_activity("pure", json!({}), false), json!(null));
    let before = run.state();
    let decision = run.signal("approval", Some("approved"), json!({"by": "ana"}), 2_000);
    assert!(decision.commands.is_empty());
    assert!(decision.diagnostics.is_empty());
    let mut expected = before;
    set(
        &mut expected,
        "/signals",
        json!([{"eventId": "e2", "sequence": "2", "name": "approval",
                "outcome": "approved", "data": {"by": "ana"}}]),
    );
    set(&mut expected, "/lastSequence", json!("2"));
    set(&mut expected, "/lastAcceptedAtMs", json!("2000"));
    assert_eq!(run.state(), expected);
}

/// Retrying past the attempt counter's range is a failure, never a wrap.
#[test]
fn an_attempt_counter_at_its_limit_cannot_be_retried() {
    let run = needs_intervention(json!({}), false);
    let invocation = run.invocation_id();
    let mut state = run.state();
    set(&mut state, "/invocation/attempt", json!(u32::MAX));
    let mut request = run.next(
        event_kind::INVOCATION_RESOLVED,
        &json!({"invocationId": invocation, "resolution": {"action": "retry"}}),
        3_000,
    );
    request.snapshot = Some(canon(&state));
    assert_failure(
        runspore_kernel::transition(&request),
        runspore_types::reducer::FailureKind::ResourceLimit,
        "budget.counter-range",
    );
}

/// A retry time beyond 2^63 − 1 is a failure, never a wrap.
#[test]
fn a_retry_time_beyond_the_counter_domain_is_a_failure() {
    let run = Run::started(
        &single_activity("idempotent", backoff(), false),
        json!(null),
    );
    let mut payload = run.result("failure");
    set(&mut payload, "/retryable", json!(true));
    let request = run.next(event_kind::ACTIVITY_RESULT, &payload, (1 << 63) - 1);
    assert_failure(
        runspore_kernel::transition(&request),
        runspore_types::reducer::FailureKind::ResourceLimit,
        "budget.counter-range",
    );
}
