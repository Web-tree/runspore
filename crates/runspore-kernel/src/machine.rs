//! Dispatch and procedures: `spec/kernel.md` 7.2–7.5.
//!
//! A [`Machine`] owns a working copy of the state. A failure drops the machine, so
//! nothing of a failed transition survives.

use std::collections::BTreeMap;

use runspore_types::digest::{self, ids};
use runspore_types::model::{
    code, command_kind, u64_str, ActionPolicy, ActivityResult, AttemptStatus, BufferedSignal,
    ErrorInfo, InvocationPhase, InvocationResolved, InvocationState, Node, NodeResult, Position,
    Resolution, RetryActivity, RunResult, RunStatus, ScheduleActivity, SignalReceived, State,
    MAX_BUFFERED_SIGNALS, OUTCOME_FAILED, OUTCOME_OK, OUTCOME_RECEIVED,
};
use runspore_types::reducer::{Command, Decision, Diagnostic, Envelope, Failure, Limits};
use serde::Serialize;
use serde_json::Value;

use crate::event::Event;
use crate::failure::{self, object};
use crate::graph::Graph;
use crate::mapping::{self, Context, MissingPath};

const MESSAGE_VISITS_EXCEEDED: &str = "node visit limit exceeded";
const MESSAGE_ACTIVATIONS_EXCEEDED: &str = "run activation limit exceeded";
const MESSAGE_MISSING_PATH: &str = "mapping path does not resolve";
const MESSAGE_UNDECLARED_OUTCOME: &str = "activity reported an undeclared outcome";
const MESSAGE_ATTEMPTS_EXHAUSTED: &str = "activity attempts exhausted";
const MESSAGE_ACTIVITY_FAILED: &str = "activity failed";

/// `reason` values of a `resolve.not-applicable` diagnostic.
const REASON_MISMATCH: &str = "invocation-mismatch";
const REASON_NOT_UNKNOWN: &str = "invocation-not-unknown";
const REASON_OUTCOME_UNDECLARED: &str = "outcome-undeclared";

/// The activity the run is parked at: its node, the visit that owns the outstanding
/// invocation, its routes, and its action's policy.
struct Activity<'a> {
    node_id: &'a str,
    visit: u32,
    outcomes: &'a BTreeMap<String, String>,
    policy: &'a ActionPolicy,
}

/// The await-signal node the run is parked at: its node, the visit, the signal name
/// it waits for, and its routes.
struct Waiter<'a> {
    node_id: &'a str,
    visit: u32,
    signal: &'a str,
    outcomes: &'a BTreeMap<String, String>,
}

pub(crate) struct Machine<'a> {
    graph: &'a Graph,
    limits: Limits,
    state: State,
    commands: Vec<Command>,
    diagnostics: Vec<Diagnostic>,
    microsteps: u64,
    operations: u64,
}

impl<'a> Machine<'a> {
    pub(crate) fn new(graph: &'a Graph, limits: Limits, state: State) -> Self {
        Self {
            graph,
            limits,
            state,
            commands: Vec::new(),
            diagnostics: Vec::new(),
            microsteps: 0,
            operations: 0,
        }
    }

    /// Bookkeeping (7.2), then dispatch by event kind (7.3). A terminal state
    /// consumes every event without any other change.
    pub(crate) fn apply(&mut self, envelope: &Envelope, event: Event) -> Result<(), Failure> {
        self.state.last_sequence = envelope.sequence;
        self.state.last_accepted_at_ms =
            self.state.last_accepted_at_ms.max(envelope.accepted_at_ms);

        if self.state.status.is_terminal() {
            let details = object([("kind", Value::String(envelope.kind.clone()))]);
            return self.diagnose(code::EVENT_IGNORED_TERMINAL, None, details);
        }
        match event {
            Event::Started(_) => self.enter(self.graph.start()),
            Event::Signal(signal) => self.on_signal(envelope, signal),
            Event::Result(result) => self.on_result(result),
            Event::Resolved(resolved) => self.on_resolved(resolved),
        }
    }

