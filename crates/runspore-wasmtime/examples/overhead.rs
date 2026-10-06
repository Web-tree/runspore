//! Measures host overhead of `WasmtimeReducer` on this machine: the cold first call
//! (engine setup, compile, `describe`, one transition) and the warm per-transition
//! cost. Run in release:
//!
//! `cargo run --release -p runspore-wasmtime --example overhead`

use std::time::Instant;

use runspore_types::reducer::{Envelope, Identity, Limits, Reducer, TransitionRequest};
use runspore_wasmtime::{WasmtimeReducer, COMPONENT};

const WARM_CALLS: u32 = 10_000;

fn main() {
    let request = TransitionRequest {
        identity: Identity {
            package_digest: "sha256:package".to_string(),
            kernel_digest: "sha256:kernel".to_string(),
            semantics_version: "0.1".to_string(),
            codec_version: "jcs-1".to_string(),
        },
        graph: b"{}".to_vec(),
        snapshot: None,
        input_event: Envelope {
            event_id: "evt-1".to_string(),
            sequence: 1,
            accepted_at_ms: 0,
            kind: "run.started".to_string(),
            payload: b"{}".to_vec(),
        },
        frozen_limits: Limits::default(),
    };

    let cold = Instant::now();
    let reducer = WasmtimeReducer::new().expect("kernel compiles");
    let first = reducer.transition(&request);
    let cold = cold.elapsed();

    let warm = Instant::now();
    for _ in 0..WARM_CALLS {
        assert_eq!(reducer.transition(&request), first);
    }
    let warm = warm.elapsed() / WARM_CALLS;

    println!("component bytes: {}", COMPONENT.len());
    println!("cold first call (compile + describe + transition): {cold:?}");
    println!("warm transition (fresh instance per call): {warm:?}");
}
