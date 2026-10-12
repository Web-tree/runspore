//! `spec/kernel.md` 3 and 7.4: a stuck step, which only a 0.2 document has.
//! The golden traces `14*` pin the decisions; these tests pin the snapshot checks.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, event_kind, WORKFLOW_FORMAT_0_2};
use runspore_types::reducer::FailureKind::InvalidInput;
use serde_json::{json, Value};

/// A 0.2 run whose only step failed for good with no `failed` route.
fn stuck() -> Run {
    let workflow = with(
        single_activity("idempotent", json!({"maxAttempts": 1}), false),
        "/format",
        json!(WORKFLOW_FORMAT_0_2),
    );
    let mut run = Run::started(&workflow, json!(null));
    let decision = run.fail(false, 2_000);
    assert_eq!(codes(&decision), [code::INVOCATION_STUCK]);
    assert!(
        decision.commands.is_empty(),
        "a stuck step emits no command"
    );
    run
}

fn retry(run: &Run) -> Value {
    json!({"invocationId": run.invocation_id(), "resolution": {"action": "retry"}})
}

#[test]
fn a_stuck_step_keeps_its_error_and_needs_intervention() {
    let run = stuck();
    let state = run.state();
    assert_eq!(state["status"], "needs-intervention");
    assert_eq!(state["invocation"]["state"], "stuck");
    assert_eq!(state["invocation"]["error"]["code"], "io.broken");
    assert_eq!(state["invocation"]["error"]["nodeId"], "work");
    assert_eq!(state["result"], Value::Null);
}

#[test]
fn a_stuck_snapshot_continues_only_with_its_0_2_document() {
    let mut run = stuck();
    let mut workflow = parse(&run.graph);
    workflow["format"] = json!("runspore.workflow/0.1");
    let request = {
        let mut other = Run::new(&workflow);
        other.snapshot.clone_from(&run.snapshot);
        other.sequence = run.sequence;
        other.next(event_kind::INVOCATION_RESOLVED, &retry(&run), 3_000)
    };
    assert_failure(transition(&request), InvalidInput, code::STATE_INVALID);

    let decision = run.step(event_kind::INVOCATION_RESOLVED, &retry(&run), 3_000);
    assert_eq!(codes(&decision), [code::ACTIVITY_RETRY_SCHEDULED]);
    assert_eq!(run.attempt(), 2, "a retry goes past maxAttempts");
    assert_eq!(run.state()["invocation"].get("error"), None);
}

/// Exactly a stuck invocation keeps an error, and only `needs-intervention` has one.
#[test]
fn a_stuck_snapshot_that_contradicts_itself_is_invalid() {
    let run = stuck();
    let error = run.state()["invocation"]["error"].clone();
    let edits: [(&str, Option<Value>); 4] = [
        ("/invocation/error", None),
        ("/invocation/state", Some(json!("unknown"))),
        ("/invocation/state", Some(json!("scheduled"))),
        ("/status", Some(json!("running"))),
    ];
    for (pointer, value) in edits {
        let mut state = run.state();
        match &value {
            Some(value) => set(&mut state, pointer, value.clone()),
            None => unset(&mut state, pointer),
        }
        let mut request = run.next(event_kind::INVOCATION_RESOLVED, &retry(&run), 3_000);
        request.snapshot = Some(canon(&state));
        let result = transition(&request);
        assert!(result.is_err(), "{pointer} = {value:?} was accepted");
        assert_failure(result, InvalidInput, code::STATE_INVALID);
    }

    let mut scheduled = Run::started(&parse(&run.graph), json!(null));
    let mut state = scheduled.state();
    set(&mut state, "/invocation/error", error);
    scheduled.snapshot = Some(canon(&state));
    let request = scheduled.next(
        event_kind::ACTIVITY_RESULT,
        &scheduled.result("success"),
        3_000,
    );
    assert_failure(transition(&request), InvalidInput, code::STATE_INVALID);
}
