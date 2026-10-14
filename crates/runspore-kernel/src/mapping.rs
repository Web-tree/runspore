//! Mappings: `spec/kernel.md` section 4.
//!
//! A mapping is a JSON value evaluated against the run context. `{"$get": path}`
//! projects from the context, falling back to its `default` when a 0.2 document gives
//! one; `{"$literal": value}` yields its value unevaluated; in a 0.2 document `$if`,
//! `$uuid`, `$json` and `$join` compute a value from their operands. Every other
//! object and array is evaluated member by member, and scalars are themselves.
//!
//! Hosts evaluate mappings outside the kernel too (an action's `argv`, a command's
//! `result`), over roots of their own: [`check`] and [`evaluate`] are that entry
//! point, with the same operators and the same error codes.

use std::collections::BTreeMap;

use runspore_types::canonical::{self, CanonError};
use runspore_types::digest::ids;
use runspore_types::model::{code, NodeResult};
use serde_json::{Map, Value};

use crate::failure::object;

const GET: &str = "$get";
const DEFAULT: &str = "default";
const LITERAL: &str = "$literal";
const IF: &str = "$if";
const UUID: &str = "$uuid";
const JSON: &str = "$json";
const JOIN: &str = "$join";
const OPERATOR_PREFIX: char = '$';

const ROOT_INPUT: &str = "input";
const ROOT_NODES: &str = "nodes";
const ROOT_RUN: &str = "run";
const ROOT_FROM: &str = "from";

const ROOTS_0_1: &[&str] = &[ROOT_INPUT, ROOT_NODES, ROOT_RUN];
const ROOTS_0_2: &[&str] = &[ROOT_INPUT, ROOT_NODES, ROOT_RUN, ROOT_FROM];

/// What a mapping may use. `from` as a path's first step, `default` beside `$get`,
/// and the operators `$if`, `$uuid`, `$json` and `$join` exist in
/// `runspore.workflow/0.2` documents only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    V0_1,
    V0_2,
}

/// The rules one mapping is checked by: whether the 0.2 additions are allowed, and
/// the first steps a `$get` path may take.
#[derive(Clone, Copy)]
struct Rules<'r> {
    v0_2: bool,
    roots: &'r [&'r str],
}

impl Dialect {
    fn rules(self) -> Rules<'static> {
        match self {
            Dialect::V0_1 => Rules {
                v0_2: false,
                roots: ROOTS_0_1,
            },
            Dialect::V0_2 => Rules {
                v0_2: true,
                roots: ROOTS_0_2,
            },
        }
    }
}

/// What a `$get` path projects from. The kernel's run context is one; a host's
/// roots, an object from root name to value, are another.
pub trait Scope {
    /// The value at a whole path, root first, or `None` when a step selects nothing.
    fn resolve(&self, steps: &[Value]) -> Option<Value>;
}

/// A host's roots: the first step of a path names a member, the rest walk into it.
impl Scope for Map<String, Value> {
    fn resolve(&self, steps: &[Value]) -> Option<Value> {
        let (root, rest) = steps.split_first()?;
        walk(self.get(root.as_str()?)?, rest).cloned()
    }
}

/// Why a mapping did not evaluate.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A `$get` without `default` whose path selects nothing; the path as written.
    MissingPath(Value),
    /// An operand of the wrong type: the operator, where the operand sits in the
    /// mapping (keys and indexes from the mapping's root), and the value it had.
    InvalidOperand {
        operator: &'static str,
        path: Vec<Value>,
        value: Value,
    },
    /// `$json` or `$uuid` was given a value with no canonical form, such as one
    /// nested deeper than the codec allows.
    Unencodable(CanonError),
}

