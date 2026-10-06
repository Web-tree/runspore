//! `spec/kernel.md` 7.4–7.5 and K7: budgets. Exceeding one returns a failure and
//! nothing else, so the prior snapshot stays the run's state.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, event_kind, WORKFLOW_FORMAT};
use runspore_types::reducer::FailureKind::ResourceLimit;
use runspore_types::reducer::Limits;
use serde_json::{json, Value};

/// `prep`, then two waiters, then `after`: five microsteps when both signals are buffered.
fn chain() -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "chain",
        "start": "prep",
        "actions": {"act": {"effect": "pure"}},
        "nodes": {
            "prep": {"kind": "activity", "action": "act", "outcomes": {"ok": "gate1"}},
            "gate1": {"kind": "await-signal", "signal": "go", "outcomes": {"a": "gate2"}},
            "gate2": {"kind": "await-signal", "signal": "go", "outcomes": {"a": "after"}},
            "after": {"kind": "activity", "action": "act", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

/// A run about to enter `gate1`, `gate2` and `after` in one transition.
fn primed() -> Run {
    let mut run = Run::started(&chain(), json!(null));
    run.signal("go", Some("a"), json!(1), 1_100);
    run.signal("go", Some("a"), json!(2), 1_200);
    run
}

/// One activity whose input is `mapping`.
fn mapping_workflow(mapping: Value) -> Value {
    with(
        single_activity("pure", json!({}), false),
        "/nodes/work/input",
        mapping,
    )
}

fn limits(edit: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    edit(&mut limits);
    limits
}

#[test]
fn the_default_budget_is_the_frozen_one() {
    assert_eq!(
        Limits::default(),
        Limits {
            microsteps: 1_000,
            expression_operations: 10_000,
            max_state_bytes: 256 * 1024,
            max_command_count: 128,
        }
    );
}

/// K7 (C16): a segment over the microstep budget commits nothing.
#[test]
fn microsteps_count_entries_and_consumed_signals() {
    let mut run = primed();
    let before = run.snapshot.clone();
    let payload = run.result("success");

    run.limits = limits(|l| l.microsteps = 4);
    let result = run.apply(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_failure(result, ResourceLimit, code::BUDGET_MICROSTEPS);
    assert_eq!(run.snapshot, before);
    assert_eq!(run.sequence, 3);

    run.limits = limits(|l| l.microsteps = 5);
    let decision = run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(decision.commands.len(), 1);
    assert_eq!(decision.commands[0].activation_id, "after/1");

    let mut reference = primed();
    let expected = reference.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(decision, expected);
}

#[test]
fn a_dropped_signal_costs_a_microstep_too() {
    let mut run = Run::started(&chain(), json!(null));
    run.signal("go", Some("zzz"), json!(1), 1_100);
    run.signal("go", Some("zzz"), json!(2), 1_200);
    let payload = run.result("success");

    run.limits = limits(|l| l.microsteps = 2);
    let result = run.apply(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_failure(result, ResourceLimit, code::BUDGET_MICROSTEPS);

    run.limits = limits(|l| l.microsteps = 3);
    let decision = run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(decision.diagnostics.len(), 2);
    assert_eq!(run.status(), "waiting");
}

#[test]
fn entering_the_start_node_costs_one_microstep() {
    let mut request = start_request(&chain(), json!(null));
    request.frozen_limits.microsteps = 0;
    assert_failure(transition(&request), ResourceLimit, code::BUDGET_MICROSTEPS);
    request.frozen_limits.microsteps = 1;
    assert!(transition(&request).is_ok());
}

#[test]
fn events_that_enter_nothing_cost_no_microsteps() {
    let mut run = Run::started(&chain(), json!(null));
    run.limits = limits(|l| l.microsteps = 0);
    run.signal("go", Some("a"), json!(1), 1_100);
    let stale = result_for("inv_other", 1, "success");
    let decision = run.step(event_kind::ACTIVITY_RESULT, &stale, 1_200);
    assert_eq!(codes(&decision), [code::RESULT_STALE]);
}

#[test]
fn a_snapshot_over_the_state_budget_is_a_failure() {
    let workflow = chain();
    let input = json!({"blob": "x".repeat(4_000)});
    let size = transition(&start_request(&workflow, input.clone()))
        .expect("fits the default budget")
        .snapshot
        .len();
    assert!(size > 4_000);

    let mut request = start_request(&workflow, input);
    request.frozen_limits.max_state_bytes = u32::try_from(size).unwrap() - 1;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_STATE_BYTES,
    );
    request.frozen_limits.max_state_bytes = u32::try_from(size).unwrap();
    assert!(transition(&request).is_ok());
}

#[test]
fn a_growing_state_fails_without_changing_the_run() {
    let mut run = Run::started(&chain(), json!(null));
    let size = run.snapshot.as_ref().unwrap().len();
    run.limits = limits(|l| l.max_state_bytes = u32::try_from(size).unwrap() + 120);
    let before = run.snapshot.clone();

    let big = json!({"name": "go", "data": "x".repeat(200)});
    let result = run.apply(event_kind::SIGNAL_RECEIVED, &big, 1_100);
    assert_failure(result, ResourceLimit, code::BUDGET_STATE_BYTES);
    assert_eq!(run.snapshot, before);

    let decision = run.signal("other", None, json!(null), 1_200);
    assert_eq!(parse(&decision.snapshot)["lastSequence"], json!("2"));
}

/// Each mapping value visited costs one operation; each path step costs one.
#[test]
fn expression_operations_count_values_and_path_steps() {
    let mapping = json!({
        "a": 1,
        "b": [true, {"$get": ["input", "x"]}],
        "c": {"$literal": {"deep": [1, 2, 3], "$get": ["not", "evaluated"]}}
    });
    let workflow = mapping_workflow(mapping);
    let mut request = start_request(&workflow, json!({"x": 7}));

    request.frozen_limits.expression_operations = 7;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_EXPRESSION_OPERATIONS,
    );
    request.frozen_limits.expression_operations = 8;
    let decision = transition(&request).expect("eight operations fit");
    assert_eq!(
        parse(&decision.commands[0].payload)["input"],
        json!({"a": 1, "b": [true, 7],
               "c": {"deep": [1, 2, 3], "$get": ["not", "evaluated"]}})
    );
}

#[test]
fn an_absent_mapping_costs_one_operation() {
    let mut request = start_request(&chain(), json!(null));
    request.frozen_limits.expression_operations = 0;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_EXPRESSION_OPERATIONS,
    );
    request.frozen_limits.expression_operations = 1;
    assert!(transition(&request).is_ok());
}

#[test]
fn a_transition_without_a_mapping_costs_no_operations() {
    let mut run = Run::started(&chain(), json!(null));
    run.limits = limits(|l| l.expression_operations = 0);
    run.signal("go", Some("a"), json!(1), 1_100);
    assert_eq!(run.status(), "running");
}

#[test]
fn more_commands_than_the_budget_is_a_failure() {
    let mut request = start_request(&chain(), json!(null));
    request.frozen_limits.max_command_count = 0;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_COMMAND_COUNT,
    );
    request.frozen_limits.max_command_count = 1;
    assert!(transition(&request).is_ok());
}

