//! `WasmtimeReducer` against the real kernel component and the test-only echo
//! component built by the same pipeline (`runspore-component` with feature `echo`).

use std::sync::Arc;

use runspore_types::digest;
use runspore_types::reducer::{
    Command, Decision, Diagnostic, Envelope, Failure, FailureKind, Identity, Limits, Reducer,
    TransitionRequest,
};
use runspore_wasmtime::{Guards, WasmtimeReducer, COMPONENT};

const ECHO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/echo.wasm"));

fn request(kind: &str, payload: Vec<u8>) -> TransitionRequest {
    TransitionRequest {
        identity: Identity {
            package_digest: "sha256:package".to_string(),
            kernel_digest: "sha256:kernel".to_string(),
            semantics_version: "0.1".to_string(),
            codec_version: "jcs-1".to_string(),
        },
        graph: br#"{"nodes":[]}"#.to_vec(),
        snapshot: Some(vec![0, 1, 2, 254, 255]),
        input_event: Envelope {
            event_id: "evt-\u{00e9}\u{1f600}".to_string(),
            sequence: u64::MAX,
            accepted_at_ms: 1_759_752_000_123,
            kind: kind.to_string(),
            payload,
        },
        frozen_limits: Limits {
            microsteps: 1,
            expression_operations: 2,
            max_state_bytes: u32::MAX,
            max_command_count: 4,
        },
    }
}

fn echo(guards: Guards) -> WasmtimeReducer {
    WasmtimeReducer::from_component(ECHO, guards).expect("echo component compiles")
}

fn host_failure(result: Result<Decision, Failure>) -> (FailureKind, String) {
    let failure = result.expect_err("guest must not produce a decision");
    (failure.kind, failure.code)
}

#[test]
fn describe_through_wasmtime_equals_native() {
    let reducer = WasmtimeReducer::new().expect("kernel compiles");
    assert_eq!(reducer.describe(), runspore_kernel::describe());
}

#[test]
fn transition_through_wasmtime_equals_native() {
    let reducer = WasmtimeReducer::new().expect("kernel compiles");
    for snapshot in [None, Some(b"{}".to_vec())] {
        let mut request = request("run.started", b"{}".to_vec());
        request.snapshot = snapshot;
        assert_eq!(
            reducer.transition(&request),
            runspore_kernel::transition(&request)
        );
    }
}

#[test]
fn kernel_digest_hashes_the_component_bytes() {
    let reducer = WasmtimeReducer::new().expect("kernel compiles");
    assert_eq!(reducer.component_bytes(), COMPONENT);
    assert_eq!(
        reducer.kernel_digest(),
        digest::hash("kernel", &[COMPONENT])
    );
}

#[test]
fn every_request_field_reaches_the_guest() {
    let reducer = echo(Guards::default());
    for snapshot in [None, Some(Vec::new()), Some(vec![0, 255])] {
        let mut sent = request("echo.request", vec![0, 159, 146, 150, 255]);
        sent.snapshot = snapshot;
        let decision = reducer.transition(&sent).expect("echo decides");
        let received: TransitionRequest =
            serde_json::from_slice(&decision.snapshot).expect("guest echoes JSON");
        assert_eq!(received, sent);
    }
}

#[test]
fn every_decision_field_reaches_the_host() {
    let reducer = echo(Guards::default());
    let decision = Decision {
        snapshot: vec![0, 1, 255],
        snapshot_digest: "sha256:snapshot".to_string(),
        commands: vec![
            Command {
                command_id: "cmd-1".to_string(),
                activation_id: "act-1".to_string(),
                kind: "activity.start".to_string(),
                payload: b"{\"a\":1}".to_vec(),
            },
            Command {
                command_id: "cmd-2".to_string(),
                activation_id: "act-\u{1f600}".to_string(),
                kind: "timer.start".to_string(),
                payload: Vec::new(),
            },
        ],
        diagnostics: vec![
            Diagnostic {
                code: "node.skipped".to_string(),
                node_id: Some("node-7".to_string()),
                details: b"{}".to_vec(),
            },
            Diagnostic {
                code: "graph.note".to_string(),
                node_id: None,
                details: vec![255],
            },
        ],
    };
    let sent = request("echo.decision", serde_json::to_vec(&decision).unwrap());
    assert_eq!(reducer.transition(&sent), Ok(decision));
}

#[test]
fn every_failure_kind_reaches_the_host() {
    let reducer = echo(Guards::default());
    for kind in [
        FailureKind::InvalidInput,
        FailureKind::IncompatibleVersion,
        FailureKind::ResourceLimit,
        FailureKind::InvariantViolation,
    ] {
        let failure = Failure {
            kind,
            code: "test.code".to_string(),
            details: "details \u{00e9}".to_string(),
        };
        let sent = request("echo.failure", serde_json::to_vec(&failure).unwrap());
        assert_eq!(reducer.transition(&sent), Err(failure));
    }
}

#[test]
fn runaway_guest_is_stopped_by_the_watchdog() {
    let reducer = echo(Guards {
        fuel: 20_000_000,
        ..Guards::default()
    });
    assert_eq!(
        host_failure(reducer.transition(&request("loop", Vec::new()))),
        (FailureKind::InvariantViolation, "host.watchdog".to_string())
    );
}

#[test]
fn trapping_guest_is_a_host_trap() {
    let reducer = echo(Guards::default());
    assert_eq!(
        host_failure(reducer.transition(&request("trap", Vec::new()))),
        (FailureKind::InvariantViolation, "host.trap".to_string())
    );
}

#[test]
fn memory_hungry_guest_is_a_host_memory_failure() {
    let reducer = echo(Guards {
        max_memory_bytes: 8 * 1024 * 1024,
        ..Guards::default()
    });
    assert_eq!(
        host_failure(reducer.transition(&request("memory", Vec::new()))),
        (FailureKind::InvariantViolation, "host.memory".to_string())
    );
}

#[test]
fn a_failed_call_leaves_the_reducer_usable() {
    let reducer = echo(Guards {
        fuel: 20_000_000,
        max_memory_bytes: 8 * 1024 * 1024,
    });
    for kind in ["trap", "loop", "memory"] {
        assert!(reducer.transition(&request(kind, Vec::new())).is_err());
        let sent = request("echo.request", Vec::new());
        let decision = reducer.transition(&sent).expect("next call succeeds");
        assert_eq!(
            serde_json::from_slice::<TransitionRequest>(&decision.snapshot).unwrap(),
            sent
        );
    }
}

#[test]
fn every_call_runs_in_a_fresh_instance() {
    let reducer = echo(Guards::default());
    for _ in 0..3 {
        let decision = reducer
            .transition(&request("count", Vec::new()))
            .expect("count decides");
        assert_eq!(decision.snapshot, b"1");
    }
}

#[test]
fn concurrent_calls_get_independent_results() {
    let reducer = Arc::new(echo(Guards::default()));
    let threads: Vec<_> = (0..16)
        .map(|thread| {
            let reducer = Arc::clone(&reducer);
            std::thread::spawn(move || {
                for call in 0..25u64 {
                    let mut sent = request("echo.request", thread.to_string().into_bytes());
                    sent.input_event.sequence = call;
                    sent.input_event.event_id = format!("evt-{thread}-{call}");
                    let decision = reducer.transition(&sent).expect("echo decides");
                    let received: TransitionRequest =
                        serde_json::from_slice(&decision.snapshot).unwrap();
                    assert_eq!(received, sent);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("no thread panicked");
    }
}
