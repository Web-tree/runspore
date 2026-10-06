//! Mappings: `spec/kernel.md` section 4.
//!
//! A mapping is a JSON value evaluated against the run context. `{"$get": path}`
//! projects from the context, `{"$literal": value}` yields its value unevaluated,
//! every other object and array is evaluated member by member, and scalars are
//! themselves.

use std::collections::BTreeMap;

use runspore_types::model::NodeResult;
use serde_json::{Map, Value};

use crate::failure::object;

const GET: &str = "$get";
const LITERAL: &str = "$literal";
const OPERATOR_PREFIX: char = '$';

const ROOT_INPUT: &str = "input";
const ROOT_NODES: &str = "nodes";
const ROOT_RUN: &str = "run";

/// What a `$get` path projects from: the run input, the latest result of each
/// node, and the run's identity.
pub(crate) struct Context<'a> {
    pub(crate) input: &'a Value,
    pub(crate) nodes: &'a BTreeMap<String, NodeResult>,
    pub(crate) run_id: &'a str,
    pub(crate) tenant: &'a str,
}

/// The path of the first `$get` that did not resolve, as written in the mapping.
pub(crate) struct MissingPath(pub(crate) Value);

/// Rule W10. A `$get` or `$literal` object has no other key; no other key may
/// start with `$`; a path is a non-empty array of strings and non-negative integers
/// whose first step is `input`, `nodes`, or `run`. The value of `$literal` is free.
pub(crate) fn check(mapping: &Value) -> Result<(), String> {
    match mapping {
        Value::Object(map) => {
            if let Some(path) = map.get(GET) {
                if map.len() != 1 {
                    return Err(format!("{GET} must be the only key of its object"));
                }
                return check_path(path);
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
                check(member)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(check),
        _ => Ok(()),
    }
}

fn check_path(path: &Value) -> Result<(), String> {
    let Some(steps) = path.as_array() else {
        return Err(format!("{GET} takes an array of steps"));
    };
    let rooted = steps
        .first()
        .and_then(Value::as_str)
        .is_some_and(|root| matches!(root, ROOT_INPUT | ROOT_NODES | ROOT_RUN));
    if !rooted {
        return Err(format!(
            "a {GET} path starts with {ROOT_INPUT:?}, {ROOT_NODES:?} or {ROOT_RUN:?}"
        ));
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
/// operation and each step of a `$get` path costs one more. Object members are
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
                return resolve(steps, context).ok_or_else(|| MissingPath(path.clone()));
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
/// object `{"input": ..., "nodes": {id: {visit, outcome, output}}, "run": {id, tenant}}`;
/// only the part a path reaches is materialized.
fn resolve(steps: &[Value], context: &Context<'_>) -> Option<Value> {
    let (root, rest) = steps.split_first()?;
    match root.as_str()? {
        ROOT_INPUT => walk(context.input, rest).cloned(),
        ROOT_NODES => resolve_nodes(rest, context.nodes),
        ROOT_RUN => resolve_run(rest, context),
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