impl Error {
    /// The `ErrorInfo` code of a mapping error: `mapping.missing-path` or
    /// `mapping.invalid-operand`. An unencodable value is not a mapping error but a
    /// budget failure, `budget.value-depth`.
    pub fn code(&self) -> &'static str {
        match self {
            Error::MissingPath(_) => code::MAPPING_MISSING_PATH,
            Error::InvalidOperand { .. } => code::MAPPING_INVALID_OPERAND,
            Error::Unencodable(_) => crate::code::BUDGET_VALUE_DEPTH,
        }
    }

    /// The `ErrorInfo` details: `{"path"}` for a missing path, `{"operator", "path",
    /// "value"}` for an invalid operand. `prefix` goes in front of an operand's path,
    /// to say which mapping of several it sits in.
    pub fn details(&self, prefix: &[Value]) -> Value {
        match self {
            Error::MissingPath(path) => object([("path", path.clone())]),
            Error::InvalidOperand {
                operator,
                path,
                value,
            } => object([
                ("operator", Value::String((*operator).to_string())),
                (
                    "path",
                    Value::Array(prefix.iter().chain(path).cloned().collect()),
                ),
                ("value", value.clone()),
            ]),
            Error::Unencodable(error) => Value::String(error.to_string()),
        }
    }
}

/// Checks a host's mapping: the 0.2 language, with `$get` paths rooted at one of
/// `roots`. The text says what is wrong.
pub fn check(mapping: &Value, roots: &[&str]) -> Result<(), String> {
    check_with(mapping, Rules { v0_2: true, roots })
}

/// Evaluates a mapping that passed [`check`] against a host's scope. `operations`
/// counts the cost as in the kernel (`spec/kernel.md` §4).
pub fn evaluate(mapping: &Value, scope: &dyn Scope, operations: &mut u64) -> Result<Value, Error> {
    Evaluator {
        scope,
        operations,
        location: Vec::new(),
    }
    .evaluate(mapping)
}

/// Rule W10. A `$get` object has no other key, or in 0.2 also `default`; a
/// `$literal` object has no other key, nor (0.2) has an operator object; `$if` takes
/// three operands, `$uuid` one or more, `$join` two; no other key may start with
/// `$`; a path is a non-empty array of strings and non-negative integers whose first
/// step is `input`, `nodes`, or `run`, or in 0.2 also `from`. `$literal` and `default`
/// are free.
pub(crate) fn check_dialect(mapping: &Value, dialect: Dialect) -> Result<(), String> {
    check_with(mapping, dialect.rules())
}

