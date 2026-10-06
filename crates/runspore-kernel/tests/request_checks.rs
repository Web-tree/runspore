//! `spec/kernel.md` 7.1: request checks run in order and the first failing check wins.
//!
//! Each test makes a later check fail as well, so it also pins the order.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::model::{code, event_kind};
use runspore_types::reducer::FailureKind::{IncompatibleVersion, InvalidInput};
use runspore_types::reducer::TransitionRequest;
use serde_json::{json, Value};

const NOT_JSON: &[u8] = b"not json";

fn start() -> TransitionRequest {
    start_request(&review_loop(), json!({"repo": "r"}))
}

/// A run parked at its first activity: a valid snapshot at sequence 1.
fn running() -> Run {
    Run::started(&review_loop(), json!({"repo": "r"}))
}

fn signal() -> Value {
    json!({"name": "approval", "outcome": "approved", "data": null})
}

/// The next request of `run` with its snapshot replaced by an edited copy.
fn with_snapshot(run: &Run, edit: impl FnOnce(&mut Value)) -> TransitionRequest {
    let mut state = run.state();
    edit(&mut state);
    let mut request = run.next(event_kind::SIGNAL_RECEIVED, &signal(), 2_000);
    request.snapshot = Some(canon(&state));
    request
}

#[test]
fn a_well_formed_request_is_accepted() {
    assert!(transition(&start()).is_ok());
}

#[test]
fn check_1_semantics_version() {
    let mut request = start();
    request.identity.semantics_version = "0.2".to_string();
    request.identity.codec_version = "jcs-int53/2".to_string();
    request.graph = NOT_JSON.to_vec();
    assert_failure(
        transition(&request),
        IncompatibleVersion,
        code::VERSION_SEMANTICS,
    );
}

#[test]
fn check_2_codec_version() {
    let mut request = start();
    request.identity.codec_version = "jcs-int53/2".to_string();
    request.graph = NOT_JSON.to_vec();
    assert_failure(
        transition(&request),
        IncompatibleVersion,
        code::VERSION_CODEC,
    );
}

#[test]
fn check_3_graph_must_be_a_canonical_workflow() {
    let canonical = canon(&review_loop());
    let pretty = serde_json::to_vec_pretty(&review_loop()).unwrap();
    let unsorted = br#"{"start":"a","format":"runspore.workflow/0.1","name":"n","nodes":{"a":{"kind":"complete"}}}"#;
    let sorted = br#"{"format":"runspore.workflow/0.1","name":"n","nodes":{"a":{"kind":"complete"}},"start":"a"}"#;
    assert!(transition(&request(
        sorted,
        None,
        envelope(1, START_MS, event_kind::RUN_STARTED, &started(json!(null)))
    ))
    .is_ok());

    let mut truncated = canonical.clone();
    truncated.pop();
    let cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        NOT_JSON.to_vec(),
        b"{}".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        truncated,
        pretty,
        unsorted.to_vec(),
        br#"{"format":"runspore.workflow/0.1","name":"n","nodes":{"a":{"kind":"complete","output":1.5}},"start":"a"}"#.to_vec(),
        br#"{"format":"runspore.workflow/0.1","name":"n","name":"n","nodes":{"a":{"kind":"complete"}},"start":"a"}"#.to_vec(),
        canon(&with(review_loop(), "/extra", json!(1))),
        canon(&without(review_loop(), "/nodes")),
    ];
    for graph in cases {
        let mut request = start();
        request.graph = graph.clone();
        request.input_event.kind = "mystery".to_string();
        assert_failure(transition(&request), InvalidInput, code::WORKFLOW_INVALID);
    }
}

#[test]
fn check_4_unknown_event_kind() {
    let mut request = start();
    request.input_event.kind = "timer.fired".to_string();
    request.input_event.payload = NOT_JSON.to_vec();
    assert_failure(transition(&request), InvalidInput, code::EVENT_UNKNOWN_KIND);
}

