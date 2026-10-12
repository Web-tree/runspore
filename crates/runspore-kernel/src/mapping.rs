//! Mappings: `spec/kernel.md` section 4.
//!
//! A mapping is a JSON value evaluated against the run context. `{"$get": path}`
//! projects from the context, falling back to its `default` when a 0.2 document gives
//! one; `{"$literal": value}` yields its value unevaluated, every other object and
//! array is evaluated member by member, and scalars are themselves.

use std::collections::BTreeMap;

use runspore_types::model::NodeResult;
use serde_json::{Map, Value};

use crate::failure::object;

const GET: &str = "$get";
const DEFAULT: &str = "default";
const LITERAL: &str = "$literal";
const OPERATOR_PREFIX: char = '$';

const ROOT_INPUT: &str = "input";
const ROOT_NODES: &str = "nodes";
const ROOT_RUN: &str = "run";
const ROOT_FROM: &str = "from";

/// What a mapping may use. `from` as a path's first step and `default` beside
/// `$get` exist in `runspore.workflow/0.2` documents only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    V0_1,
    V0_2,
}

/// What a `$get` path projects from: the run input, the latest result of each
/// node, the run's identity, and the arrow the run took into the node being entered.
pub(crate) struct Context<'a> {
    pub(crate) input: &'a Value,
    pub(crate) nodes: &'a BTreeMap<String, NodeResult>,
    pub(crate) run_id: &'a str,
    pub(crate) tenant: &'a str,
    /// The node whose recorded result routed the run here; `None` at the start node.
    pub(crate) from: Option<(&'a str, &'a NodeResult)>,
}

/// The path of the first `$get` that did not resolve, as written in the mapping.
pub(crate) struct MissingPath(pub(crate) Value);

