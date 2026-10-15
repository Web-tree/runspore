//! What a run's committed transitions changed, for a person watching it work
//! (`spec/cli.md`, Progress).
//!
//! The kernel's state records no changes, only where a run stands. The changes of one
//! transition are found from the snapshot before it, the snapshot after it, and the
//! event it consumed, the way [`crate::arrival`] finds arrows. A worker reports them
//! as it commits; a reader of the store can find the same ones by replaying a run's
//! events. This module only reads; it decides nothing.

use std::collections::HashMap;

use runspore_types::model::{
    event_kind, ActivityResult, AttemptStatus, ErrorInfo, InvocationPhase, InvocationResolved,
    Node, OnInterventionFailed, Resolution, RunResult, RunStatus, SignalReceived, State, Workflow,
};
use runspore_types::store::RunKey;
use serde_json::Value;

use crate::arrival::taken;

/// One change of one run, as a worker committed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub key: RunKey,
    pub start_key: String,
    /// Wall-clock time of the commit, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub change: Change,
}

/// A change in a run's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The kernel authorized a try of the step at `node_id`.
    StepStarted { node_id: String, attempt: u32 },
    /// A try of the step ended without an outcome. `message` is the first line of the
    /// error's message.
    TryFailed {
        node_id: String,
        attempt: u32,
        code: String,
        message: String,
    },
    /// The node returned `outcome` and the run took the arrow `arrow` (the outcome, or
    /// `exhausted` when it used up a loop limit) to `to`.
    Outcome {
        node_id: String,
        outcome: String,
        arrow: String,
        to: String,
    },
    /// The run waits at an await-signal node for the signal `signal`.
    Waiting { node_id: String, signal: String },
    /// The run waits for a person: a stuck step, or an unsafe step whose outcome is
    /// unknown.
    NeedsIntervention { node_id: String, reason: String },
    /// The run completed, at its complete node when known.
    Completed { node_id: Option<String> },
    /// The run failed with `code` and the first line of its message, at the node the
    /// error names when it names one.
    Failed {
        node_id: Option<String>,
        code: String,
        message: String,
    },
    /// The worker quarantined the run.
    Quarantined { code: String, details: String },
}

impl Change {
    /// The change's name in JSON: `step-started`, `try-failed`, `outcome`, `waiting`,
    /// `needs-intervention`, `completed`, `failed` or `quarantined`.
    pub fn name(&self) -> &'static str {
        match self {
            Change::StepStarted { .. } => "step-started",
            Change::TryFailed { .. } => "try-failed",
            Change::Outcome { .. } => "outcome",
            Change::Waiting { .. } => "waiting",
            Change::NeedsIntervention { .. } => "needs-intervention",
            Change::Completed { .. } => "completed",
            Change::Failed { .. } => "failed",
            Change::Quarantined { .. } => "quarantined",
        }
    }
}

/// The first line of `text`, without the line break.
pub fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

/// What one consumed event was about, for a person reading a run's events
/// (`spec/cli.md`, `events`), as Temporal's history shows each event's attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Happened {
    /// The node the event was for: the start node of `run.started`, the step of the
    /// invocation a result or a resolution names, the node a taken signal routed, the
    /// step an `onIntervention` action told about. `None` for a buffered signal.
    pub node_id: Option<String>,
    /// The outcome the event carried: a result's outcome, or its status when it has
    /// none (`failure`, `unknown`, `expired`); a signal's outcome; a resolution's
    /// action, or its outcome when it completes.
    pub outcome: Option<String>,
    /// The first line of the event's error as `code: message`, or else of its output or
    /// data as JSON, cut to [`DETAIL_CHARS`] characters.
    pub detail: Option<String>,
    /// What the transition that consumed the event changed.
    pub changes: Vec<Change>,
}

/// The most characters [`Happened::detail`] keeps of a line.
pub const DETAIL_CHARS: usize = 100;

