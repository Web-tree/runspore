//! Reducers for this crate's unit tests.
//!
//! [`Toy`] follows the outline of `spec/kernel.md` closely enough to return
//! real `State` snapshots, commands, diagnostics, and failures, which is what
//! the runner and the generator need to be exercised. It is not the kernel: it
//! evaluates no mappings, buffers no signals, and applies no backoff.

use std::collections::BTreeMap;

use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::model::{
    code, command_kind, event_kind, ActionPolicy, ActivityResult, AttemptStatus, ErrorInfo,
    InvocationPhase, InvocationResolved, InvocationState, Node, NodeResult, Position, Resolution,
    RetryActivity, RunResult, RunStarted, RunStatus, ScheduleActivity, SignalReceived, State,
    Workflow, ABI_VERSION, CODEC_VERSION, OUTCOME_FAILED, OUTCOME_OK, OUTCOME_RECEIVED,
    SEMANTICS_VERSION, STATE_FORMAT, WORKFLOW_FORMAT,
};
use runspore_types::reducer::{
    Command, Decision, Descriptor, Diagnostic, Failure, FailureKind, Reducer, TransitionRequest,
};
use serde_json::{json, Value};

use crate::format::{Event, Step, Trace};

pub(crate) struct Toy;

/// [`Toy`], with each result passed through a function before it is returned.
pub(crate) struct Altered<F>(pub(crate) F);

impl<F> Reducer for Altered<F>
where
    F: Fn(&TransitionRequest, &mut Result<Decision, Failure>) + Send + Sync,
{
    fn describe(&self) -> Descriptor {
        Toy.describe()
    }

    fn kernel_digest(&self) -> String {
        Toy.kernel_digest()
    }

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        let mut outcome = Toy.transition(request);
        (self.0)(request, &mut outcome);
        outcome
    }
}

impl Reducer for Toy {
    fn describe(&self) -> Descriptor {
        Descriptor {
            abi_version: ABI_VERSION.to_string(),
            semantics_version: SEMANTICS_VERSION.to_string(),
            graph_format_version: WORKFLOW_FORMAT.to_string(),
            codec_version: CODEC_VERSION.to_string(),
        }
    }

    fn kernel_digest(&self) -> String {
        "test:toy".to_string()
    }

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        if request.identity.semantics_version != SEMANTICS_VERSION {
            return Err(failure(
                FailureKind::IncompatibleVersion,
                code::VERSION_SEMANTICS,
            ));
        }
        let invalid = |code| failure(FailureKind::InvalidInput, code);
        let workflow: Workflow =
            canonical::decode(&request.graph).map_err(|_| invalid(code::WORKFLOW_INVALID))?;
        let event = &request.input_event;
        let mut turn = Turn {
            workflow: &workflow,
            commands: Vec::new(),
            diagnostics: Vec::new(),
        };

        let state = if event.kind == event_kind::RUN_STARTED {
            let started: RunStarted =
                canonical::decode(&event.payload).map_err(|_| invalid(code::EVENT_INVALID))?;
            if request.snapshot.is_some() {
                return Err(invalid(code::STATE_UNEXPECTED_START));
            }
            if event.sequence != 1 {
                return Err(invalid(code::EVENT_OUT_OF_ORDER));
            }
            let mut state = State {
                format: STATE_FORMAT.to_string(),
                semantics: SEMANTICS_VERSION.to_string(),
                tenant: started.tenant,
                run_id: started.run_id,
                status: RunStatus::Running,
                input: started.input,
                nodes: BTreeMap::new(),
                visits: BTreeMap::new(),
                activations: 0,
                position: None,
                invocation: None,
                signals: Vec::new(),
                result: None,
                last_sequence: 1,
                last_accepted_at_ms: event.accepted_at_ms,
            };
            turn.enter(&mut state, &workflow.start);
            state
        } else {
            let payload = Payload::decode(&event.kind, &event.payload)?;
            let snapshot = request
                .snapshot
                .as_ref()
                .ok_or_else(|| invalid(code::STATE_MISSING))?;
            let mut state: State =
                canonical::decode(snapshot).map_err(|_| invalid(code::STATE_INVALID))?;
            if event.sequence != state.last_sequence + 1 {
                return Err(invalid(code::EVENT_OUT_OF_ORDER));
            }
            state.last_sequence = event.sequence;
            state.last_accepted_at_ms = state.last_accepted_at_ms.max(event.accepted_at_ms);
            if state.status.is_terminal() {
                turn.diagnose(code::EVENT_IGNORED_TERMINAL, json!({"kind": event.kind}));
            } else {
                match payload {
                    Payload::Result(result) => turn.result(&mut state, result),
                    Payload::Signal(signal) => turn.signal(&mut state, signal),
                    Payload::Resolved(resolved) => turn.resolved(&mut state, resolved),
                }
            }
            state
        };

