//! The Runspore graph kernel. Pure and import-free: see `spec/kernel.md`.
//!
//! [`transition`] is the whole kernel: one request in, one decision or one failure
//! out, with the same bytes on every host. It reads no clock, performs no I/O, keeps
//! no state between calls, and returns a [`Failure`] for every malformed request.

mod event;
mod failure;
mod graph;
mod machine;
mod mapping;
mod pattern;
mod snapshot;

use runspore_types::model::{
    code as wire, ABI_VERSION, CODEC_VERSION, SEMANTICS_VERSION, WORKFLOW_FORMAT,
};
use runspore_types::reducer::{Decision, Descriptor, Failure, Reducer, TransitionRequest};

use crate::event::Event;
use crate::graph::Graph;
use crate::machine::Machine;

/// Failure codes this kernel reports beyond `runspore_types::model::code`, for
/// conditions the frozen vocabulary does not name.
pub mod code {
    /// `resource-limit`: a value the kernel must emit would nest deeper than
    /// `canonical::MAX_DEPTH`.
    pub const BUDGET_VALUE_DEPTH: &str = "budget.value-depth";
    /// `resource-limit`: an attempt number or a retry time would leave its counter domain.
    pub const BUDGET_COUNTER_RANGE: &str = "budget.counter-range";
    /// `invariant-violation`: the kernel reached a state its own request checks rule out.
    pub const KERNEL_INVARIANT: &str = "kernel.invariant";
}

pub fn describe() -> Descriptor {
    Descriptor {
        abi_version: ABI_VERSION.to_string(),
        semantics_version: SEMANTICS_VERSION.to_string(),
        graph_format_version: WORKFLOW_FORMAT.to_string(),
        codec_version: CODEC_VERSION.to_string(),
    }
}

/// Applies one event. Request checks run in the order of `spec/kernel.md` 7.1 and
/// the first failing check is the result; a failure carries no partial decision.
pub fn transition(request: &TransitionRequest) -> Result<Decision, Failure> {
    let identity = &request.identity;
    if identity.semantics_version != SEMANTICS_VERSION {
        return Err(failure::incompatible(
            wire::VERSION_SEMANTICS,
            format!(
                "semantics version {:?} is not {SEMANTICS_VERSION:?}",
                identity.semantics_version
            ),
        ));
    }
    if identity.codec_version != CODEC_VERSION {
        return Err(failure::incompatible(
            wire::VERSION_CODEC,
            format!(
                "codec version {:?} is not {CODEC_VERSION:?}",
                identity.codec_version
            ),
        ));
    }

    let graph = Graph::load(&request.graph)?;
    let envelope = &request.input_event;
    let event = Event::decode(envelope)?;

    let state = match (request.snapshot.as_deref(), &event) {
        (None, Event::Started(started)) => {
            if envelope.sequence != 1 {
                return Err(out_of_order(envelope.sequence));
            }
            if !pattern::is_name(&started.tenant) || !pattern::is_name(&started.run_id) {
                return Err(failure::invalid(
                    wire::EVENT_INVALID,
                    "tenant and runId must match [A-Za-z0-9_.-]{1,128}",
                ));
            }
            snapshot::initial(started)
        }
        (None, _) => {
            return Err(failure::invalid(
                wire::STATE_MISSING,
                format!("{} needs a snapshot", envelope.kind),
            ));
        }
        (Some(_), Event::Started(_)) => {
            return Err(failure::invalid(
                wire::STATE_UNEXPECTED_START,
                "run.started does not take a snapshot",
            ));
        }
        (Some(bytes), _) => {
            let state = snapshot::load(bytes, &graph)?;
            if state.last_sequence.checked_add(1) != Some(envelope.sequence) {
                return Err(out_of_order(envelope.sequence));
            }
            state
        }
    };

    let mut machine = Machine::new(&graph, request.frozen_limits, state);
    machine.apply(envelope, event)?;
    machine.finish()
}

fn out_of_order(found: u64) -> Failure {
    failure::invalid(
        wire::EVENT_OUT_OF_ORDER,
        format!("sequence {found} does not follow the last applied event"),
    )
}

/// The kernel called in-process, without a WebAssembly boundary.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeReducer;

impl Reducer for NativeReducer {
    fn describe(&self) -> Descriptor {
        describe()
    }

    fn kernel_digest(&self) -> String {
        format!("native:runspore-kernel@{}", env!("CARGO_PKG_VERSION"))
    }

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        transition(request)
    }
}
