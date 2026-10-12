//! The arrow a run took into where it is, read back from its snapshots
//! (`spec/cli.md`, `status --json`: `position.from` and `end`).
//!
//! The kernel's state does not record arrows. A transition that moves the run
//! records the result of every node it leaves and then enters that result's route,
//! so the arrow into the new node is found from the snapshot before the transition
//! and the one after it: start at the node the run was parked at and follow, node by
//! node, the route of the result each node now holds. Following each node's latest
//! result skips any loop the transition went round, and ends with its last arrow.
//! This module only reads routes; it decides nothing.

use runspore_types::model::{Node, RunResult, RunStatus, State, Workflow};

/// An arrow: the node whose result routed the run, and the outcome it routed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrow {
    pub node_id: String,
    pub outcome: String,
}

/// The node a `completed` or `failed` run ended at, and the arrow into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct End {
    pub node_id: String,
    pub from: Option<Arrow>,
}

/// Where a run arrived. `from` is the arrow into the current position (`None` at the
/// start node's first activation or with no position); `end` is set once the run is
/// `completed` or `failed`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Arrival {
    pub from: Option<Arrow>,
    pub end: Option<End>,
}

/// Folds a run's snapshots, oldest first, one per committed transition.
pub struct Trail<'a> {
    workflow: &'a Workflow,
    last: Option<State>,
    arrival: Arrival,
}

impl<'a> Trail<'a> {
    pub fn new(workflow: &'a Workflow) -> Self {
        Self {
            workflow,
            last: None,
            arrival: Arrival::default(),
        }
    }

    /// The snapshot the next committed transition produced.
    pub fn push(&mut self, state: State) {
        let Some(before) = self.last.take() else {
            if state.status.is_terminal() {
                self.arrival.end = Some(End {
                    node_id: self.workflow.start.clone(),
                    from: None,
                });
            }
            self.last = Some(state);
            return;
        };
        if !before.status.is_terminal() {
            self.step(&before, &state);
        }
        self.last = Some(state);
    }

    pub fn finish(self) -> Arrival {
        self.arrival
    }

    fn step(&mut self, before: &State, after: &State) {
        let Some(parked) = &before.position else {
            return;
        };
        let moved =
            after.position.as_ref().map(|p| &p.activation_id) != Some(&parked.activation_id);
        if !moved {
            return;
        }
        let left = after
            .nodes
            .get(&parked.node_id)
            .is_some_and(|result| result.visit == parked.visit)
            && before
                .nodes
                .get(&parked.node_id)
                .is_none_or(|result| result.visit != parked.visit);
        if let Some(position) = &after.position {
            self.arrival.from =
                self.walk(after, &parked.node_id, |node, _| node == position.node_id);
            return;
        }
        if !matches!(after.status, RunStatus::Completed | RunStatus::Failed) {
            return;
        }
        let failed_at = match &after.result {
            Some(RunResult::Error(error)) => error.node_id.clone(),
            _ => None,
        };
        // An activity that failed with no `failed` route ends the run where it stands.
        if !left {
            self.arrival.end = failed_at.map(|node_id| End {
                node_id,
                from: self.arrival.from.take(),
            });
            return;
        }
        let ends_here = |node: &str, kind: &Node| match &failed_at {
            Some(failed) => node == failed,
            None => matches!(kind, Node::Complete { .. }),
        };
        let mut end = None;
        let from = self.walk(after, &parked.node_id, |node, kind| {
            let found = ends_here(node, kind);
            if found {
                end = Some(node.to_string());
            }
            found
        });
        self.arrival.from = None;
        self.arrival.end = end.map(|node_id| End { node_id, from });
    }