    /// Result checks (7.5), in order: command count, state bytes, expression operations.
    pub(crate) fn finish(self) -> Result<Decision, Failure> {
        let command_count = u64::try_from(self.commands.len()).unwrap_or(u64::MAX);
        if command_count > u64::from(self.limits.max_command_count) {
            return Err(failure::resource(
                code::BUDGET_COMMAND_COUNT,
                format!(
                    "{command_count} commands exceed the budget of {}",
                    self.limits.max_command_count
                ),
            ));
        }
        let snapshot = failure::encode(&self.state)?;
        let state_bytes = u64::try_from(snapshot.len()).unwrap_or(u64::MAX);
        if state_bytes > u64::from(self.limits.max_state_bytes) {
            return Err(failure::resource(
                code::BUDGET_STATE_BYTES,
                format!(
                    "snapshot of {state_bytes} bytes exceeds the budget of {}",
                    self.limits.max_state_bytes
                ),
            ));
        }
        if self.operations > u64::from(self.limits.expression_operations) {
            return Err(failure::resource(
                code::BUDGET_EXPRESSION_OPERATIONS,
                format!(
                    "{} expression operations exceed the budget of {}",
                    self.operations, self.limits.expression_operations
                ),
            ));
        }
        Ok(Decision {
            snapshot_digest: digest::state(&snapshot),
            snapshot,
            commands: self.commands,
            diagnostics: self.diagnostics,
        })
    }

    /// A signal the parked waiter waits for is taken directly, so a full buffer never
    /// starves it. Any other signal is appended, or dropped when the buffer is full.
    fn on_signal(&mut self, envelope: &Envelope, signal: SignalReceived) -> Result<(), Failure> {
        let arrived = BufferedSignal {
            event_id: envelope.event_id.clone(),
            sequence: envelope.sequence,
            name: signal.name,
            outcome: signal.outcome,
            data: signal.data,
        };
        if self.state.status == RunStatus::Waiting {
            let waiter = self.waiter()?;
            if arrived.name == waiter.signal {
                return match self.take(&waiter, arrived)? {
                    Some(target) => self.enter(target),
                    None => Ok(()),
                };
            }
        }
        if self.state.signals.len() >= MAX_BUFFERED_SIGNALS {
            let details = object([("eventId", Value::String(arrived.event_id))]);
            return self.diagnose(code::SIGNAL_BUFFER_OVERFLOW, None, details);
        }
        self.state.signals.push(arrived);
        Ok(())
    }