/// What the event of `kind` with `payload` was about; `before` and `after` are the
/// snapshots around the transition that consumed it, and `nodes` the step of every
/// invocation the run had until then, by invocation ID.
pub fn happened(
    workflow: &Workflow,
    before: Option<&State>,
    after: &State,
    kind: &str,
    payload: &[u8],
    nodes: &HashMap<String, String>,
) -> Happened {
    let step = |id: &str| nodes.get(id).cloned();
    let error = |e: &ErrorInfo| format!("{}: {}", e.code, first_line(&e.message));
    let (node_id, outcome, detail) = match kind {
        event_kind::RUN_STARTED => (Some(workflow.start.clone()), None, None),
        event_kind::ACTIVITY_RESULT => match serde_json::from_slice::<ActivityResult>(payload) {
            Ok(r) => {
                let outcome = match r.status {
                    AttemptStatus::Success => r.outcome.unwrap_or_else(|| "ok".to_string()),
                    other => status(other).to_string(),
                };
                let detail = r.error.as_ref().map(error).or_else(|| data(&r.output));
                (step(&r.invocation_id), Some(outcome), detail)
            }
            Err(_) => (None, None, None),
        },
        event_kind::SIGNAL_RECEIVED => match serde_json::from_slice::<SignalReceived>(payload) {
            Ok(s) => {
                let taken = before
                    .filter(|b| b.status == RunStatus::Waiting)
                    .and_then(|b| b.position.as_ref())
                    .filter(|p| {
                        matches!(workflow.nodes.get(&p.node_id),
                                 Some(Node::AwaitSignal { signal, .. }) if *signal == s.name)
                    })
                    .map(|p| p.node_id.clone());
                let outcome = s.outcome.unwrap_or_else(|| "received".to_string());
                (taken, Some(outcome), data(&s.data))
            }
            Err(_) => (None, None, None),
        },
        event_kind::INVOCATION_RESOLVED => {
            match serde_json::from_slice::<InvocationResolved>(payload) {
                Ok(r) => {
                    let (outcome, detail) = match &r.resolution {
                        Resolution::Complete { outcome, output } => {
                            (format!("complete {outcome}"), data(output))
                        }
                        Resolution::Retry => ("retry".to_string(), None),
                        Resolution::Fail { error: e } => ("fail".to_string(), Some(error(e))),
                    };
                    (step(&r.invocation_id), Some(outcome), detail)
                }
                Err(_) => (None, None, None),
            }
        }
        event_kind::ON_INTERVENTION_FAILED => {
            match serde_json::from_slice::<OnInterventionFailed>(payload) {
                Ok(f) => {
                    let node = before.and_then(|b| {
                        b.invocation
                            .as_ref()
                            .map(|i| i.node_id.clone())
                            .or_else(|| b.position.as_ref().map(|p| p.node_id.clone()))
                    });
                    let detail = format!("onIntervention {}: {}", f.action, error(&f.error));
                    (node, None, Some(detail))
                }
                Err(_) => (None, None, None),
            }
        }
        _ => (None, None, None),
    };
    Happened {
        node_id,
        outcome,
        detail: detail.map(|d| cut(&d)),
        changes: changes(workflow, before, after, kind, payload),
    }
}

/// The first line of an output or of signal data: the text itself, or else its JSON.
/// `None` when there is none.
fn data(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(first_line(text)).filter(|t| !t.is_empty()),
        other => Some(other.to_string()),
    }
}

