//! The workflow document and its validation: `spec/kernel.md` section 2.

use std::collections::{BTreeMap, BTreeSet};

use runspore_types::canonical;
use runspore_types::model::{
    code, ActionPolicy, Effect, Node, Workflow, WorkflowLimits, MAX_NODES, OUTCOME_FAILED,
    WORKFLOW_FORMAT,
};
use runspore_types::reducer::Failure;
use serde::Deserialize;

use crate::{failure, mapping, pattern};

const MAX_VISITS_PER_NODE: u32 = 1_000;
const MAX_ACTIVATIONS: u32 = 10_000;
const MAX_ATTEMPTS: u32 = 100;

/// A workflow that passed validation, with the policy the kernel reads from each action.
pub(crate) struct Graph {
    workflow: Workflow,
    policies: BTreeMap<String, ActionPolicy>,
}

impl Graph {
    /// Decodes a canonical workflow document and applies rules W01–W11 in order.
    pub(crate) fn load(bytes: &[u8]) -> Result<Self, Failure> {
        let workflow: Workflow = canonical::decode(bytes).map_err(|error| {
            failure::invalid(
                code::WORKFLOW_INVALID,
                format!("workflow document: {error}"),
            )
        })?;
        let policies = validate(&workflow)?;
        Ok(Self { workflow, policies })
    }

    pub(crate) fn start(&self) -> &str {
        &self.workflow.start
    }

    pub(crate) fn limits(&self) -> &WorkflowLimits {
        &self.workflow.limits
    }

    /// A node with its ID as the graph spells it, so both outlive the caller's borrow.
    pub(crate) fn node(&self, id: &str) -> Option<(&str, &Node)> {
        self.workflow
            .nodes
            .get_key_value(id)
            .map(|(id, node)| (id.as_str(), node))
    }

    pub(crate) fn policy(&self, action: &str) -> Option<&ActionPolicy> {
        self.policies.get(action)
    }
}

fn violation(rule: &str, details: impl AsRef<str>) -> Failure {
    failure::invalid(
        code::WORKFLOW_INVALID,
        format!("{rule}: {}", details.as_ref()),
    )
}

/// Rules are checked in order; within a rule, nodes and actions are visited in
/// ascending byte order of their IDs, which is the iteration order of the maps.
fn validate(workflow: &Workflow) -> Result<BTreeMap<String, ActionPolicy>, Failure> {
    let nodes = &workflow.nodes;

    if workflow.format != WORKFLOW_FORMAT {
        return Err(violation(
            "W01",
            format!("format {:?} is not {WORKFLOW_FORMAT:?}", workflow.format),
        ));
    }

    if !pattern::is_name(&workflow.name) {
        return Err(violation("W02", "name must match [A-Za-z0-9_.-]{1,128}"));
    }

    if nodes.is_empty() || nodes.len() > MAX_NODES {
        return Err(violation("W03", format!("1 to {MAX_NODES} nodes required")));
    }
    if let Some(id) = nodes.keys().find(|id| !pattern::is_id(id)) {
        return Err(violation(
            "W03",
            format!("node ID {id:?} must match [A-Za-z0-9_-]{{1,64}}"),
        ));
    }

    if let Some(id) = workflow.actions.keys().find(|id| !pattern::is_id(id)) {
        return Err(violation(
            "W04",
            format!("action ID {id:?} must match [A-Za-z0-9_-]{{1,64}}"),
        ));
    }

    if !nodes.contains_key(&workflow.start) {
        return Err(violation(
            "W05",
            format!("start {:?} names no node", workflow.start),
        ));
    }

    let limits = &workflow.limits;
    if !(1..=MAX_VISITS_PER_NODE).contains(&limits.max_visits_per_node) {
        return Err(violation(
            "W06",
            format!("maxVisitsPerNode must be in 1..={MAX_VISITS_PER_NODE}"),
        ));
    }
    if !(1..=MAX_ACTIVATIONS).contains(&limits.max_activations) {
        return Err(violation(
            "W06",
            format!("maxActivations must be in 1..={MAX_ACTIVATIONS}"),
        ));
    }

    let mut policies = BTreeMap::new();
    for (id, action) in &workflow.actions {
        policies.insert(id.clone(), action_policy(id, action)?);
    }

    for (id, node) in nodes {
        if let Node::Activity {
            action, outcomes, ..
        } = node
        {
            let Some(policy) = policies.get(action) else {
                return Err(violation(
                    "W08",
                    format!("node {id:?}: action {action:?} is not defined"),
                ));
            };
            if let Some(outcome) = policy
                .outcomes
                .iter()
                .find(|outcome| !outcomes.contains_key(*outcome))
            {
                return Err(violation(
                    "W08",
                    format!("node {id:?} does not route outcome {outcome:?}"),
                ));
            }
            if let Some(extra) = outcomes
                .keys()
                .find(|key| *key != OUTCOME_FAILED && !policy.outcomes.contains(key))
            {
                return Err(violation(
                    "W08",
                    format!("node {id:?} routes {extra:?}, which its action does not declare"),
                ));
            }
            targets_exist("W08", id, outcomes, workflow)?;
        }
    }

    for (id, node) in nodes {
        if let Node::AwaitSignal { signal, outcomes } = node {
            if !pattern::is_signal(signal) {
                return Err(violation(
                    "W09",
                    format!("node {id:?}: signal must match [A-Za-z0-9_.-]{{1,64}}"),
                ));
            }
            if outcomes.is_empty() {
                return Err(violation("W09", format!("node {id:?} routes no outcome")));
            }
            if let Some(key) = outcomes.keys().find(|key| !pattern::is_outcome(key)) {
                return Err(violation(
                    "W09",
                    format!("node {id:?}: outcome {key:?} must match [a-z0-9][a-z0-9-]{{0,63}}"),
                ));
            }
            targets_exist("W09", id, outcomes, workflow)?;
        }
    }

    for (id, node) in nodes {
        let checked = match node {
            Node::Activity { input, .. } => mapping::check(input),
            Node::Complete { output } => mapping::check(output),
            Node::AwaitSignal { .. } | Node::Fail { .. } => Ok(()),
        };
        if let Err(reason) = checked {
            return Err(violation("W10", format!("node {id:?}: {reason}")));
        }
    }

    for (id, node) in nodes {
        if let Node::Fail { error } = node {
            if !pattern::is_error_code(&error.code) {
                return Err(violation(
                    "W11",
                    format!("node {id:?}: error.code must match [a-z0-9][a-z0-9.-]{{0,63}}"),
                ));
            }
            if error.node_id.is_some() {
                return Err(violation(
                    "W11",
                    format!("node {id:?}: error.nodeId must be null"),
                ));
            }
        }
    }

    Ok(policies)
}