        let snapshot = canonical::encode(&state).unwrap();
        if snapshot.len() > request.frozen_limits.max_state_bytes as usize {
            return Err(failure(
                FailureKind::ResourceLimit,
                code::BUDGET_STATE_BYTES,
            ));
        }
        Ok(Decision {
            snapshot_digest: digest::state(&snapshot),
            snapshot,
            commands: turn.commands,
            diagnostics: turn.diagnostics,
        })
    }
}

fn failure(kind: FailureKind, code: &str) -> Failure {
    Failure {
        kind,
        code: code.to_string(),
        details: format!("toy reducer refused the request: {code}"),
    }
}

enum Payload {
    Result(ActivityResult),
    Signal(SignalReceived),
    Resolved(InvocationResolved),
}

impl Payload {
    fn decode(kind: &str, bytes: &[u8]) -> Result<Payload, Failure> {
        let invalid = |_| failure(FailureKind::InvalidInput, code::EVENT_INVALID);
        Ok(match kind {
            event_kind::ACTIVITY_RESULT => {
                Payload::Result(canonical::decode(bytes).map_err(invalid)?)
            }
            event_kind::SIGNAL_RECEIVED => {
                Payload::Signal(canonical::decode(bytes).map_err(invalid)?)
            }
            event_kind::INVOCATION_RESOLVED => {
                Payload::Resolved(canonical::decode(bytes).map_err(invalid)?)
            }
            _ => return Err(failure(FailureKind::InvalidInput, code::EVENT_UNKNOWN_KIND)),
        })
    }
}

struct Turn<'a> {
    workflow: &'a Workflow,
    commands: Vec<Command>,
    diagnostics: Vec<Diagnostic>,
}