#[test]
fn a_transition_without_commands_passes_a_zero_command_budget() {
    let mut run = Run::started(&chain(), json!(null));
    run.limits = limits(|l| l.max_command_count = 0);
    run.signal("go", Some("a"), json!(1), 1_100);
    assert_eq!(run.status(), "running");
}

#[test]
fn result_checks_run_in_their_documented_order() {
    let mut request = start_request(&chain(), json!(null));
    request.frozen_limits = Limits {
        microsteps: 1,
        expression_operations: 0,
        max_state_bytes: 1,
        max_command_count: 0,
    };
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_COMMAND_COUNT,
    );

    request.frozen_limits.max_command_count = 1;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_STATE_BYTES,
    );

    request.frozen_limits.max_state_bytes = 256 * 1024;
    assert_failure(
        transition(&request),
        ResourceLimit,
        code::BUDGET_EXPRESSION_OPERATIONS,
    );

    request.frozen_limits.microsteps = 0;
    assert_failure(transition(&request), ResourceLimit, code::BUDGET_MICROSTEPS);
}

/// Request checks come before any budget.
#[test]
fn budgets_do_not_mask_request_failures() {
    let mut request = start_request(&chain(), json!(null));
    request.frozen_limits = Limits {
        microsteps: 0,
        expression_operations: 0,
        max_state_bytes: 0,
        max_command_count: 0,
    };
    request.input_event.sequence = 2;
    assert_failure(
        transition(&request),
        runspore_types::reducer::FailureKind::InvalidInput,
        code::EVENT_OUT_OF_ORDER,
    );
}

/// A value nested deeper than the canonical limit of 32 cannot be emitted. An
/// activity output 31 levels deep is a canonical payload but sits three levels down
/// in the snapshot.
#[test]
fn a_value_too_deep_to_record_is_a_failure() {
    let mut run = Run::started(&chain(), json!(null));
    let before = run.snapshot.clone();
    let nest = |levels: usize| (0..levels).fold(json!(0), |inner, _| json!([inner]));

    let mut payload = run.result("success");
    set(&mut payload, "/output", nest(30));
    let result = run.apply(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_failure(result, ResourceLimit, "budget.value-depth");
    assert_eq!(run.snapshot, before);

    set(&mut payload, "/output", nest(29));
    run.step(event_kind::ACTIVITY_RESULT, &payload, 2_000);
    assert_eq!(run.status(), "waiting");
}
