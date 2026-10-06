//! Test-only behaviours selected by `input_event.kind`, compiled only with the
//! `echo` feature.
//!
//! - `echo.request`: a decision whose snapshot is the received request as JSON.
//! - `echo.decision`: the decision encoded as JSON in `input_event.payload`.
//! - `echo.failure`: the failure encoded as JSON in `input_event.payload`.
//! - `count`: a decision whose snapshot is the number of calls this instance has
//!   served, in decimal.
//! - `loop`: never returns.
//! - `trap`: executes `unreachable`.
//! - `memory`: allocates until the host refuses to grow memory.
//!
//! Any other kind delegates to the kernel.

use core::sync::atomic::{AtomicU64, Ordering};

use runspore_types::reducer::{Decision, Failure, TransitionRequest};

/// Calls seen by this instance; stays at one when every call gets a fresh instance.
static CALLS: AtomicU64 = AtomicU64::new(0);

pub fn transition(request: &TransitionRequest) -> Result<Decision, Failure> {
    let payload = &request.input_event.payload;
    match request.input_event.kind.as_str() {
        "echo.request" => Ok(Decision {
            snapshot: serde_json::to_vec(request).expect("request serialises"),
            snapshot_digest: String::new(),
            commands: Vec::new(),
            diagnostics: Vec::new(),
        }),
        "echo.decision" => Ok(serde_json::from_slice(payload).expect("payload is a decision")),
        "echo.failure" => Err(serde_json::from_slice(payload).expect("payload is a failure")),
        "count" => {
            let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(Decision {
                snapshot: calls.to_string().into_bytes(),
                snapshot_digest: String::new(),
                commands: Vec::new(),
                diagnostics: Vec::new(),
            })
        }
        "loop" => loop {
            core::hint::black_box(());
        },
        "trap" => core::arch::wasm32::unreachable(),
        "memory" => {
            let mut hoard: Vec<Vec<u8>> = Vec::new();
            loop {
                hoard.push(core::hint::black_box(vec![1u8; 1 << 20]));
            }
        }
        _ => runspore_kernel::transition(request),
    }
}