impl Turn<'_> {
    fn diagnose(&mut self, code: &str, details: Value) {
        self.diagnostics.push(Diagnostic {
            code: code.to_string(),
            node_id: None,
            details: canonical::to_vec(&details).unwrap(),
        });
    }

    fn enter(&mut self, state: &mut State, node_id: &str) {
        let visit = state.visits.get(node_id).copied().unwrap_or(0) + 1;
        let limits = &self.workflow.limits;
        if visit > limits.max_visits_per_node || state.activations + 1 > limits.max_activations {
            finish(state, error(code::LIMIT_VISITS_EXCEEDED, node_id));
            return;
        }
        state.visits.insert(node_id.to_string(), visit);
        state.activations += 1;
        let activation_id = ids::activation(node_id, visit);
        let position = Position {
            node_id: node_id.to_string(),
            activation_id: activation_id.clone(),
            visit,
        };
        match &self.workflow.nodes[node_id] {
            Node::Activity { action, .. } => {
                let invocation_id = ids::invocation(&state.tenant, &state.run_id, &activation_id);
                let schedule = ScheduleActivity {
                    invocation_id: invocation_id.clone(),
                    activation_id: activation_id.clone(),
                    node_id: node_id.to_string(),
                    action_id: action.clone(),
                    effect_key: ids::effect_key(&invocation_id),
                    attempt: 1,
                    not_before_ms: state.last_accepted_at_ms,
                    input: Value::Null,
                };
                self.commands.push(Command {
                    command_id: ids::command(&state.tenant, &state.run_id, &activation_id, 1),
                    activation_id: activation_id.clone(),
                    kind: command_kind::ACTIVITY_SCHEDULE.to_string(),
                    payload: canonical::encode(&schedule).unwrap(),
                });
                state.status = RunStatus::Running;
                state.position = Some(position);
                state.invocation = Some(InvocationState {
                    invocation_id,
                    activation_id,
                    node_id: node_id.to_string(),
                    action_id: action.clone(),
                    attempt: 1,
                    state: InvocationPhase::Scheduled,
                    input_digest: digest::body(b"null"),
                });
            }
            Node::AwaitSignal { .. } => {
                state.status = RunStatus::Waiting;
                state.position = Some(position);
                state.invocation = None;
            }
            Node::Complete { .. } => {
                state.status = RunStatus::Completed;
                state.result = Some(RunResult::Output(Value::Null));
                state.position = None;
                state.invocation = None;
            }
            Node::Fail { error } => finish(state, error.clone()),
        }
    }

    /// Records the node's result and follows its route for `outcome`.
    fn route(&mut self, state: &mut State, outcome: &str, output: Value) {
        let Some(position) = state.position.clone() else {
            return;
        };
        let routes = match &self.workflow.nodes[&position.node_id] {
            Node::Activity { outcomes, .. } | Node::AwaitSignal { outcomes, .. } => outcomes,
            _ => return,
        };
        state.invocation = None;
        match routes.get(outcome) {
            Some(target) => {
                state.nodes.insert(
                    position.node_id,
                    NodeResult {
                        visit: position.visit,
                        outcome: outcome.to_string(),
                        output,
                    },
                );
                self.enter(state, target);
            }
            None => finish(state, error(code::ACTIVITY_FAILED, &position.node_id)),
        }
    }

    fn retry(&mut self, state: &mut State) {
        let Some(invocation) = state.invocation.as_mut() else {
            return;
        };
        invocation.attempt += 1;
        invocation.state = InvocationPhase::Scheduled;
        state.status = RunStatus::Running;
        let retry = RetryActivity {
            invocation_id: invocation.invocation_id.clone(),
            attempt: invocation.attempt,
            not_before_ms: state.last_accepted_at_ms,
        };
        self.commands.push(Command {
            command_id: ids::command(
                &state.tenant,
                &state.run_id,
                &invocation.activation_id,
                invocation.attempt,
            ),
            activation_id: invocation.activation_id.clone(),
            kind: command_kind::ACTIVITY_RETRY.to_string(),
            payload: canonical::encode(&retry).unwrap(),
        });
        let attempt = invocation.attempt;
        self.diagnose(code::ACTIVITY_RETRY_SCHEDULED, json!({"attempt": attempt}));
    }

    fn result(&mut self, state: &mut State, result: ActivityResult) {
        let outstanding = state.invocation.clone().filter(|invocation| {
            invocation.invocation_id == result.invocation_id
                && invocation.attempt == result.attempt
                && invocation.state == InvocationPhase::Scheduled
        });
        let Some(invocation) = outstanding else {
            self.diagnose(
                code::RESULT_STALE,
                json!({"invocationId": result.invocation_id, "attempt": result.attempt}),
            );
            return;
        };
        let policy: ActionPolicy =
            serde_json::from_value(self.workflow.actions[&invocation.action_id].clone()).unwrap();
        let attempts_left = invocation.attempt < policy.retry.max_attempts;
        match result.status {
            AttemptStatus::Success => {
                let outcome = result.outcome.unwrap_or_else(|| OUTCOME_OK.to_string());
                self.route(state, &outcome, result.output);
            }
            AttemptStatus::Failure if result.retryable && attempts_left => self.retry(state),
            AttemptStatus::Failure => self.route(state, OUTCOME_FAILED, Value::Null),
            _ if !policy.effect.repeatable() => {
                if let Some(invocation) = state.invocation.as_mut() {
                    invocation.state = InvocationPhase::Unknown;
                }
                state.status = RunStatus::NeedsIntervention;
                self.diagnose(
                    code::INVOCATION_UNKNOWN,
                    json!({"invocationId": result.invocation_id, "attempt": result.attempt}),
                );
            }
            _ if attempts_left => self.retry(state),
            _ => self.route(state, OUTCOME_FAILED, Value::Null),
        }
    }

    fn signal(&mut self, state: &mut State, signal: SignalReceived) {
        let awaited = state.position.as_ref().is_some_and(|position| {
            match &self.workflow.nodes[&position.node_id] {
                Node::AwaitSignal {
                    signal: name,
                    outcomes,
                } => {
                    *name == signal.name
                        && outcomes
                            .contains_key(signal.outcome.as_deref().unwrap_or(OUTCOME_RECEIVED))
                }
                _ => false,
            }
        });
        if awaited {
            let outcome = signal
                .outcome
                .unwrap_or_else(|| OUTCOME_RECEIVED.to_string());
            self.route(state, &outcome, signal.data);
        } else {
            self.diagnose(code::SIGNAL_OUTCOME_UNROUTED, json!({"name": signal.name}));
        }
    }

    fn resolved(&mut self, state: &mut State, resolved: InvocationResolved) {
        let applies = state.invocation.as_ref().is_some_and(|invocation| {
            invocation.invocation_id == resolved.invocation_id
                && invocation.state == InvocationPhase::Unknown
        });
        if !applies {
            self.diagnose(
                code::RESOLVE_NOT_APPLICABLE,
                json!({"invocationId": resolved.invocation_id}),
            );
            return;
        }
        match resolved.resolution {
            Resolution::Complete { outcome, output } => self.route(state, &outcome, output),
            Resolution::Retry => self.retry(state),
            Resolution::Fail { .. } => self.route(state, OUTCOME_FAILED, Value::Null),
        }
    }
}

