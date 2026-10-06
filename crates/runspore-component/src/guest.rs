use runspore_types::reducer as types;

use exports::runspore::machine::reducer as wit;

wit_bindgen::generate!({
    path: "../../contracts/wit/machine",
    world: "machine",
});

struct Machine;

export!(Machine);

impl wit::Guest for Machine {
    fn describe() -> wit::Descriptor {
        descriptor_to_wit(runspore_kernel::describe())
    }

    fn transition(input: wit::Request) -> Result<wit::Decision, wit::Failure> {
        run(&request_from_wit(input))
            .map(decision_to_wit)
            .map_err(failure_to_wit)
    }
}

#[cfg(not(feature = "echo"))]
fn run(request: &types::TransitionRequest) -> Result<types::Decision, types::Failure> {
    runspore_kernel::transition(request)
}

#[cfg(feature = "echo")]
fn run(request: &types::TransitionRequest) -> Result<types::Decision, types::Failure> {
    crate::echo::transition(request)
}

fn request_from_wit(r: wit::Request) -> types::TransitionRequest {
    types::TransitionRequest {
        identity: types::Identity {
            package_digest: r.identity.package_digest,
            kernel_digest: r.identity.kernel_digest,
            semantics_version: r.identity.semantics_version,
            codec_version: r.identity.codec_version,
        },
        graph: r.graph,
        snapshot: r.snapshot,
        input_event: types::Envelope {
            event_id: r.input_event.event_id,
            sequence: r.input_event.sequence,
            accepted_at_ms: r.input_event.accepted_at_ms,
            kind: r.input_event.kind,
            payload: r.input_event.payload,
        },
        frozen_limits: types::Limits {
            microsteps: r.frozen_limits.microsteps,
            expression_operations: r.frozen_limits.expression_operations,
            max_state_bytes: r.frozen_limits.max_state_bytes,
            max_command_count: r.frozen_limits.max_command_count,
        },
    }
}

fn decision_to_wit(d: types::Decision) -> wit::Decision {
    wit::Decision {
        snapshot: d.snapshot,
        snapshot_digest: d.snapshot_digest,
        commands: d
            .commands
            .into_iter()
            .map(|c| wit::Command {
                command_id: c.command_id,
                activation_id: c.activation_id,
                kind: c.kind,
                payload: c.payload,
            })
            .collect(),
        diagnostics: d
            .diagnostics
            .into_iter()
            .map(|g| wit::Diagnostic {
                code: g.code,
                node_id: g.node_id,
                details: g.details,
            })
            .collect(),
    }
}

fn failure_to_wit(f: types::Failure) -> wit::Failure {
    wit::Failure {
        kind: match f.kind {
            types::FailureKind::InvalidInput => wit::FailureKind::InvalidInput,
            types::FailureKind::IncompatibleVersion => wit::FailureKind::IncompatibleVersion,
            types::FailureKind::ResourceLimit => wit::FailureKind::ResourceLimit,
            types::FailureKind::InvariantViolation => wit::FailureKind::InvariantViolation,
        },
        code: f.code,
        details: f.details,
    }
}

fn descriptor_to_wit(d: types::Descriptor) -> wit::Descriptor {
    wit::Descriptor {
        abi_version: d.abi_version,
        semantics_version: d.semantics_version,
        graph_format_version: d.graph_format_version,
        codec_version: d.codec_version,
    }
}
