//! The workflow document and its validation: `spec/kernel.md` section 2.

use std::collections::{BTreeMap, BTreeSet};

use runspore_types::canonical;
use runspore_types::model::{
    code, ActionPolicy, Effect, Node, Workflow, WorkflowLimits, MAX_NODES, OUTCOME_EXHAUSTED,
    OUTCOME_FAILED, WORKFLOW_FORMATS, WORKFLOW_FORMAT_0_2,
};
use runspore_types::reducer::Failure;
use serde::Deserialize;
use serde_json::Value;

use crate::mapping::Dialect;
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
    /// Decodes a canonical workflow document and applies rules W01–W15 in order.
    /// Within W10, a 0.2 document's mappings may use `from` and `default`; W12 admits
    /// `loopLimits` only in a 0.2 document, and W14 and W15 apply only to one.
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

    /// Whether a step that gives up with no `failed` route is stuck (a 0.2 document)
    /// rather than ending the run (0.1).
    pub(crate) fn parks_stuck_steps(&self) -> bool {
        is_0_2(&self.workflow)
    }

    /// Whether `exhausted` is reserved, so no signal takes it (a 0.2 document).
    pub(crate) fn reserves_exhausted(&self) -> bool {
        is_0_2(&self.workflow)
    }
}

fn is_0_2(workflow: &Workflow) -> bool {
    workflow.format == WORKFLOW_FORMAT_0_2
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

    if !WORKFLOW_FORMATS.contains(&workflow.format.as_str()) {
        return Err(violation(
            "W01",
            format!(
                "format {:?} is not one of {WORKFLOW_FORMATS:?}",
                workflow.format
            ),
        ));
    }

    if workflow.format != WORKFLOW_FORMAT_0_2 {
        if let Some(id) = nodes
            .keys()
            .find(|id| output_schemas(&nodes[*id]).is_some())
        {
            return Err(violation(
                "W01",
                format!("node {id:?}: outputSchemas needs format {WORKFLOW_FORMAT_0_2:?}"),
            ));
        }
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

    let v0_2 = is_0_2(workflow);
    let mut policies = BTreeMap::new();
    for (id, action) in &workflow.actions {
        policies.insert(id.clone(), action_policy(id, action, v0_2)?);
    }

    // In a 0.2 document, a node routes `exhausted` only when it has loop limits.
    let may_route_exhausted = |node: &Node| v0_2 && node.loop_limits().is_some();

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
            if let Some(extra) = outcomes.keys().find(|key| {
                *key != OUTCOME_FAILED
                    && !(*key == OUTCOME_EXHAUSTED && may_route_exhausted(node))
                    && !policy.outcomes.contains(key)
            }) {
                return Err(violation(
                    "W08",
                    format!("node {id:?} routes {extra:?}, which its action does not declare"),
                ));
            }
            targets_exist("W08", id, outcomes, workflow)?;
        }
    }

    for (id, node) in nodes {
        if let Node::AwaitSignal {
            signal, outcomes, ..
        } = node
        {
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
            if v0_2 && outcomes.contains_key(OUTCOME_EXHAUSTED) && !may_route_exhausted(node) {
                return Err(violation(
                    "W09",
                    format!("node {id:?} routes {OUTCOME_EXHAUSTED:?} but has no loopLimits"),
                ));
            }
            targets_exist("W09", id, outcomes, workflow)?;
        }
    }

    let dialect = dialect(workflow);
    for (id, node) in nodes {
        let checked = match node {
            Node::Activity { input, .. } => mapping::check(input, dialect),
            Node::Complete { output } => mapping::check(output, dialect),
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

    for (id, node) in nodes {
        loop_limits(id, node, workflow, v0_2)?;
    }

    output_schemas_fit(workflow, &policies)?;

    if v0_2 {
        reachability(workflow)?;
    }

    Ok(policies)
}

fn output_schemas(node: &Node) -> Option<&BTreeMap<String, Value>> {
    match node {
        Node::Activity { output_schemas, .. } | Node::AwaitSignal { output_schemas, .. } => {
            output_schemas.as_ref()
        }
        Node::Complete { .. } | Node::Fail { .. } => None,
    }
}

/// W13: each node's `outputSchemas` is non-empty, keyed by outcomes the node can
/// produce, and holds objects; await-signal nodes that wait for one signal declare
/// equal schemas for every outcome they share. The schemas' contents are the host's.
fn output_schemas_fit(
    workflow: &Workflow,
    policies: &BTreeMap<String, ActionPolicy>,
) -> Result<(), Failure> {
    let mut waiters: BTreeMap<&str, (&str, &BTreeMap<String, String>)> = BTreeMap::new();
    for (id, node) in &workflow.nodes {
        let schemas = output_schemas(node);
        for (outcome, schema) in schemas.into_iter().flatten() {
            let produced = match node {
                Node::Activity { action, .. } => policies
                    .get(action)
                    .is_some_and(|policy| policy.outcomes.contains(outcome)),
                Node::AwaitSignal { outcomes, .. } => outcomes.contains_key(outcome),
                Node::Complete { .. } | Node::Fail { .. } => false,
            };
            if !produced || outcome == OUTCOME_FAILED || outcome == OUTCOME_EXHAUSTED {
                return Err(violation(
                    "W13",
                    format!(
                        "node {id:?}: outputSchemas names {outcome:?}, which is not an outcome \
                         the node produces"
                    ),
                ));
            }
            if !schema.is_object() {
                return Err(violation(
                    "W13",
                    format!("node {id:?}: the schema of {outcome:?} is not an object"),
                ));
            }
        }
        if schemas.is_some_and(BTreeMap::is_empty) {
            return Err(violation(
                "W13",
                format!("node {id:?}: outputSchemas is empty"),
            ));
        }
        let Node::AwaitSignal {
            signal, outcomes, ..
        } = node
        else {
            continue;
        };
        let Some((other, other_outcomes)) = waiters.get(signal.as_str()) else {
            waiters.insert(signal, (id, outcomes));
            continue;
        };
        let theirs = output_schemas(&workflow.nodes[*other]);
        for outcome in outcomes.keys().filter(|o| other_outcomes.contains_key(*o)) {
            if schemas.and_then(|s| s.get(outcome)) != theirs.and_then(|s| s.get(outcome)) {
                return Err(violation(
                    "W13",
                    format!(
                        "nodes {other:?} and {id:?} wait for signal {signal:?} with different \
                         schemas for outcome {outcome:?}"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Rule W12 for one node.
fn loop_limits(id: &str, node: &Node, workflow: &Workflow, v0_2: bool) -> Result<(), Failure> {
    // An end node cannot carry the field at all: it does not decode.
    let (Some(limits), Some(outcomes)) = (node.loop_limits(), node.outcomes()) else {
        return Ok(());
    };
    if !v0_2 {
        return Err(violation(
            "W12",
            format!("node {id:?}: loopLimits needs format {WORKFLOW_FORMAT_0_2:?}"),
        ));
    }
    if limits.is_empty() {
        return Err(violation(
            "W12",
            format!("node {id:?}: loopLimits is empty"),
        ));
    }
    let ceiling = workflow.limits.max_visits_per_node;
    for (outcome, limit) in limits {
        if outcome == OUTCOME_EXHAUSTED || !outcomes.contains_key(outcome) {
            return Err(violation(
                "W12",
                format!("node {id:?}: loopLimits names {outcome:?}, which the node does not route"),
            ));
        }
        if *limit == 0 || limit.saturating_add(1) > ceiling {
            return Err(violation(
                "W12",
                format!(
                    "node {id:?}: loop limit {limit} for {outcome:?} must be at least 1 and \
                     at most maxVisitsPerNode - 1 ({})",
                    ceiling.saturating_sub(1)
                ),
            ));
        }
    }
    if !outcomes.contains_key(OUTCOME_EXHAUSTED) {
        return Err(violation(
            "W12",
            format!("node {id:?} has loopLimits but routes no {OUTCOME_EXHAUSTED:?}"),
        ));
    }
    Ok(())
}

/// Rules W14 and W15: every node is reachable from `start`, and from every node some
/// path reaches a `complete` or `fail` node. Each reports the first node in byte order
/// that breaks it.
fn reachability(workflow: &Workflow) -> Result<(), Failure> {
    let nodes = &workflow.nodes;
    fn targets(node: &Node) -> Vec<&String> {
        node.outcomes()
            .map(|outcomes| outcomes.values().collect())
            .unwrap_or_default()
    }

    let mut reached = BTreeSet::from([workflow.start.as_str()]);
    let mut frontier = vec![workflow.start.as_str()];
    while let Some(id) = frontier.pop() {
        for target in nodes.get(id).map(targets).unwrap_or_default() {
            if reached.insert(target) {
                frontier.push(target);
            }
        }
    }
    if let Some(id) = nodes.keys().find(|id| !reached.contains(id.as_str())) {
        return Err(violation(
            "W14",
            format!(
                "node {id:?} is not reachable from start {:?}",
                workflow.start
            ),
        ));
    }

    // Walk the arrows backwards from the ends.
    let mut into: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, node) in nodes {
        for target in targets(node) {
            into.entry(target.as_str()).or_default().push(id.as_str());
        }
    }
    let mut ends: BTreeSet<&str> = nodes
        .iter()
        .filter(|(_, node)| node.outcomes().is_none())
        .map(|(id, _)| id.as_str())
        .collect();
    let mut frontier: Vec<&str> = ends.iter().copied().collect();
    while let Some(id) = frontier.pop() {
        for source in into.get(id).into_iter().flatten() {
            if ends.insert(source) {
                frontier.push(source);
            }
        }
    }
    if let Some(id) = nodes.keys().find(|id| !ends.contains(id.as_str())) {
        return Err(violation(
            "W15",
            format!("node {id:?} reaches no complete or fail node"),
        ));
    }
    Ok(())
}

/// The mapping dialect of a document whose format passed W01.
fn dialect(workflow: &Workflow) -> Dialect {
    if is_0_2(workflow) {
        Dialect::V0_2
    } else {
        Dialect::V0_1
    }
}

/// Rule W07 for one action. An effect this semantics version does not support is
/// reported as soon as the action parses, ahead of the action's other requirements.
fn action_policy(
    id: &str,
    action: &serde_json::Value,
    v0_2: bool,
) -> Result<ActionPolicy, Failure> {
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
        if outcome == OUTCOME_FAILED || (v0_2 && outcome == OUTCOME_EXHAUSTED) {
            return Err(violation(
                "W07",
                format!("action {id:?}: outcome {outcome:?} is reserved"),
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