fn check_with(mapping: &Value, rules: Rules<'_>) -> Result<(), String> {
    match mapping {
        Value::Object(map) => {
            if let Some(path) = map.get(GET) {
                let allowed = |key: &String| key == GET || (rules.v0_2 && key == DEFAULT);
                if !map.keys().all(allowed) {
                    return Err(if rules.v0_2 {
                        format!("a {GET} object takes only {GET} and {DEFAULT}")
                    } else {
                        format!("{GET} must be the only key of its object")
                    });
                }
                return check_path(path, rules);
            }
            if map.contains_key(LITERAL) {
                if map.len() != 1 {
                    return Err(format!("{LITERAL} must be the only key of its object"));
                }
                return Ok(());
            }
            if rules.v0_2 {
                if let Some((operator, operand)) = map
                    .iter()
                    .find(|(key, _)| matches!(key.as_str(), IF | UUID | JSON | JOIN))
                {
                    if map.len() != 1 {
                        return Err(format!("{operator} must be the only key of its object"));
                    }
                    return check_operands(operator, operand, rules);
                }
            }
            for (key, member) in map {
                if key.starts_with(OPERATOR_PREFIX) {
                    return Err(format!("unknown mapping operator {key:?}"));
                }
                check_with(member, rules)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|item| check_with(item, rules)),
        _ => Ok(()),
    }
}

fn check_operands(operator: &str, operand: &Value, rules: Rules<'_>) -> Result<(), String> {
    let list = |shape: &str, fits: fn(usize) -> bool| match operand.as_array() {
        Some(items) if fits(items.len()) => {
            items.iter().try_for_each(|item| check_with(item, rules))
        }
        _ => Err(format!("{operator} takes {shape}")),
    };
    match operator {
        IF => list("[cond, then, else]", |n| n == 3),
        UUID => list("a list of one or more parts", |n| n >= 1),
        JOIN => list("[list, separator]", |n| n == 2),
        _ => check_with(operand, rules),
    }
}

fn check_path(path: &Value, rules: Rules<'_>) -> Result<(), String> {
    let Some(steps) = path.as_array() else {
        return Err(format!("{GET} takes an array of steps"));
    };
    let rooted = steps
        .first()
        .and_then(Value::as_str)
        .is_some_and(|root| rules.roots.contains(&root));
    if !rooted {
        return Err(format!(
            "a {GET} path starts with {}",
            alternatives(rules.roots)
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

/// `"a", "b" or "c"`.
fn alternatives(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("{name:?}")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

/// What a `$get` path projects from in the kernel: the run input, the latest result
/// of each node, the run's identity, and the arrow the run took into the node being
/// entered.
pub(crate) struct Context<'a> {
    pub(crate) input: &'a Value,
    pub(crate) nodes: &'a BTreeMap<String, NodeResult>,
    pub(crate) run_id: &'a str,
    pub(crate) tenant: &'a str,
    /// The node whose recorded result routed the run here; `None` at the start node.
    pub(crate) from: Option<(&'a str, &'a NodeResult)>,
}

/// The context is the object `{"input": ..., "nodes": {id: {visit, outcome, output}},
/// "run": {id, tenant}, "from": {node, outcome, output} | null}`; only the part a path
/// reaches is materialized.
impl Scope for Context<'_> {
    fn resolve(&self, steps: &[Value]) -> Option<Value> {
        let (root, rest) = steps.split_first()?;
        match root.as_str()? {
            ROOT_INPUT => walk(self.input, rest).cloned(),
            ROOT_NODES => resolve_nodes(rest, self.nodes),
            ROOT_RUN => resolve_run(rest, self),
            ROOT_FROM => resolve_from(rest, self.from),
            _ => None,
        }
    }
}

/// Evaluates a mapping that passed W10. Each mapping value visited costs one
/// operation and each step of a `$get` path costs one more; a `$get` that falls back
/// to its `default` costs one more again; `$uuid` and `$join` cost one more per part
/// or item, `$json` one more per value it writes. Object members are evaluated in
/// canonical key order (UTF-16 code units), array elements and operands in index
/// order, so the first error is the same on every host.
struct Evaluator<'s, 'o> {
    scope: &'s dyn Scope,
    operations: &'o mut u64,
    /// Keys and indexes from the mapping's root to the value being evaluated.
    location: Vec<Value>,
}

impl Evaluator<'_, '_> {
    fn count(&mut self, cost: usize) {
        let cost = u64::try_from(cost).unwrap_or(u64::MAX);
        *self.operations = self.operations.saturating_add(cost);
    }

    fn evaluate(&mut self, mapping: &Value) -> Result<Value, Error> {
        self.count(1);
        match mapping {
            Value::Object(map) => {
                if let Some(path) = map.get(GET) {
                    let steps = path.as_array().map(Vec::as_slice).unwrap_or_default();
                    self.count(steps.len());
                    return match (self.scope.resolve(steps), map.get(DEFAULT)) {
                        (Some(value), _) => Ok(value),
                        (None, Some(fallback)) => {
                            self.count(1);
                            Ok(fallback.clone())
                        }
                        (None, None) => Err(Error::MissingPath(path.clone())),
                    };
                }
                if let Some(literal) = map.get(LITERAL) {
                    return Ok(literal.clone());
                }
                if map.len() == 1 {
                    if let Some((operator, operand)) = map.iter().next() {
                        match operator.as_str() {
                            IF => return self.operator(IF, operand, Self::choose),
                            UUID => return self.operator(UUID, operand, Self::uuid),
                            JSON => return self.operator(JSON, operand, Self::json),
                            JOIN => return self.operator(JOIN, operand, Self::join),
                            _ => {}
                        }
                    }
                }
                let mut members: Vec<(&String, &Value)> = map.iter().collect();
                members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                let mut evaluated = Map::new();
                for (key, member) in members {
                    let value = self.at(Value::String(key.clone()), member)?;
                    evaluated.insert(key.clone(), value);
                }
                Ok(Value::Object(evaluated))
            }
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| self.at(Value::from(index), item))
                .collect::<Result<Vec<Value>, Error>>()
                .map(Value::Array),
            scalar => Ok(scalar.clone()),
        }
    }

    /// Evaluates `mapping` one step below the current location.
    fn at(&mut self, step: Value, mapping: &Value) -> Result<Value, Error> {
        self.location.push(step);
        let value = self.evaluate(mapping);
        self.location.pop();
        value
    }

    /// Runs one operator with its key on the location.
    fn operator(
        &mut self,
        name: &'static str,
        operand: &Value,
        run: fn(&mut Self, &Value) -> Result<Value, Error>,
    ) -> Result<Value, Error> {
        self.location.push(Value::String(name.to_string()));
        let value = run(self, operand);
        self.location.pop();
        value
    }

    /// The operand at `index` of an operator's list, evaluated.
    fn operand(&mut self, operands: &Value, index: usize) -> Result<Value, Error> {
        let item = operands.get(index).unwrap_or(&Value::Null);
        self.at(Value::from(index), item)
    }

    /// An operand that does not fit, at the current location plus `steps`.
    fn invalid(&self, operator: &'static str, steps: &[usize], value: Value) -> Error {
        let mut path = self.location.clone();
        path.extend(steps.iter().map(|step| Value::from(*step)));
        Error::InvalidOperand {
            operator,
            path,
            value,
        }
    }

    /// `{"$if": [cond, then, else]}`: only the chosen branch is evaluated.
    fn choose(&mut self, operands: &Value) -> Result<Value, Error> {
        match self.operand(operands, 0)? {
            Value::Bool(true) => self.operand(operands, 1),
            Value::Bool(false) => self.operand(operands, 2),
            other => Err(self.invalid(IF, &[0], other)),
        }
    }

    /// `{"$uuid": [part, ...]}`: the version 5 UUID of the canonical JSON of the
    /// parts, each a string or a number.
    fn uuid(&mut self, operands: &Value) -> Result<Value, Error> {
        let count = operands.as_array().map_or(0, Vec::len);
        let mut parts = Vec::with_capacity(count);
        for index in 0..count {
            let part = self.operand(operands, index)?;
            if !(part.is_string() || part.is_number()) {
                return Err(self.invalid(UUID, &[index], part));
            }
            parts.push(part);
        }
        self.count(count);
        let name = canonical::to_vec(&Value::Array(parts)).map_err(Error::Unencodable)?;
        Ok(Value::String(ids::uuid(&name)))
    }

    /// `{"$json": value}`: the value as canonical JSON text.
    fn json(&mut self, operand: &Value) -> Result<Value, Error> {
        let value = self.evaluate(operand)?;
        self.count(size(&value));
        let text = canonical::to_vec(&value).map_err(Error::Unencodable)?;
        Ok(Value::String(String::from_utf8_lossy(&text).into_owned()))
    }

    /// `{"$join": [list, separator]}`: the list's strings joined by the separator.
    fn join(&mut self, operands: &Value) -> Result<Value, Error> {
        let list = self.operand(operands, 0)?;
        let Value::Array(items) = list else {
            return Err(self.invalid(JOIN, &[0], list));
        };
        let mut texts = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            match item.as_str() {
                Some(text) => texts.push(text),
                None => return Err(self.invalid(JOIN, &[0, index], item.clone())),
            }
        }
        let separator = self.operand(operands, 1)?;
        let Some(separator) = separator.as_str() else {
            return Err(self.invalid(JOIN, &[1], separator));
        };
        self.count(texts.len());
        Ok(Value::String(texts.join(separator)))
    }
}

/// How many JSON values `value` holds, itself included.
fn size(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.iter().map(size).fold(1, usize::saturating_add),
        Value::Object(map) => map.values().map(size).fold(1, usize::saturating_add),
        _ => 1,
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