/// `text` cut to [`DETAIL_CHARS`] characters, ending `…` when it was longer.
fn cut(text: &str) -> String {
    match text.char_indices().nth(DETAIL_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// The changes of one committed transition from `before` (`None` for a run's first)
/// to `after`, which consumed an event of `kind` with `payload`; in the order they
/// happened.
pub fn changes(
    workflow: &Workflow,
    before: Option<&State>,
    after: &State,
    kind: &str,
    payload: &[u8],
) -> Vec<Change> {
    let mut changes = Vec::new();
    if let Some(failed) = failed_try(before, kind, payload) {
        changes.push(failed);
    }
    let mut end = before.is_none().then(|| workflow.start.clone());
    if let Some(parked) = before.and_then(|b| b.position.as_ref()) {
        let moved =
            after.position.as_ref().map(|p| &p.activation_id) != Some(&parked.activation_id);
        if moved {
            end = outcomes(workflow, before, after, &parked.node_id, &mut changes);
        }
    }
    let invocation = |s: &State| {
        s.invocation
            .as_ref()
            .map(|i| (i.invocation_id.clone(), i.attempt))
    };
    let was = before.map(|b| b.status);
    match after.status {
        RunStatus::Running => {
            if let Some(i) = &after.invocation {
                let same = before.and_then(invocation) == invocation(after)
                    && before.and_then(|b| b.invocation.as_ref()).map(|i| i.state)
                        == Some(InvocationPhase::Scheduled);
                if i.state == InvocationPhase::Scheduled && !same {
                    changes.push(Change::StepStarted {
                        node_id: i.node_id.clone(),
                        attempt: i.attempt,
                    });
                }
            }
        }
        RunStatus::Waiting => {
            let position = after.position.as_ref();
            let same = was == Some(RunStatus::Waiting)
                && before
                    .and_then(|b| b.position.as_ref())
                    .map(|p| &p.activation_id)
                    == position.map(|p| &p.activation_id);
            if let (Some(position), false) = (position, same) {
                let signal = match workflow.nodes.get(&position.node_id) {
                    Some(Node::AwaitSignal { signal, .. }) => signal.clone(),
                    _ => String::new(),
                };
                changes.push(Change::Waiting {
                    node_id: position.node_id.clone(),
                    signal,
                });
            }
        }
        RunStatus::NeedsIntervention => {
            let phase = |s: &State| s.invocation.as_ref().map(|i| i.state);
            let same = was == Some(RunStatus::NeedsIntervention)
                && before.and_then(invocation) == invocation(after)
                && before.and_then(phase) == phase(after);
            if let (Some(i), false) = (&after.invocation, same) {
                let reason = match (&i.state, &i.error) {
                    (InvocationPhase::Stuck, Some(error)) => {
                        format!("stuck: {}: {}", error.code, first_line(&error.message))
                    }
                    (InvocationPhase::Stuck, None) => "stuck".to_string(),
                    _ => format!("the outcome of attempt {} is unknown", i.attempt),
                };
                changes.push(Change::NeedsIntervention {
                    node_id: i.node_id.clone(),
                    reason,
                });
            }
        }
        RunStatus::Completed if was != Some(RunStatus::Completed) => {
            let node_id =
                end.filter(|n| matches!(workflow.nodes.get(n), Some(Node::Complete { .. })));
            changes.push(Change::Completed { node_id });
        }
        RunStatus::Failed if was != Some(RunStatus::Failed) => {
            let error = match &after.result {
                Some(RunResult::Error(error)) => Some(error),
                _ => None,
            };
            changes.push(Change::Failed {
                node_id: error.and_then(|e| e.node_id.clone()).or(end),
                code: error.map(|e| e.code.clone()).unwrap_or_default(),
                message: error.map(|e| first_line(&e.message)).unwrap_or_default(),
            });
        }
        RunStatus::Completed | RunStatus::Failed => {}
    }
    changes
}

/// The try a result of the run's outstanding attempt ended without an outcome. A
/// stale or duplicate result changes nothing.
fn failed_try(before: Option<&State>, kind: &str, payload: &[u8]) -> Option<Change> {
    if kind != event_kind::ACTIVITY_RESULT {
        return None;
    }
    let result: ActivityResult = serde_json::from_slice(payload).ok()?;
    let outstanding = before?.invocation.as_ref()?;
    let current = outstanding.invocation_id == result.invocation_id
        && outstanding.attempt == result.attempt
        && outstanding.state == InvocationPhase::Scheduled;
    if !current || result.status == AttemptStatus::Success {
        return None;
    }
    let ErrorInfo { code, message, .. } = result.error.unwrap_or_else(|| ErrorInfo {
        code: format!("attempt.{}", status(result.status)),
        message: match result.status {
            AttemptStatus::Expired => "the attempt's lease ran out".to_string(),
            _ => "the attempt's outcome is unknown".to_string(),
        },
        node_id: None,
        details: serde_json::Value::Null,
    });
    Some(Change::TryFailed {
        node_id: outstanding.node_id.clone(),
        attempt: result.attempt,
        code,
        message: first_line(&message),
    })
}

fn status(status: AttemptStatus) -> &'static str {
    match status {
        AttemptStatus::Success => "success",
        AttemptStatus::Failure => "failure",
        AttemptStatus::Unknown => "unknown",
        AttemptStatus::Expired => "expired",
    }
}

/// From the node the run was parked at, the arrow each node it left took, following
/// the route of each node's new result until the run's new position or an end node.
/// Returns the node the walk ended at. A loop gone round within the transition shows
/// once, with each node's last arrow.
fn outcomes(
    workflow: &Workflow,
    before: Option<&State>,
    after: &State,
    first: &str,
    changes: &mut Vec<Change>,
) -> Option<String> {
    let fresh = |node: &str| {
        after.nodes.contains_key(node)
            && after.nodes.get(node) != before.and_then(|b| b.nodes.get(node))
    };
    let arrived = after.position.as_ref().map(|p| p.node_id.as_str());
    let mut node_id = first.to_string();
    for _ in 0..workflow.nodes.len() {
        if !fresh(&node_id) {
            return Some(node_id);
        }
        let node = workflow.nodes.get(&node_id)?;
        let arrow = taken(after, &node_id, node)?;
        let to = node.outcomes()?.get(arrow)?.clone();
        changes.push(Change::Outcome {
            node_id: node_id.clone(),
            outcome: after.nodes[&node_id].outcome.clone(),
            arrow: arrow.to_string(),
            to: to.clone(),
        });
        let ends = workflow.nodes.get(&to).and_then(Node::outcomes).is_none();
        if ends || arrived == Some(to.as_str()) {
            return Some(to);
        }
        node_id = to;
    }
    Some(node_id)
}

#[cfg(test)]
mod tests {
    use runspore_types::model::{CODEC_VERSION, SEMANTICS_VERSION};
    use runspore_types::reducer::{Envelope, Identity, Limits, TransitionRequest};
    use runspore_types::{canonical, digest};
    use serde_json::{json, Value};

    use super::*;

    const RUN: &str = "run_progress";

    /// `build` retries once, then `test` loops back to `build` on `red` at most once,
    /// and `gate` waits for `approval`.
    fn flow() -> Value {
        json!({
            "format": "runspore.workflow/0.2", "name": "flow", "start": "build",
            "actions": {
                "build": {"effect": "idempotent", "outcomes": ["ok"],
                          "retry": {"maxAttempts": 2}},
                "test": {"effect": "pure", "outcomes": ["green", "red"]},
                "deploy": {"effect": "unsafe", "outcomes": ["ok"]}
            },
            "nodes": {
                "build": {"kind": "activity", "action": "build", "outcomes": {"ok": "test"}},
                "test": {"kind": "activity", "action": "test", "loopLimits": {"red": 1},
                         "outcomes": {"green": "gate", "red": "build", "exhausted": "gate"}},
                "gate": {"kind": "await-signal", "signal": "approval",
                         "outcomes": {"go": "deploy", "stop": "stopped"}},
                "deploy": {"kind": "activity", "action": "deploy", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"},
                "stopped": {"kind": "fail",
                            "error": {"code": "stopped", "message": "stopped\nby a person"}}
            }
        })
    }

    /// Applies `events` with the native kernel; the changes of each transition.
    fn replay(workflow: &Value, events: &[(&str, Value)]) -> Vec<Vec<Change>> {
        history(workflow, events)
            .into_iter()
            .map(|h| h.changes)
            .collect()
    }

    /// Applies `events` with the native kernel; what each was about.
    fn history(workflow: &Value, events: &[(&str, Value)]) -> Vec<Happened> {
        let graph = canonical::to_vec(workflow).unwrap();
        let parsed: Workflow = canonical::decode(&graph).unwrap();
        let mut before: Option<State> = None;
        let mut nodes = HashMap::new();
        let mut snapshot: Option<Vec<u8>> = None;
        let started = json!({"tenant": "default", "runId": RUN, "input": null});
        let all = std::iter::once((event_kind::RUN_STARTED, started)).chain(events.iter().cloned());
        let mut seen = Vec::new();
        for (index, (kind, payload)) in all.enumerate() {
            let sequence = u64::try_from(index).unwrap() + 1;
            let payload = canonical::to_vec(&payload).unwrap();
            let decision = runspore_kernel::transition(&TransitionRequest {
                identity: Identity {
                    package_digest: digest::package(&graph),
                    kernel_digest: "test".into(),
                    semantics_version: SEMANTICS_VERSION.into(),
                    codec_version: CODEC_VERSION.into(),
                },
                graph: graph.clone(),
                snapshot: snapshot.take(),
                input_event: Envelope {
                    event_id: format!("e{sequence}"),
                    sequence,
                    accepted_at_ms: 1_000,
                    kind: kind.to_string(),
                    payload: payload.clone(),
                },
                frozen_limits: Limits::default(),
            })
            .unwrap();
            let after: State = canonical::decode(&decision.snapshot).unwrap();
            seen.push(happened(
                &parsed,
                before.as_ref(),
                &after,
                kind,
                &payload,
                &nodes,
            ));
            if let Some(i) = &after.invocation {
                nodes.insert(i.invocation_id.clone(), i.node_id.clone());
            }
            before = Some(after);
            snapshot = Some(decision.snapshot);
        }
        seen
    }

    fn invocation(node: &str, visit: u32) -> String {
        digest::ids::invocation("default", RUN, &format!("{node}/{visit}"))
    }

    fn success(node: &str, visit: u32, attempt: u32, outcome: &str) -> (&'static str, Value) {
        let id = invocation(node, visit);
        (
            event_kind::ACTIVITY_RESULT,
            json!({"invocationId": id, "attemptId": format!("{id}.{attempt}"),
                   "attempt": attempt, "status": "success", "outcome": outcome}),
        )
    }

    fn failure(node: &str, visit: u32, attempt: u32, retryable: bool) -> (&'static str, Value) {
        let id = invocation(node, visit);
        (
            event_kind::ACTIVITY_RESULT,
            json!({"invocationId": id, "attemptId": format!("{id}.{attempt}"),
                   "attempt": attempt, "status": "failure", "retryable": retryable,
                   "error": {"code": "command.exit", "message": "exit code 1\nmore"}}),
        )
    }

    fn signal(outcome: &str) -> (&'static str, Value) {
        (
            event_kind::SIGNAL_RECEIVED,
            json!({"name": "approval", "outcome": outcome}),
        )
    }

    fn started(node: &str, attempt: u32) -> Change {
        Change::StepStarted {
            node_id: node.into(),
            attempt,
        }
    }

    fn outcome(node: &str, outcome: &str, arrow: &str, to: &str) -> Change {
        Change::Outcome {
            node_id: node.into(),
            outcome: outcome.into(),
            arrow: arrow.into(),
            to: to.into(),
        }
    }

    fn waiting() -> Change {
        Change::Waiting {
            node_id: "gate".into(),
            signal: "approval".into(),
        }
    }

    #[test]
    fn a_run_reports_each_try_arrow_wait_and_its_end() {
        let seen = replay(
            &flow(),
            &[
                failure("build", 1, 1, true),
                success("build", 1, 2, "ok"),
                success("test", 1, 1, "red"),
                success("build", 2, 1, "ok"),
                success("test", 2, 1, "red"),
                signal("go"),
                success("deploy", 1, 1, "ok"),
            ],
        );
        let failed = Change::TryFailed {
            node_id: "build".into(),
            attempt: 1,
            code: "command.exit".into(),
            message: "exit code 1".into(),
        };
        assert_eq!(
            seen,
            vec![
                vec![started("build", 1)],
                vec![failed, started("build", 2)],
                vec![outcome("build", "ok", "ok", "test"), started("test", 1)],
                vec![outcome("test", "red", "red", "build"), started("build", 1)],
                vec![outcome("build", "ok", "ok", "test"), started("test", 1)],
                vec![outcome("test", "red", "exhausted", "gate"), waiting()],
                vec![outcome("gate", "go", "go", "deploy"), started("deploy", 1)],
                vec![
                    outcome("deploy", "ok", "ok", "done"),
                    Change::Completed {
                        node_id: Some("done".into())
                    }
                ],
            ]
        );
    }

    #[test]
    fn a_fail_node_ends_the_run_with_its_error() {
        let seen = replay(
            &flow(),
            &[
                success("build", 1, 1, "ok"),
                success("test", 1, 1, "green"),
                signal("stop"),
            ],
        );
        assert_eq!(
            seen.last().unwrap(),
            &vec![
                outcome("gate", "stop", "stop", "stopped"),
                Change::Failed {
                    node_id: Some("stopped".into()),
                    code: "stopped".into(),
                    message: "stopped".into(),
                },
            ]
        );
    }

    #[test]
    fn a_step_that_gives_up_needs_intervention() {
        let seen = replay(&flow(), &[failure("build", 1, 1, false)]);
        assert_eq!(
            seen[1],
            vec![
                Change::TryFailed {
                    node_id: "build".into(),
                    attempt: 1,
                    code: "command.exit".into(),
                    message: "exit code 1".into(),
                },
                Change::NeedsIntervention {
                    node_id: "build".into(),
                    reason: "stuck: command.exit: exit code 1".into(),
                },
            ]
        );
    }

    /// The node, outcome and detail of each event, without its changes.
    fn said(happened: &Happened) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            happened.node_id.as_deref(),
            happened.outcome.as_deref(),
            happened.detail.as_deref(),
        )
    }

    #[test]
    fn each_event_names_its_node_its_outcome_and_its_first_line() {
        let mut long = success("test", 2, 1, "red");
        long.1["output"] = json!("x".repeat(DETAIL_CHARS + 5) + "\nsecond");
        let seen = history(
            &flow(),
            &[
                failure("build", 1, 1, true),
                success("build", 1, 2, "ok"),
                success("test", 1, 1, "red"),
                success("build", 2, 1, "ok"),
                long,
                failure("build", 1, 2, true),
                signal("go"),
            ],
        );
        let said: Vec<_> = seen.iter().map(said).collect();
        let cut = "x".repeat(DETAIL_CHARS) + "…";
        assert_eq!(
            said,
            vec![
                (Some("build"), None, None),
                (
                    Some("build"),
                    Some("failure"),
                    Some("command.exit: exit code 1")
                ),
                (Some("build"), Some("ok"), None),
                (Some("test"), Some("red"), None),
                (Some("build"), Some("ok"), None),
                (Some("test"), Some("red"), Some(cut.as_str())),
                // A stale result still names its step.
                (
                    Some("build"),
                    Some("failure"),
                    Some("command.exit: exit code 1")
                ),
                (Some("gate"), Some("go"), None),
            ]
        );
        assert_eq!(
            seen[5].changes,
            vec![outcome("test", "red", "exhausted", "gate"), waiting()]
        );
    }

    #[test]
    fn a_buffered_signal_names_no_node() {
        let seen = history(&flow(), &[signal("go")]);
        assert_eq!(said(&seen[1]), (None, Some("go"), None));
    }

    #[test]
    fn a_stale_result_or_an_early_signal_changes_nothing() {
        let seen = replay(&flow(), &[signal("go"), failure("test", 1, 1, true)]);
        assert_eq!(seen[1], vec![]);
        assert_eq!(seen[2], vec![]);
    }
}