    /// A result applies only to the attempt the state authorizes; anything else is stale.
    fn on_result(&mut self, result: ActivityResult) -> Result<(), Failure> {
        let current = self
            .state
            .invocation
            .as_ref()
            .filter(|invocation| invocation.invocation_id == result.invocation_id);
        let applies = current.is_some_and(|invocation| {
            invocation.attempt == result.attempt && invocation.state == InvocationPhase::Scheduled
        });
        if !applies {
            let node = current.map(|invocation| invocation.node_id.clone());
            let details = object([
                ("invocationId", Value::String(result.invocation_id)),
                ("attempt", Value::from(result.attempt)),
            ]);
            return self.diagnose(code::RESULT_STALE, node.as_deref(), details);
        }

        let activity = self.activity()?;
        let retry = &activity.policy.retry;
        match result.status {
            AttemptStatus::Success => {
                let outcome = result.outcome.unwrap_or_else(|| OUTCOME_OK.to_string());
                if activity.policy.outcomes.contains(&outcome) {
                    self.complete(&activity, outcome, result.output)
                } else {
                    let details = object([("outcome", Value::String(outcome))]);
                    self.fail_invocation(
                        &activity,
                        kernel_error(
                            code::ACTIVITY_UNDECLARED_OUTCOME,
                            MESSAGE_UNDECLARED_OUTCOME,
                            details,
                        ),
                    )
                }
            }
            AttemptStatus::Failure => {
                if result.retryable && result.attempt < retry.max_attempts {
                    self.retry(&activity, retry.delay_after(result.attempt))
                } else {
                    let error = result.error.unwrap_or_else(|| {
                        kernel_error(code::ACTIVITY_FAILED, MESSAGE_ACTIVITY_FAILED, Value::Null)
                    });
                    self.fail_invocation(&activity, error)
                }
            }
            AttemptStatus::Unknown | AttemptStatus::Expired => {
                let status = Value::String(status_name(result.status).to_string());
                if !activity.policy.effect.repeatable() {
                    if let Some(invocation) = self.state.invocation.as_mut() {
                        invocation.state = InvocationPhase::Unknown;
                    }
                    self.state.status = RunStatus::NeedsIntervention;
                    let details = object([
                        ("invocationId", Value::String(result.invocation_id)),
                        ("attempt", Value::from(result.attempt)),
                        ("status", status),
                    ]);
                    self.diagnose(code::INVOCATION_UNKNOWN, Some(activity.node_id), details)
                } else if result.attempt < retry.max_attempts {
                    self.retry(&activity, retry.delay_after(result.attempt))
                } else {
                    self.fail_invocation(
                        &activity,
                        kernel_error(
                            code::ACTIVITY_ATTEMPTS_EXHAUSTED,
                            MESSAGE_ATTEMPTS_EXHAUSTED,
                            object([("lastStatus", status)]),
                        ),
                    )
                }
            }
        }
    }

    /// A resolution applies only to the current invocation while its outcome is unknown.
    fn on_resolved(&mut self, resolved: InvocationResolved) -> Result<(), Failure> {
        let current = self
            .state
            .invocation
            .as_ref()
            .filter(|invocation| invocation.invocation_id == resolved.invocation_id);
        let refusal = match current {
            None => Some(REASON_MISMATCH),
            Some(invocation) if invocation.state != InvocationPhase::Unknown => {
                Some(REASON_NOT_UNKNOWN)
            }
            Some(_) => None,
        };
        if let Some(reason) = refusal {
            let node = current.map(|invocation| invocation.node_id.clone());
            return self.not_applicable(resolved.invocation_id, node.as_deref(), reason);
        }

        let activity = self.activity()?;
        match resolved.resolution {
            Resolution::Complete { outcome, output } => {
                if activity.policy.outcomes.contains(&outcome) {
                    self.complete(&activity, outcome, output)
                } else {
                    self.not_applicable(
                        resolved.invocation_id,
                        Some(activity.node_id),
                        REASON_OUTCOME_UNDECLARED,
                    )
                }
            }
            Resolution::Retry => self.retry(&activity, 0),
            Resolution::Fail { error } => self.fail_invocation(&activity, error),
        }
    }

    fn not_applicable(
        &mut self,
        invocation_id: String,
        node_id: Option<&str>,
        reason: &str,
    ) -> Result<(), Failure> {
        let details = object([
            ("invocationId", Value::String(invocation_id)),
            ("reason", Value::String(reason.to_string())),
        ]);
        self.diagnose(code::RESOLVE_NOT_APPLICABLE, node_id, details)
    }

