//! Snapshots: the initial state and request check 5 of `spec/kernel.md` 7.1.

use std::collections::BTreeMap;

use runspore_types::canonical;
use runspore_types::digest::ids;
use runspore_types::model::{
    code, InvocationPhase, InvocationState, Node, Position, RunResult, RunStarted, RunStatus,
    State, MAX_BUFFERED_SIGNALS, SEMANTICS_VERSION, STATE_FORMAT,
};
use runspore_types::reducer::Failure;
use serde::Deserialize;
use serde_json::Value;

use crate::failure;
use crate::graph::Graph;

/// The state a `run.started` event creates, before its start node is entered.
pub(crate) fn initial(started: &RunStarted) -> State {
    State {
        format: STATE_FORMAT.to_string(),
        semantics: SEMANTICS_VERSION.to_string(),
        tenant: started.tenant.clone(),
        run_id: started.run_id.clone(),
        status: RunStatus::Running,
        input: started.input.clone(),
        nodes: BTreeMap::new(),
        visits: BTreeMap::new(),
        activations: 0,
        position: None,
        invocation: None,
        signals: Vec::new(),
        result: None,
        last_sequence: 0,
        last_accepted_at_ms: 0,
    }
}

/// Decodes a canonical snapshot. A `format` or `semantics` of another version is
/// reported before the shape is judged, because a later format need not decode as
/// this one. A snapshot that decodes but contradicts itself or the graph is not a
/// state this kernel wrote, and is rejected like an undecodable one.
pub(crate) fn load(bytes: &[u8], graph: &Graph) -> Result<State, Failure> {
    let value = canonical::parse_canonical(bytes)
        .map_err(|error| failure::invalid(code::STATE_INVALID, format!("snapshot: {error}")))?;
    for (field, expected) in [("format", STATE_FORMAT), ("semantics", SEMANTICS_VERSION)] {
        if let Some(found) = value.get(field).and_then(Value::as_str) {
            if found != expected {
                return Err(failure::incompatible(
                    code::VERSION_STATE_FORMAT,
                    format!("snapshot {field} {found:?} is not {expected:?}"),
                ));
            }
        }
    }
    let state = State::deserialize(value)
        .map_err(|error| failure::invalid(code::STATE_INVALID, format!("snapshot: {error}")))?;
    consistent(&state, graph)
        .map_err(|reason| failure::invalid(code::STATE_INVALID, format!("snapshot: {reason}")))?;
    Ok(state)
}

/// The invariants of section 3 that the transition procedures rely on.
fn consistent(state: &State, graph: &Graph) -> Result<(), &'static str> {
    if state.signals.len() > MAX_BUFFERED_SIGNALS {
        return Err("more buffered signals than the buffer holds");
    }
    if state.status.is_terminal() {
        if state.position.is_some() || state.invocation.is_some() {
            return Err("a terminal state has no position and no invocation");
        }
        return match (state.status, &state.result) {
            (RunStatus::Completed, Some(RunResult::Output(_)))
            | (RunStatus::Failed, Some(RunResult::Error(_))) => Ok(()),
            _ => Err("a terminal state carries the result of its status"),
        };
    }

    if state.result.is_some() {
        return Err("only a terminal state has a result");
    }
    let Some(position) = &state.position else {
        return Err("a state that is not terminal has a position");
    };
    let Some((_, node)) = graph.node(&position.node_id) else {
        return Err("position names no node of the graph");
    };
    if position.visit == 0 || state.visits.get(&position.node_id) != Some(&position.visit) {
        return Err("position.visit is not the node's visit count");
    }
    if position.activation_id != ids::activation(&position.node_id, position.visit) {
        return Err("position.activationId is not <nodeId>/<visit>");
    }

    match (state.status, node, &state.invocation) {
        (RunStatus::Waiting, Node::AwaitSignal { .. }, None) => Ok(()),
        (RunStatus::Running, Node::Activity { action, .. }, Some(invocation))
            if invocation.state == InvocationPhase::Scheduled =>
        {
            owned_by(invocation, position, action, state)
        }
        (RunStatus::NeedsIntervention, Node::Activity { action, .. }, Some(invocation))
            if invocation.state == InvocationPhase::Unknown =>
        {
            owned_by(invocation, position, action, state)
        }
        _ => Err("status, position and invocation do not agree"),
    }
}

/// The invocation belongs to the activation the run is parked at.
fn owned_by(
    invocation: &InvocationState,
    position: &Position,
    action: &str,
    state: &State,
) -> Result<(), &'static str> {
    let derived = ids::invocation(&state.tenant, &state.run_id, &position.activation_id);
    if invocation.node_id != position.node_id
        || invocation.activation_id != position.activation_id
        || invocation.action_id != action
        || invocation.invocation_id != derived
    {
        return Err("invocation does not belong to the position's activation");
    }
    if invocation.attempt == 0 {
        return Err("invocation.attempt starts at 1");
    }
    Ok(())
}