fn error(code: &str, node_id: &str) -> ErrorInfo {
    ErrorInfo {
        code: code.to_string(),
        message: "toy".to_string(),
        node_id: Some(node_id.to_string()),
        details: Value::Null,
    }
}

fn finish(state: &mut State, error: ErrorInfo) {
    state.status = RunStatus::Failed;
    state.result = Some(RunResult::Error(error));
    state.position = None;
    state.invocation = None;
}

pub(crate) const TENANT: &str = "default";
pub(crate) const RUN_ID: &str = "run_toy";

/// A build that may loop, an approval, and an unsafe shipment.
pub(crate) fn workflow() -> Value {
    json!({
        "format": "runspore.workflow/0.1",
        "name": "toy",
        "start": "build",
        "actions": {
            "build": {"effect": "idempotent", "outcomes": ["ok", "red"],
                      "retry": {"maxAttempts": 2}},
            "ship": {"effect": "unsafe"}
        },
        "nodes": {
            "build": {"kind": "activity", "action": "build",
                      "outcomes": {"ok": "approve", "red": "build", "failed": "broken"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "ship", "rejected": "build"}},
            "ship": {"kind": "activity", "action": "ship", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"},
            "broken": {"kind": "fail",
                       "error": {"code": "build.broken", "message": "never passed"}}
        }
    })
}

pub(crate) fn invocation(activation_id: &str) -> String {
    ids::invocation(TENANT, RUN_ID, activation_id)
}

/// A seven-step run of [`workflow`] with no expectations yet: start, build
/// succeeds, approval arrives, an unknown event kind is refused, the shipment
/// ends unknown, an operator completes it, and a late signal is ignored.
pub(crate) fn unblessed() -> Trace {
    let build = invocation("build/1");
    let ship = invocation("ship/1");
    let mut trace = Trace::new("toy-run", "a run of the toy workflow", workflow());
    let events = [
        Event::new(
            "start",
            1,
            1_000,
            event_kind::RUN_STARTED,
            json!({"tenant": TENANT, "runId": RUN_ID, "input": {"repo": "r"}}),
        ),
        Event::new(
            "build-ok",
            2,
            1_500,
            event_kind::ACTIVITY_RESULT,
            json!({"invocationId": build, "attemptId": ids::attempt(&build, 1), "attempt": 1,
                   "status": "success", "output": {"sha": "abc"}}),
        ),
        Event::new(
            "approved",
            3,
            2_000,
            event_kind::SIGNAL_RECEIVED,
            json!({"name": "approval", "outcome": "approved"}),
        ),
        Event::new("timer", 4, 2_100, "timer.fired", json!({})),
        Event::new(
            "ship-unknown",
            4,
            2_500,
            event_kind::ACTIVITY_RESULT,
            json!({"invocationId": ship, "attemptId": ids::attempt(&ship, 1), "attempt": 1,
                   "status": "unknown"}),
        ),
        Event::new(
            "operator",
            5,
            3_000,
            event_kind::INVOCATION_RESOLVED,
            json!({"invocationId": ship, "resolution": {"action": "complete", "outcome": "ok"}}),
        ),
        Event::new(
            "late",
            6,
            3_500,
            event_kind::SIGNAL_RECEIVED,
            json!({"name": "approval"}),
        ),
    ];
    trace.steps = events.into_iter().map(Step::new).collect();
    trace
}