    /// Enter (7.4): admit the node against the graph limits, count the visit, then
    /// act by node kind. Only a consumed signal continues the loop.
    fn enter(&mut self, first: &'a str) -> Result<(), Failure> {
        let graph = self.graph;
        let mut target = first;
        loop {
            self.count_microstep()?;
            let Some((node_id, node)) = graph.node(target) else {
                return Err(failure::invariant(format!(
                    "route to {target:?}, which names no node"
                )));
            };

            let visits = self.state.visits.get(node_id).copied().unwrap_or(0);
            if visits >= graph.limits().max_visits_per_node {
                self.terminate(node_error(
                    code::LIMIT_VISITS_EXCEEDED,
                    MESSAGE_VISITS_EXCEEDED,
                    node_id,
                    Value::Null,
                ));
                return Ok(());
            }
            if self.state.activations >= graph.limits().max_activations {
                self.terminate(node_error(
                    code::LIMIT_ACTIVATIONS_EXCEEDED,
                    MESSAGE_ACTIVATIONS_EXCEEDED,
                    node_id,
                    Value::Null,
                ));
                return Ok(());
            }
            let (Some(visit), Some(activations)) =
                (visits.checked_add(1), self.state.activations.checked_add(1))
            else {
                return Err(failure::invariant("visit counter beyond the graph limits"));
            };
            self.state.visits.insert(node_id.to_string(), visit);
            self.state.activations = activations;
            let position = Position {
                node_id: node_id.to_string(),
                activation_id: ids::activation(node_id, visit),
                visit,
            };

            match node {
                Node::Activity { action, input, .. } => {
                    return match self.evaluate(input, node_id) {
                        Some(input) => self.schedule(position, action, input),
                        None => Ok(()),
                    };
                }
                Node::AwaitSignal { .. } => {
                    self.state.position = Some(position);
                    self.state.status = RunStatus::Waiting;
                    self.state.invocation = None;
                    match self.consume()? {
                        Some(next) => target = next,
                        None => return Ok(()),
                    }
                }
                Node::Complete { output } => {
                    if let Some(output) = self.evaluate(output, node_id) {
                        self.state.status = RunStatus::Completed;
                        self.state.result = Some(RunResult::Output(output));
                        self.state.position = None;
                    }
                    return Ok(());
                }
                Node::Fail { error } => {
                    let mut error = error.clone();
                    error.node_id = Some(node_id.to_string());
                    self.terminate(error);
                    return Ok(());
                }
            }
        }
    }

    /// Creates the invocation of a freshly entered activity and authorizes attempt 1.
    fn schedule(&mut self, position: Position, action: &str, input: Value) -> Result<(), Failure> {
        let input_digest = digest::body(&failure::to_vec(&input)?);
        let invocation_id = ids::invocation(
            &self.state.tenant,
            &self.state.run_id,
            &position.activation_id,
        );
        let payload = ScheduleActivity {
            invocation_id: invocation_id.clone(),
            activation_id: position.activation_id.clone(),
            node_id: position.node_id.clone(),
            action_id: action.to_string(),
            effect_key: ids::effect_key(&invocation_id),
            attempt: 1,
            not_before_ms: self.state.last_accepted_at_ms,
            input,
        };
        self.command(
            &position.activation_id,
            1,
            command_kind::ACTIVITY_SCHEDULE,
            &payload,
        )?;
        self.state.invocation = Some(InvocationState {
            invocation_id,
            activation_id: position.activation_id.clone(),
            node_id: position.node_id.clone(),
            action_id: action.to_string(),
            attempt: 1,
            state: InvocationPhase::Scheduled,
            input_digest,
        });
        self.state.position = Some(position);
        self.state.status = RunStatus::Running;
        Ok(())
    }

