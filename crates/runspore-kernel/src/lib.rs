//! The Runspore graph kernel. Pure and import-free: see `spec/kernel.md`.

use runspore_types::model::{ABI_VERSION, CODEC_VERSION, SEMANTICS_VERSION, WORKFLOW_FORMAT};
use runspore_types::reducer::{Decision, Descriptor, Failure, FailureKind, TransitionRequest};

pub fn describe() -> Descriptor {
    Descriptor {
        abi_version: ABI_VERSION.to_string(),
        semantics_version: SEMANTICS_VERSION.to_string(),
        graph_format_version: WORKFLOW_FORMAT.to_string(),
        codec_version: CODEC_VERSION.to_string(),
    }
}

pub fn transition(_request: &TransitionRequest) -> Result<Decision, Failure> {
    Err(Failure {
        kind: FailureKind::InvariantViolation,
        code: "kernel.unimplemented".to_string(),
        details: "placeholder until the kernel package lands".to_string(),
    })
}