/// Rule W10. A `$get` object has no other key, or in 0.2 also `default`; a
/// `$literal` object has no other key; no other key may start with `$`; a path is a
/// non-empty array of strings and non-negative integers whose first step is `input`,
/// `nodes`, or `run`, or in 0.2 also `from`. `$literal` and `default` are free.
pub(crate) fn check(mapping: &Value, dialect: Dialect) -> Result<(), String> {
    match mapping {
        Value::Object(map) => {
            if let Some(path) = map.get(GET) {
                let allowed =
                    |key: &String| key == GET || (dialect == Dialect::V0_2 && key == DEFAULT);
                if !map.keys().all(allowed) {
                    return Err(match dialect {
                        Dialect::V0_1 => format!("{GET} must be the only key of its object"),
                        Dialect::V0_2 => format!("a {GET} object takes only {GET} and {DEFAULT}"),
                    });
                }
                return check_path(path, dialect);
            }
            if map.contains_key(LITERAL) {
                if map.len() != 1 {
                    return Err(format!("{LITERAL} must be the only key of its object"));
                }
                return Ok(());
            }
            for (key, member) in map {
                if key.starts_with(OPERATOR_PREFIX) {
                    return Err(format!("unknown mapping operator {key:?}"));
                }
                check(member, dialect)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|item| check(item, dialect)),
        _ => Ok(()),
    }
}

fn check_path(path: &Value, dialect: Dialect) -> Result<(), String> {
    let Some(steps) = path.as_array() else {
        return Err(format!("{GET} takes an array of steps"));
    };
    let rooted = steps.first().and_then(Value::as_str).is_some_and(|root| {
        matches!(root, ROOT_INPUT | ROOT_NODES | ROOT_RUN)
            || (dialect == Dialect::V0_2 && root == ROOT_FROM)
    });
    if !rooted {
        return Err(match dialect {
            Dialect::V0_1 => {
                format!("a {GET} path starts with {ROOT_INPUT:?}, {ROOT_NODES:?} or {ROOT_RUN:?}")
            }
            Dialect::V0_2 => format!(
                "a {GET} path starts with {ROOT_INPUT:?}, {ROOT_NODES:?}, {ROOT_RUN:?} or {ROOT_FROM:?}"
            ),
        });
    }
    let well_formed = steps.iter().all(|step| match step {
        Value::String(_) => true,
        Value::Number(index) => index.is_u64(),
        _ => false,
    });
    if !well_formed {
        return Err(format!(
            "a {GET} step is a string or a non-negative integer"
        ));
    }
    Ok(())
}

/// Evaluates a mapping that passed [`check`]. Each mapping value visited costs one
/// operation and each step of a `$get` path costs one more; a `$get` that falls back
/// to its `default` costs one more again. Object members are
/// evaluated in canonical key order (UTF-16 code units), array elements in index
/// order, so the first unresolved path is the same on every host.
pub(crate) fn evaluate(
    mapping: &Value,
    context: &Context<'_>,
    operations: &mut u64,
) -> Result<Value, MissingPath> {
    *operations = operations.saturating_add(1);
    match mapping {
        Value::Object(map) => {
            if let Some(path) = map.get(GET) {
                let steps = path.as_array().map(Vec::as_slice).unwrap_or_default();
                let cost = u64::try_from(steps.len()).unwrap_or(u64::MAX);
                *operations = operations.saturating_add(cost);
                return match (resolve(steps, context), map.get(DEFAULT)) {
                    (Some(value), _) => Ok(value),
                    (None, Some(fallback)) => {
                        *operations = operations.saturating_add(1);
                        Ok(fallback.clone())
                    }
                    (None, None) => Err(MissingPath(path.clone())),
                };
            }
            if let Some(literal) = map.get(LITERAL) {
                return Ok(literal.clone());
            }
            let mut members: Vec<(&String, &Value)> = map.iter().collect();
            members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let mut evaluated = Map::new();
            for (key, member) in members {
                evaluated.insert(key.clone(), evaluate(member, context, operations)?);
            }
            Ok(Value::Object(evaluated))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| evaluate(item, context, operations))
            .collect::<Result<Vec<Value>, MissingPath>>()
            .map(Value::Array),
        scalar => Ok(scalar.clone()),
    }
}

/// The value at a path, or `None` when a step selects nothing. The context is the
/// object `{"input": ..., "nodes": {id: {visit, outcome, output}}, "run": {id, tenant},
/// "from": {node, outcome, output} | null}`; only the part a path reaches is
/// materialized.
fn resolve(steps: &[Value], context: &Context<'_>) -> Option<Value> {
    let (root, rest) = steps.split_first()?;
    match root.as_str()? {
        ROOT_INPUT => walk(context.input, rest).cloned(),
        ROOT_NODES => resolve_nodes(rest, context.nodes),
        ROOT_RUN => resolve_run(rest, context),
        ROOT_FROM => resolve_from(rest, context.from),
        _ => None,
    }
}

fn resolve_nodes(steps: &[Value], nodes: &BTreeMap<String, NodeResult>) -> Option<Value> {
    let Some((node, rest)) = steps.split_first() else {
        let all = nodes
            .iter()
            .map(|(id, result)| (id.clone(), node_value(result)))
            .collect();
        return Some(Value::Object(all));
    };
    let result = nodes.get(node.as_str()?)?;
    let Some((field, rest)) = rest.split_first() else {
        return Some(node_value(result));
    };
    match field.as_str()? {
        "output" => walk(&result.output, rest).cloned(),
        "visit" if rest.is_empty() => Some(Value::from(result.visit)),
        "outcome" if rest.is_empty() => Some(Value::String(result.outcome.clone())),
        _ => None,
    }
}

fn node_value(result: &NodeResult) -> Value {
    object([
        ("visit", Value::from(result.visit)),
        ("outcome", Value::String(result.outcome.clone())),
        ("output", result.output.clone()),
    ])
}

/// `from` is `null` at the start node, so only the bare path resolves there.
fn resolve_from(steps: &[Value], from: Option<(&str, &NodeResult)>) -> Option<Value> {
    let Some((node, result)) = from else {
        return steps.is_empty().then_some(Value::Null);
    };
    let Some((field, rest)) = steps.split_first() else {
        return Some(object([
            ("node", Value::String(node.to_string())),
            ("outcome", Value::String(result.outcome.clone())),
            ("output", result.output.clone()),
        ]));
    };
    match field.as_str()? {
        "output" => walk(&result.output, rest).cloned(),
        "node" if rest.is_empty() => Some(Value::String(node.to_string())),
        "outcome" if rest.is_empty() => Some(Value::String(result.outcome.clone())),
        _ => None,
    }
}

fn resolve_run(steps: &[Value], context: &Context<'_>) -> Option<Value> {
    let id = || Value::String(context.run_id.to_string());
    let tenant = || Value::String(context.tenant.to_string());
    let Some((field, rest)) = steps.split_first() else {
        return Some(object([("id", id()), ("tenant", tenant())]));
    };
    if !rest.is_empty() {
        return None;
    }
    match field.as_str()? {
        "id" => Some(id()),
        "tenant" => Some(tenant()),
        _ => None,
    }
}

/// A string step selects an object member; an integer step selects an array element.
fn walk<'v>(mut value: &'v Value, steps: &[Value]) -> Option<&'v Value> {
    for step in steps {
        value = match step {
            Value::String(key) => value.as_object()?.get(key)?,
            Value::Number(index) => {
                let index = usize::try_from(index.as_u64()?).ok()?;
                value.as_array()?.get(index)?
            }
            _ => return None,
        };
    }
    Some(value)
}