/// Rule W07 for one action. An effect this semantics version does not support is
/// reported as soon as the action parses, ahead of the action's other requirements.
fn action_policy(id: &str, action: &serde_json::Value) -> Result<ActionPolicy, Failure> {
    let policy = ActionPolicy::deserialize(action)
        .map_err(|error| violation("W07", format!("action {id:?}: {error}")))?;
    if policy.effect == Effect::Reconcilable {
        return Err(failure::incompatible(
            code::WORKFLOW_UNSUPPORTED_EFFECT,
            format!("action {id:?}: effect reconcilable is not supported"),
        ));
    }
    if policy.outcomes.is_empty() {
        return Err(violation(
            "W07",
            format!("action {id:?} declares no outcome"),
        ));
    }
    let mut seen = BTreeSet::new();
    for outcome in &policy.outcomes {
        if !pattern::is_outcome(outcome) {
            return Err(violation(
                "W07",
                format!("action {id:?}: outcome {outcome:?} must match [a-z0-9][a-z0-9-]{{0,63}}"),
            ));
        }
        if outcome == OUTCOME_FAILED {
            return Err(violation(
                "W07",
                format!("action {id:?}: outcome {OUTCOME_FAILED:?} is reserved"),
            ));
        }
        if !seen.insert(outcome.as_str()) {
            return Err(violation(
                "W07",
                format!("action {id:?} declares outcome {outcome:?} twice"),
            ));
        }
    }
    if !(1..=MAX_ATTEMPTS).contains(&policy.retry.max_attempts) {
        return Err(violation(
            "W07",
            format!("action {id:?}: maxAttempts must be in 1..={MAX_ATTEMPTS}"),
        ));
    }
    Ok(policy)
}

fn targets_exist(
    rule: &str,
    id: &str,
    outcomes: &BTreeMap<String, String>,
    workflow: &Workflow,
) -> Result<(), Failure> {
    match outcomes
        .iter()
        .find(|(_, target)| !workflow.nodes.contains_key(*target))
    {
        Some((outcome, target)) => Err(violation(
            rule,
            format!("node {id:?}: outcome {outcome:?} targets {target:?}, which names no node"),
        )),
        None => Ok(()),
    }
}
