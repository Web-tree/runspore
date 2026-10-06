//! Field-for-field conversion between the generated WIT records and
//! `runspore_types::reducer`.

use runspore_types::reducer as types;

use crate::exports::runspore::machine::reducer as wit;

pub fn request_to_wit(r: &types::TransitionRequest) -> wit::Request {
    wit::Request {
        identity: wit::Identity {
            package_digest: r.identity.package_digest.clone(),
            kernel_digest: r.identity.kernel_digest.clone(),
            semantics_version: r.identity.semantics_version.clone(),
            codec_version: r.identity.codec_version.clone(),
        },
        graph: r.graph.clone(),
        snapshot: r.snapshot.clone(),
        input_event: wit::Envelope {
            event_id: r.input_event.event_id.clone(),
            sequence: r.input_event.sequence,
            accepted_at_ms: r.input_event.accepted_at_ms,
            kind: r.input_event.kind.clone(),
            payload: r.input_event.payload.clone(),
        },
        frozen_limits: wit::Limits {
            microsteps: r.frozen_limits.microsteps,
            expression_operations: r.frozen_limits.expression_operations,
            max_state_bytes: r.frozen_limits.max_state_bytes,
            max_command_count: r.frozen_limits.max_command_count,
        },
    }
}

pub fn decision_from_wit(d: wit::Decision) -> types::Decision {
    types::Decision {
        snapshot: d.snapshot,
        snapshot_digest: d.snapshot_digest,
        commands: d
            .commands
            .into_iter()
            .map(|c| types::Command {
                command_id: c.command_id,
                activation_id: c.activation_id,
                kind: c.kind,
                payload: c.payload,
            })
            .collect(),
        diagnostics: d
            .diagnostics
            .into_iter()
            .map(|g| types::Diagnostic {
                code: g.code,
                node_id: g.node_id,
                details: g.details,
            })
            .collect(),
    }
}

pub fn failure_from_wit(f: wit::Failure) -> types::Failure {
    types::Failure {
        kind: match f.kind {
            wit::FailureKind::InvalidInput => types::FailureKind::InvalidInput,
            wit::FailureKind::IncompatibleVersion => types::FailureKind::IncompatibleVersion,
            wit::FailureKind::ResourceLimit => types::FailureKind::ResourceLimit,
            wit::FailureKind::InvariantViolation => types::FailureKind::InvariantViolation,
        },
        code: f.code,
        details: f.details,
    }
}

pub fn descriptor_from_wit(d: wit::Descriptor) -> types::Descriptor {
    types::Descriptor {
        abi_version: d.abi_version,
        semantics_version: d.semantics_version,
        graph_format_version: d.graph_format_version,
        codec_version: d.codec_version,
    }
}