    /// Consume (7.4), at the await-signal node the run is parked at: take matching
    /// signals earliest first until one routes. Returns the node to enter, if any.
    fn consume(&mut self) -> Result<Option<&'a str>, Failure> {
        let waiter = self.waiter()?;
        loop {
            let Some(index) = self
                .state
                .signals
                .iter()
                .position(|buffered| buffered.name == waiter.signal)
            else {
                return Ok(None);
            };
            let taken = self.state.signals.remove(index);
            if let Some(target) = self.take(&waiter, taken)? {
                return Ok(Some(target));
            }
        }
    }

    /// The await-signal node a waiting run is parked at.
    fn waiter(&self) -> Result<Waiter<'a>, Failure> {
        let graph = self.graph;
        let Some(position) = &self.state.position else {
            return Err(failure::invariant("a waiting run has no position"));
        };
        let Some((node_id, Node::AwaitSignal { signal, outcomes })) = graph.node(&position.node_id)
        else {
            return Err(failure::invariant(
                "a waiting run is not at an await-signal node",
            ));
        };
        Ok(Waiter {
            node_id,
            visit: position.visit,
            signal,
            outcomes,
        })
    }

    /// Takes one signal at the waiter for one microstep: it either records the node's
    /// result and returns the route, or is dropped with `signal.outcome-unrouted`.
    fn take(
        &mut self,
        waiter: &Waiter<'a>,
        taken: BufferedSignal,
    ) -> Result<Option<&'a str>, Failure> {
        self.count_microstep()?;
        let outcome = taken
            .outcome
            .unwrap_or_else(|| OUTCOME_RECEIVED.to_string());
        let Some(target) = waiter.outcomes.get(&outcome) else {
            let details = object([
                ("eventId", Value::String(taken.event_id)),
                ("outcome", Value::String(outcome)),
            ]);
            self.diagnose(code::SIGNAL_OUTCOME_UNROUTED, Some(waiter.node_id), details)?;
            return Ok(None);
        };
        self.state.nodes.insert(
            waiter.node_id.to_string(),
            NodeResult {
                visit: waiter.visit,
                outcome,
                output: taken.data,
            },
        );
        Ok(Some(target.as_str()))
    }

    /// A success: record the node's result, clear the invocation, follow the route.
    fn complete(
        &mut self,
        activity: &Activity<'a>,
        outcome: String,
        output: Value,
    ) -> Result<(), Failure> {
        let Some(target) = activity.outcomes.get(&outcome) else {
            return Err(failure::invariant(format!(
                "declared outcome {outcome:?} has no route"
            )));
        };
        self.state.nodes.insert(
            activity.node_id.to_string(),
            NodeResult {
                visit: activity.visit,
                outcome,
                output,
            },
        );
        self.state.invocation = None;
        self.enter(target)
    }

    /// Retry (7.4): authorize the next attempt of the same invocation. The delay
    /// counts from logical now.
    fn retry(&mut self, activity: &Activity<'a>, delay_ms: u64) -> Result<(), Failure> {
        let now = self.state.last_accepted_at_ms;
        let Some(not_before_ms) = now
            .checked_add(delay_ms)
            .filter(|time| *time <= u64_str::MAX)
        else {
            return Err(failure::resource(
                crate::code::BUDGET_COUNTER_RANGE,
                "retry time exceeds 2^63 - 1",
            ));
        };
        let Some(invocation) = self.state.invocation.as_mut() else {
            return Err(failure::invariant("retry without an invocation"));
        };
        let Some(attempt) = invocation.attempt.checked_add(1) else {
            return Err(failure::resource(
                crate::code::BUDGET_COUNTER_RANGE,
                "attempt number exceeds its range",
            ));
        };
        invocation.attempt = attempt;
        invocation.state = InvocationPhase::Scheduled;
        let payload = RetryActivity {
            invocation_id: invocation.invocation_id.clone(),
            attempt,
            not_before_ms,
        };
        let activation_id = invocation.activation_id.clone();
        self.state.status = RunStatus::Running;
        self.command(
            &activation_id,
            attempt,
            command_kind::ACTIVITY_RETRY,
            &payload,
        )?;
        let details = object([("attempt", Value::from(attempt))]);
        self.diagnose(
            code::ACTIVITY_RETRY_SCHEDULED,
            Some(activity.node_id),
            details,
        )
    }

    /// Fail the invocation (7.4): take the node's `failed` route when it has one,
    /// otherwise end the run with the error.
    fn fail_invocation(
        &mut self,
        activity: &Activity<'a>,
        mut error: ErrorInfo,
    ) -> Result<(), Failure> {
        error.node_id = Some(activity.node_id.to_string());
        self.state.invocation = None;
        let Some(target) = activity.outcomes.get(OUTCOME_FAILED) else {
            self.terminate(error);
            return Ok(());
        };
        let error = serde_json::to_value(&error)
            .map_err(|cause| failure::invariant(format!("error is not JSON: {cause}")))?;
        self.state.nodes.insert(
            activity.node_id.to_string(),
            NodeResult {
                visit: activity.visit,
                outcome: OUTCOME_FAILED.to_string(),
                output: object([("error", error)]),
            },
        );
        self.enter(target)
    }

    /// Terminal `failed`: the error becomes the result; position and invocation clear.
    fn terminate(&mut self, error: ErrorInfo) {
        self.state.status = RunStatus::Failed;
        self.state.result = Some(RunResult::Error(error));
        self.state.position = None;
        self.state.invocation = None;
    }

    /// Evaluates a node's mapping. An unresolved path ends the run and yields `None`.
    fn evaluate(&mut self, mapping: &Value, node_id: &str) -> Option<Value> {
        let context = Context {
            input: &self.state.input,
            nodes: &self.state.nodes,
            run_id: &self.state.run_id,
            tenant: &self.state.tenant,
        };
        match mapping::evaluate(mapping, &context, &mut self.operations) {
            Ok(value) => Some(value),
            Err(MissingPath(path)) => {
                self.terminate(node_error(
                    code::MAPPING_MISSING_PATH,
                    MESSAGE_MISSING_PATH,
                    node_id,
                    object([("path", path)]),
                ));
                None
            }
        }
    }

    fn activity(&self) -> Result<Activity<'a>, Failure> {
        let graph = self.graph;
        let Some(position) = &self.state.position else {
            return Err(failure::invariant("an invocation without a position"));
        };
        let Some((
            node_id,
            Node::Activity {
                action, outcomes, ..
            },
        )) = graph.node(&position.node_id)
        else {
            return Err(failure::invariant("an invocation outside an activity node"));
        };
        let Some(policy) = graph.policy(action) else {
            return Err(failure::invariant("an activity without an action policy"));
        };
        Ok(Activity {
            node_id,
            visit: position.visit,
            outcomes,
            policy,
        })
    }

    fn count_microstep(&mut self) -> Result<(), Failure> {
        self.microsteps = self.microsteps.saturating_add(1);
        if self.microsteps > u64::from(self.limits.microsteps) {
            return Err(failure::resource(
                code::BUDGET_MICROSTEPS,
                format!(
                    "more than {} microsteps in one transition",
                    self.limits.microsteps
                ),
            ));
        }
        Ok(())
    }

    /// `ordinal` is the attempt the command authorizes.
    fn command<T: Serialize>(
        &mut self,
        activation_id: &str,
        ordinal: u32,
        kind: &str,
        payload: &T,
    ) -> Result<(), Failure> {
        let command_id = ids::command(
            &self.state.tenant,
            &self.state.run_id,
            activation_id,
            ordinal,
        );
        self.commands.push(Command {
            command_id,
            activation_id: activation_id.to_string(),
            kind: kind.to_string(),
            payload: failure::encode(payload)?,
        });
        Ok(())
    }

    /// `node_id` is the position's node when the diagnostic concerns it.
    fn diagnose(
        &mut self,
        code: &str,
        node_id: Option<&str>,
        details: Value,
    ) -> Result<(), Failure> {
        self.diagnostics.push(Diagnostic {
            code: code.to_string(),
            node_id: node_id.map(str::to_string),
            details: failure::to_vec(&details)?,
        });
        Ok(())
    }
}

fn kernel_error(code: &str, message: &str, details: Value) -> ErrorInfo {
    ErrorInfo {
        code: code.to_string(),
        message: message.to_string(),
        node_id: None,
        details,
    }
}

fn node_error(code: &str, message: &str, node_id: &str, details: Value) -> ErrorInfo {
    ErrorInfo {
        node_id: Some(node_id.to_string()),
        ..kernel_error(code, message, details)
    }
}

fn status_name(status: AttemptStatus) -> &'static str {
    match status {
        AttemptStatus::Success => "success",
        AttemptStatus::Failure => "failure",
        AttemptStatus::Unknown => "unknown",
        AttemptStatus::Expired => "expired",
    }
}