    /// From `first`, follows the route of each node's recorded result until a target
    /// satisfies `arrived`; returns the arrow into that target.
    fn walk(
        &self,
        state: &State,
        first: &str,
        mut arrived: impl FnMut(&str, &Node) -> bool,
    ) -> Option<Arrow> {
        let mut node_id = first.to_string();
        for _ in 0..=self.workflow.nodes.len() {
            let outcome = &state.nodes.get(&node_id)?.outcome;
            let routes = match self.workflow.nodes.get(&node_id)? {
                Node::Activity { outcomes, .. } | Node::AwaitSignal { outcomes, .. } => outcomes,
                Node::Complete { .. } | Node::Fail { .. } => return None,
            };
            let target = routes.get(outcome)?;
            if arrived(target, self.workflow.nodes.get(target)?) {
                return Some(Arrow {
                    node_id,
                    outcome: outcome.clone(),
                });
            }
            node_id = target.clone();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use runspore_types::model::{event_kind, CODEC_VERSION, SEMANTICS_VERSION};
    use runspore_types::reducer::{Envelope, Identity, Limits, TransitionRequest};
    use runspore_types::{canonical, digest};
    use serde_json::{json, Value};

    use super::*;

    const RUN: &str = "run_trail";

    /// `work` hands over to two gates that pass a run back and forth on buffered
    /// signals before it returns to `work` or finishes.
    fn gates() -> Value {
        json!({
            "format": "runspore.workflow/0.2", "name": "gates", "start": "work",
            "actions": {"act": {"effect": "pure", "outcomes": ["ok", "red"]}},
            "nodes": {
                "work": {"kind": "activity", "action": "act",
                         "outcomes": {"ok": "gate1", "red": "work"}},
                "gate1": {"kind": "await-signal", "signal": "s1",
                          "outcomes": {"go": "gate2", "back": "work"}},
                "gate2": {"kind": "await-signal", "signal": "s2",
                          "outcomes": {"go": "work", "loop": "gate1", "end": "done"}},
                "done": {"kind": "complete"}
            }
        })
    }

    /// Applies `events` with the native kernel and folds every snapshot.
    fn arrival(workflow: &Value, events: &[(&str, Value)]) -> Arrival {
        let graph = canonical::to_vec(workflow).unwrap();
        let parsed: Workflow = canonical::decode(&graph).unwrap();
        let mut trail = Trail::new(&parsed);
        let mut snapshot: Option<Vec<u8>> = None;
        let started = json!({"tenant": "default", "runId": RUN, "input": null});
        let all = std::iter::once((event_kind::RUN_STARTED, started)).chain(events.iter().cloned());
        for (index, (kind, payload)) in all.enumerate() {
            let sequence = u64::try_from(index).unwrap() + 1;
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
                    payload: canonical::to_vec(&payload).unwrap(),
                },
                frozen_limits: Limits::default(),
            })
            .unwrap();
            trail.push(canonical::decode(&decision.snapshot).unwrap());
            snapshot = Some(decision.snapshot);
        }
        trail.finish()
    }

    fn result(visit: u32, outcome: &str) -> (&'static str, Value) {
        let invocation = digest::ids::invocation("default", RUN, &format!("work/{visit}"));
        (
            event_kind::ACTIVITY_RESULT,
            json!({"invocationId": invocation, "attemptId": format!("{invocation}.1"),
                   "attempt": 1, "status": "success", "outcome": outcome}),
        )
    }

    fn signal(name: &str, outcome: &str) -> (&'static str, Value) {
        (
            event_kind::SIGNAL_RECEIVED,
            json!({"name": name, "outcome": outcome}),
        )
    }

    fn arrow(node: &str, outcome: &str) -> Option<Arrow> {
        Some(Arrow {
            node_id: node.into(),
            outcome: outcome.into(),
        })
    }

    #[test]
    fn the_start_node_has_no_arrow() {
        assert_eq!(arrival(&gates(), &[]), Arrival::default());
    }

    #[test]
    fn a_self_loop_is_its_own_arrow() {
        let arrived = arrival(&gates(), &[result(1, "red")]);
        assert_eq!(arrived.from, arrow("work", "red"));
    }

    /// One transition takes four buffered signals and goes round the gates twice;
    /// the arrow is the last one taken.
    #[test]
    fn one_transition_round_a_loop_reports_its_last_arrow() {
        let events = [
            signal("s1", "go"),
            signal("s2", "loop"),
            signal("s1", "go"),
            signal("s2", "go"),
            result(1, "ok"),
        ];
        assert_eq!(arrival(&gates(), &events).from, arrow("gate2", "go"));

        let events = [signal("s1", "go"), result(1, "ok")];
        assert_eq!(arrival(&gates(), &events).from, arrow("gate1", "go"));
    }

    #[test]
    fn a_completed_run_ends_at_its_complete_node() {
        let events = [
            signal("s1", "go"),
            signal("s2", "loop"),
            signal("s1", "go"),
            signal("s2", "end"),
            result(1, "ok"),
        ];
        assert_eq!(
            arrival(&gates(), &events),
            Arrival {
                from: None,
                end: Some(End {
                    node_id: "done".into(),
                    from: arrow("gate2", "end"),
                }),
            }
        );
    }

    #[test]
    fn a_run_over_the_visit_limit_ends_where_it_could_not_enter() {
        let mut workflow = gates();
        workflow["limits"] = json!({"maxVisitsPerNode": 1});
        assert_eq!(
            arrival(&workflow, &[result(1, "red")]).end,
            Some(End {
                node_id: "work".into(),
                from: arrow("work", "red"),
            })
        );
    }
}