#[test]
fn check_4_payload_must_decode_canonically_as_its_kind() {
    let cases: Vec<(&str, &[u8])> = vec![
        (event_kind::RUN_STARTED, b""),
        (event_kind::RUN_STARTED, NOT_JSON),
        (event_kind::RUN_STARTED, br#"{"tenant":"default"}"#),
        (
            event_kind::RUN_STARTED,
            br#"{"input":null,"runId":"run_1","tenant":"default","zzz":1}"#,
        ),
        (
            event_kind::RUN_STARTED,
            br#"{"tenant":"default","runId":"run_1","input":null}"#,
        ),
        (
            event_kind::RUN_STARTED,
            br#"{"input": null,"runId":"run_1","tenant":"default"}"#,
        ),
        (
            event_kind::RUN_STARTED,
            br#"{"input":1.5,"runId":"run_1","tenant":"default"}"#,
        ),
        (event_kind::ACTIVITY_RESULT, b"{}"),
        (
            event_kind::ACTIVITY_RESULT,
            br#"{"attempt":1,"attemptId":"a.1","invocationId":"a","status":"maybe"}"#,
        ),
        (
            event_kind::ACTIVITY_RESULT,
            br#"{"attempt":"1","attemptId":"a.1","invocationId":"a","status":"success"}"#,
        ),
        (
            event_kind::ACTIVITY_RESULT,
            br#"{"attempt":-1,"attemptId":"a.1","invocationId":"a","status":"success"}"#,
        ),
        (
            event_kind::ACTIVITY_RESULT,
            br#"{"attempt":4294967296,"attemptId":"a.1","invocationId":"a","status":"success"}"#,
        ),
        (event_kind::SIGNAL_RECEIVED, b"{}"),
        (event_kind::SIGNAL_RECEIVED, br#"{"name":7}"#),
        (event_kind::SIGNAL_RECEIVED, br#"{"extra":1,"name":"a"}"#),
        (event_kind::INVOCATION_RESOLVED, br#"{"invocationId":"a"}"#),
        (
            event_kind::INVOCATION_RESOLVED,
            br#"{"invocationId":"a","resolution":{"action":"shrug"}}"#,
        ),
        (
            event_kind::INVOCATION_RESOLVED,
            br#"{"invocationId":"a","resolution":{"action":"complete"}}"#,
        ),
        (
            event_kind::INVOCATION_RESOLVED,
            br#"{"invocationId":"a","resolution":{"action":"fail"}}"#,
        ),
    ];
    for (kind, payload) in cases {
        let mut request = start();
        request.input_event.kind = kind.to_string();
        request.input_event.payload = payload.to_vec();
        request.input_event.sequence = 7;
        assert_failure(transition(&request), InvalidInput, code::EVENT_INVALID);
    }
}

/// Counters and timestamps stop at 2^63 − 1 (section 1); an envelope beyond that
/// cannot be recorded in a snapshot.
#[test]
fn check_4_envelope_counters_stay_within_the_counter_domain() {
    let mut request = start();
    request.input_event.accepted_at_ms = 1 << 63;
    assert_failure(transition(&request), InvalidInput, code::EVENT_INVALID);

    let mut request = start();
    request.input_event.sequence = u64::MAX;
    assert_failure(transition(&request), InvalidInput, code::EVENT_INVALID);

    let mut request = start();
    request.input_event.accepted_at_ms = (1 << 63) - 1;
    assert!(transition(&request).is_ok());
}

#[test]
fn check_5_snapshot_absent_without_start() {
    let invocation = running().invocation_id();
    let cases = [
        (
            event_kind::ACTIVITY_RESULT,
            result_for(&invocation, 1, "success"),
        ),
        (event_kind::SIGNAL_RECEIVED, signal()),
        (
            event_kind::INVOCATION_RESOLVED,
            json!({"invocationId": invocation, "resolution": {"action": "retry"}}),
        ),
    ];
    for (kind, payload) in cases {
        let request = request(
            &canon(&review_loop()),
            None,
            envelope(9, 2_000, kind, &payload),
        );
        assert_failure(transition(&request), InvalidInput, code::STATE_MISSING);
    }
}

#[test]
fn check_5_snapshot_present_with_start() {
    let mut request = start();
    request.snapshot = Some(NOT_JSON.to_vec());
    request.input_event.sequence = 9;
    assert_failure(
        transition(&request),
        InvalidInput,
        code::STATE_UNEXPECTED_START,
    );

    let mut request = start();
    request.snapshot = running().snapshot;
    assert_failure(
        transition(&request),
        InvalidInput,
        code::STATE_UNEXPECTED_START,
    );
}

#[test]
fn check_5_snapshot_undecodable() {
    let run = running();
    let pretty = serde_json::to_vec_pretty(&run.state()).unwrap();
    let mut truncated = run.snapshot.clone().unwrap();
    truncated.pop();
    let cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        NOT_JSON.to_vec(),
        b"{}".to_vec(),
        b"null".to_vec(),
        truncated,
        pretty,
        canon(&without(run.state(), "/visits")),
        canon(&without(run.state(), "/format")),
        canon(&with(run.state(), "/extra", json!(1))),
        canon(&with(run.state(), "/status", json!("paused"))),
        canon(&with(run.state(), "/lastSequence", json!(1))),
        canon(&with(run.state(), "/lastSequence", json!("01"))),
        canon(&with(run.state(), "/activations", json!(-1))),
        canon(&with(run.state(), "/format", json!(1))),
    ];
    for snapshot in cases {
        let mut request = run.next(event_kind::SIGNAL_RECEIVED, &signal(), 2_000);
        request.snapshot = Some(snapshot);
        request.input_event.sequence = 9;
        assert_failure(transition(&request), InvalidInput, code::STATE_INVALID);
    }
}

#[test]
fn check_5_snapshot_of_another_version() {
    let run = running();
    let edits: [fn(&mut Value); 3] = [
        |state| set(state, "/format", json!("runspore.state/0.2")),
        |state| set(state, "/semantics", json!("0.2")),
        |state| {
            set(state, "/format", json!("runspore.state/9.9"));
            set(state, "/addedLater", json!(true));
        },
    ];
    for edit in edits {
        let mut request = with_snapshot(&run, edit);
        request.input_event.sequence = 9;
        assert_failure(
            transition(&request),
            IncompatibleVersion,
            code::VERSION_STATE_FORMAT,
        );
    }
}

/// A snapshot that decodes but contradicts itself or the graph is not a state this
/// kernel produced.
#[test]
fn check_5_snapshot_inconsistent_with_itself_or_the_graph() {
    let run = running();
    let signal = json!({"eventId": "e", "sequence": "1", "name": "n",
                        "outcome": null, "data": null});
    let edits = [
        ("/position", json!(null)),
        ("/invocation", json!(null)),
        ("/status", json!("waiting")),
        ("/status", json!("needs-intervention")),
        ("/status", json!("completed")),
        ("/status", json!("failed")),
        ("/result", json!({"output": 1})),
        ("/position/nodeId", json!("approve")),
        ("/position/nodeId", json!("nowhere")),
        ("/position/activationId", json!("build/2")),
        ("/position/visit", json!(2)),
        ("/position/visit", json!(0)),
        ("/visits", json!({})),
        ("/invocation/nodeId", json!("approve")),
        ("/invocation/actionId", json!("other")),
        ("/invocation/activationId", json!("build/2")),
        ("/invocation/invocationId", json!("inv_other")),
        ("/invocation/attempt", json!(0)),
        ("/invocation/state", json!("unknown")),
        ("/signals", Value::Array(vec![signal; 65])),
    ];
    for (pointer, value) in edits {
        let mut request = with_snapshot(&run, |state| set(state, pointer, value.clone()));
        request.input_event.sequence = 9;
        let result = transition(&request);
        assert!(result.is_err(), "{pointer} = {value} was accepted");
        assert_failure(result, InvalidInput, code::STATE_INVALID);
    }
}

#[test]
fn check_6_run_started_has_sequence_one() {
    for sequence in [0, 2, 100] {
        let mut request = start();
        request.input_event.sequence = sequence;
        request.input_event.payload =
            canon(&json!({"tenant": "bad tenant", "runId": RUN_ID, "input": null}));
        assert_failure(transition(&request), InvalidInput, code::EVENT_OUT_OF_ORDER);
    }
}

#[test]
fn check_6_later_events_follow_the_last_sequence() {
    let mut run = running();
    run.signal("noise", None, json!(null), 2_000);
    assert_eq!(run.state()["lastSequence"], json!("2"));
    for sequence in [0, 1, 2, 4, 100] {
        let mut request = run.next(event_kind::SIGNAL_RECEIVED, &signal(), 3_000);
        request.input_event.sequence = sequence;
        assert_failure(transition(&request), InvalidInput, code::EVENT_OUT_OF_ORDER);
    }
    let request = run.next(event_kind::SIGNAL_RECEIVED, &signal(), 3_000);
    assert_eq!(request.input_event.sequence, 3);
    assert!(transition(&request).is_ok());
}

#[test]
fn check_7_tenant_and_run_id_patterns() {
    let long = "a".repeat(129);
    let bad = ["", "has space", "slash/y", "ünï", long.as_str()];
    for value in bad {
        for field in ["/tenant", "/runId"] {
            let mut request = start();
            request.input_event.payload = canon(&with(started(json!(null)), field, json!(value)));
            assert_failure(transition(&request), InvalidInput, code::EVENT_INVALID);
        }
    }
    let widest = "aZ0_.-".repeat(21) + "ab";
    assert_eq!(widest.len(), 128);
    let mut request = start();
    request.input_event.payload =
        canon(&json!({"tenant": widest, "runId": widest, "input": {"repo": "r"}}));
    assert!(transition(&request).is_ok());
}
