//! Schemas (`spec/schema.md`): the JSON Schema subset a workflow may declare per
//! outcome and for a run's input, the checks of a schema itself, and the check of a
//! step's data or a run's input.
//!
//! Shared by the build, which inlines `$ref`s and checks schemas, and the host, which
//! checks schemas in `validate_workflow`, every result against its outcome's schema,
//! and a run's input against the input schema when the run starts.

use std::collections::BTreeSet;
use std::fmt;

use runspore_types::canonical::{self, MAX_DEPTH, MAX_SAFE_INT};
use serde_json::{Map, Value};

/// The level of a schema's root object in a package: the package, `nodes`, the node,
/// `outputSchemas`, the schema.
pub const ROOT_LEVEL: usize = 5;
/// The level of the input schema's root object in a package: the package, the schema.
pub const INPUT_LEVEL: usize = 2;
/// The largest result a step may report, in bytes; a schema must admit a smaller value.
pub const MAX_RESULT_BYTES: u64 = 65_536;
/// The most errors one check reports.
pub const MAX_ERRORS: usize = 16;
/// The largest `minLength`, `maxLength`, `minItems` or `maxItems`.
pub const MAX_BOUND: i64 = 65_536;

/// The error code of a schema outside the subset.
pub const INVALID_SCHEMA: &str = "workflow.invalid-schema";
/// The error code of data that does not fit its schema.
pub const OUTPUT_INVALID: &str = "output.invalid";
/// The error code of a run's input that does not fit the workflow's input schema.
pub const INPUT_INVALID: &str = "input.invalid";

const TYPES: [&str; 6] = ["object", "array", "string", "integer", "boolean", "null"];
const ANNOTATIONS: [&str; 2] = ["description", "title"];
const DEFS_PREFIX: &str = "#/$defs/";

/// What is wrong with a schema: the JSON Pointer of the keyword, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", shown(&self.path), self.message)
    }
}

/// One place where data does not fit: the JSON Pointer into the data, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", shown(&self.path), self.message)
    }
}

/// Data refused for an outcome: `output.invalid` with details `{outcome, errors}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misfit {
    pub outcome: String,
    pub errors: Vec<DataError>,
}

impl Misfit {
    /// One line naming the outcome.
    pub fn summary(&self) -> String {
        format!(
            "the data for outcome {:?} does not fit its schema",
            self.outcome
        )
    }

    /// The details of an `output.invalid` error.
    pub fn details(&self) -> Value {
        let errors = self
            .errors
            .iter()
            .map(|e| serde_json::json!({"path": e.path, "message": e.message}))
            .collect();
        serde_json::json!({"outcome": self.outcome, "errors": Value::Array(errors)})
    }

    /// The summary and every error on one line, for an error message that is stored.
    pub fn message(&self) -> String {
        let errors: Vec<String> = self.errors.iter().map(ToString::to_string).collect();
        format!("{}: {}", self.summary(), errors.join("; "))
    }
}

/// The summary, then one error per line.
impl fmt::Display for Misfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())?;
        for error in &self.errors {
            write!(f, "\n{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Misfit {}

/// A run's input refused: `input.invalid` with details `{errors}` (spec/schema.md §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputMisfit {
    pub errors: Vec<DataError>,
}

impl InputMisfit {
    /// One line naming what was checked.
    pub fn summary(&self) -> &'static str {
        "the input does not fit the workflow's inputSchema"
    }

    /// The details of an `input.invalid` error.
    pub fn details(&self) -> Value {
        let errors = self
            .errors
            .iter()
            .map(|e| serde_json::json!({"path": e.path, "message": e.message}))
            .collect();
        serde_json::json!({"errors": Value::Array(errors)})
    }
}

/// The summary, then one error per line.
impl fmt::Display for InputMisfit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.summary())?;
        for error in &self.errors {
            write!(f, "\n{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for InputMisfit {}

/// A path as people read it: `(root)` for the whole value.
pub fn shown(path: &str) -> &str {
    if path.is_empty() {
        "(root)"
    } else {
        path
    }
}

/// One JSON Pointer step (RFC 6901): `~` becomes `~0`, `/` becomes `~1`.
pub fn step(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}

/// Levels of containers in `value`: 0 for a scalar, 1 for `{}`, as the package's depth
/// limit counts them (kernel.md §1).
pub fn nesting(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(nesting).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(nesting).max().unwrap_or(0),
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// The authoring form: `$defs` and `$ref`
// ---------------------------------------------------------------------------

/// Turns a schema as written in a workflow folder into its package form: every
/// `$ref` replaced by the definition it names, and `$defs` removed.
pub fn inline(schema: &Value) -> Result<Value, SchemaError> {
    let defs = match schema.get("$defs") {
        None => Map::new(),
        Some(Value::Object(defs)) => defs.clone(),
        Some(_) => return Err(error("/$defs", "an object of named schemas")),
    };
    let inliner = Inliner { defs: &defs };
    for (name, def) in &defs {
        let path = step("/$defs", name);
        inliner.value(def, &path, &mut vec![name.clone()])?;
    }
    let mut root = schema.clone();
    if let Value::Object(map) = &mut root {
        map.remove("$defs");
    }
    inliner.value(&root, "", &mut Vec::new())
}

struct Inliner<'a> {
    defs: &'a Map<String, Value>,
}

impl Inliner<'_> {
    /// A copy of `value` with its references replaced; `stack` holds the definitions
    /// being expanded, so a reference back into one of them is a loop.
    fn value(
        &self,
        value: &Value,
        path: &str,
        stack: &mut Vec<String>,
    ) -> Result<Value, SchemaError> {
        match value {
            Value::Object(map) if map.contains_key("$ref") => self.reference(map, path, stack),
            Value::Object(map) => {
                if map.contains_key("$defs") {
                    return Err(error(
                        &step(path, "$defs"),
                        "`$defs` belongs at the root of a schema",
                    ));
                }
                let mut out = Map::new();
                for (key, item) in map {
                    let item = match key.as_str() {
                        // Data, not schemas: copied as they are.
                        "enum" | "const" | "required" => item.clone(),
                        _ => self.value(item, &step(path, key), stack)?,
                    };
                    out.insert(key.clone(), item);
                }
                Ok(Value::Object(out))
            }
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, item)| self.value(item, &step(path, &i.to_string()), stack))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            other => Ok(other.clone()),
        }
    }

    fn reference(
        &self,
        map: &Map<String, Value>,
        path: &str,
        stack: &mut Vec<String>,
    ) -> Result<Value, SchemaError> {
        let at = step(path, "$ref");
        let Some(Value::String(target)) = map.get("$ref") else {
            return Err(error(&at, "`$ref` is a string, `#/$defs/<name>`"));
        };
        if let Some(key) = map
            .keys()
            .find(|k| *k != "$ref" && !ANNOTATIONS.contains(&k.as_str()))
        {
            return Err(error(
                &step(path, key),
                "`$ref` stands alone; only `description` and `title` may sit beside it",
            ));
        }
        let Some(name) = target.strip_prefix(DEFS_PREFIX) else {
            let why = if target.starts_with('#') {
                format!("{target:?}: a reference names `#/$defs/<name>`")
            } else {
                format!("{target:?}: remote references are not allowed")
            };
            return Err(error(&at, why));
        };
        let Some(def) = self.defs.get(name) else {
            return Err(error(&at, format!("{target:?}: `$defs` has no {name:?}")));
        };
        if stack.iter().any(|s| s == name) {
            return Err(error(
                &at,
                format!("{target:?} refers back to itself; a schema may not recurse"),
            ));
        }
        stack.push(name.to_string());
        let inlined = self.value(def, &step("/$defs", name), stack);
        stack.pop();
        let mut inlined = inlined?;
        if let Value::Object(out) = &mut inlined {
            for key in ANNOTATIONS {
                if let Some(text) = map.get(key) {
                    out.insert(key.to_string(), text.clone());
                }
            }
        }
        Ok(inlined)
    }
}

// ---------------------------------------------------------------------------
// The subset
// ---------------------------------------------------------------------------

/// Checks a schema in its package form whose root sits at `level` of the package
/// ([`ROOT_LEVEL`] for an `outputSchemas` entry): only the subset's keywords, the
/// package's depth limit, and a smallest admitted value within the result limit.
pub fn check(schema: &Value, level: usize) -> Result<(), SchemaError> {
    let room = MAX_DEPTH + 1 - level.min(MAX_DEPTH + 1);
    if nesting(schema) > room {
        return Err(error(
            "",
            format!(
                "nests {} levels, and here the package has room for {room} (32 levels from \
                 the package root)",
                nesting(schema)
            ),
        ));
    }
    keywords(schema, "")?;
    let smallest = smallest(schema);
    if smallest > MAX_RESULT_BYTES {
        return Err(error(
            "",
            format!(
                "the smallest value it admits is {smallest} bytes, above the result limit of \
                 {MAX_RESULT_BYTES}"
            ),
        ));
    }
    Ok(())
}

/// Checks an input schema (spec/schema.md §4): the subset at [`INPUT_LEVEL`], and a
/// root of `"type": "object"`, so a run's input always has named fields.
pub fn check_input(schema: &Value) -> Result<(), SchemaError> {
    check(schema, INPUT_LEVEL)?;
    match schema.get("type") {
        Some(Value::String(t)) if t == "object" => Ok(()),
        _ => Err(error(
            "/type",
            "an input schema's root must be `\"type\": \"object\"`",
        )),
    }
}

fn error(path: &str, message: impl Into<String>) -> SchemaError {
    SchemaError {
        path: path.to_string(),
        message: message.into(),
    }
}

/// The keyword rules of the subset, for one schema and the schemas inside it.
fn keywords(schema: &Value, path: &str) -> Result<(), SchemaError> {
    let Value::Object(map) = schema else {
        return Err(error(path, "a schema is a JSON object"));
    };
    let at = |key: &str| step(path, key);
    let ty = match map.get("type") {
        None => None,
        Some(Value::String(t)) if TYPES.contains(&t.as_str()) => Some(t.as_str()),
        Some(Value::String(t)) if t == "number" => {
            return Err(error(
                &at("type"),
                "`number` is not allowed: data holds integers only; use `integer`",
            ))
        }
        Some(Value::Array(_)) => {
            return Err(error(
                &at("type"),
                "a single type name; write a union with `anyOf`",
            ))
        }
        Some(other) => {
            return Err(error(
                &at("type"),
                format!("{other} is not one of {}", TYPES.join(", ")),
            ))
        }
    };
    for (key, value) in map {
        let here = at(key);
        let needs = |wanted: &str| -> Result<(), SchemaError> {
            if ty == Some(wanted) {
                Ok(())
            } else {
                Err(error(
                    &here,
                    format!("`{key}` applies only beside `\"type\": \"{wanted}\"`"),
                ))
            }
        };
        match key.as_str() {
            "type" => {}
            "description" | "title" => {
                if !value.is_string() {
                    return Err(error(&here, "a string"));
                }
            }
            "anyOf" => {
                if let Some(other) = map
                    .keys()
                    .find(|k| *k != "anyOf" && !ANNOTATIONS.contains(&k.as_str()))
                {
                    return Err(error(
                        &at(other),
                        "`anyOf` stands alone; only `description` and `title` may sit beside it",
                    ));
                }
                match value {
                    Value::Array(branches) if !branches.is_empty() => {
                        for (i, branch) in branches.iter().enumerate() {
                            keywords(branch, &step(&here, &i.to_string()))?;
                        }
                    }
                    _ => return Err(error(&here, "a non-empty list of schemas")),
                }
            }
            "properties" => {
                needs("object")?;
                let Value::Object(properties) = value else {
                    return Err(error(&here, "an object of property schemas"));
                };
                for (name, property) in properties {
                    keywords(property, &step(&here, name))?;
                }
            }
            "required" => {
                needs("object")?;
                let Value::Array(names) = value else {
                    return Err(error(&here, "a list of property names"));
                };
                let properties = map.get("properties").and_then(Value::as_object);
                let mut seen = BTreeSet::new();
                for (i, name) in names.iter().enumerate() {
                    let item = step(&here, &i.to_string());
                    let Value::String(name) = name else {
                        return Err(error(&item, "a property name"));
                    };
                    if !seen.insert(name) {
                        return Err(error(&item, format!("{name:?} is listed twice")));
                    }
                    if !properties.is_some_and(|p| p.contains_key(name)) {
                        return Err(error(
                            &item,
                            format!("{name:?} is not a key of `properties`"),
                        ));
                    }
                }
            }
            "additionalProperties" => {
                needs("object")?;
                if value != &Value::Bool(false) {
                    return Err(error(
                        &here,
                        "only `false`; leave it out to allow other keys",
                    ));
                }
            }
            "items" => {
                needs("array")?;
                keywords(value, &here)?;
            }
            "minItems" | "maxItems" => {
                needs("array")?;
                bound(value, &here)?;
            }
            "minLength" | "maxLength" => {
                needs("string")?;
                bound(value, &here)?;
            }
            "minimum" | "maximum" => {
                needs("integer")?;
                if !value
                    .as_i64()
                    .is_some_and(|v| (-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&v))
                {
                    return Err(error(&here, "an integer within ±(2^53 − 1)"));
                }
            }
            "enum" => {
                let Value::Array(values) = value else {
                    return Err(error(&here, "a non-empty list of values"));
                };
                if values.is_empty() {
                    return Err(error(&here, "a non-empty list of values"));
                }
                for (i, item) in values.iter().enumerate() {
                    let item_at = step(&here, &i.to_string());
                    scalar(item, &item_at)?;
                    if values[..i].contains(item) {
                        return Err(error(&item_at, format!("{item} is listed twice")));
                    }
                }
            }
            "const" => scalar(value, &here)?,
            "$ref" | "$defs" => {
                return Err(error(
                    &here,
                    format!("`{key}` is resolved by the build; a package holds none"),
                ))
            }
            _ => return Err(error(&here, format!("`{key}` is not in the schema subset"))),
        }
    }
    for (low, high) in [
        ("minLength", "maxLength"),
        ("minItems", "maxItems"),
        ("minimum", "maximum"),
    ] {
        let (Some(l), Some(h)) = (
            map.get(low).and_then(Value::as_i64),
            map.get(high).and_then(Value::as_i64),
        ) else {
            continue;
        };
        if l > h {
            return Err(error(
                &at(low),
                format!("admits no value: `{low}` {l} is above `{high}` {h}"),
            ));
        }
    }
    Ok(())
}

fn bound(value: &Value, path: &str) -> Result<(), SchemaError> {
    if value.as_i64().is_some_and(|v| (0..=MAX_BOUND).contains(&v)) {
        Ok(())
    } else {
        Err(error(path, format!("an integer from 0 to {MAX_BOUND}")))
    }
}

fn scalar(value: &Value, path: &str) -> Result<(), SchemaError> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(()),
        _ => Err(error(path, "a string, an integer, a boolean or null")),
    }
}

/// The serialized size in bytes of the smallest value a checked schema admits
/// (spec/schema.md §3). A lower bound: it counts every string character as one byte.
pub fn smallest(schema: &Value) -> u64 {
    let get = |key: &str| schema.get(key);
    let int = |key: &str| get(key).and_then(Value::as_i64);
    if let Some(Value::Array(branches)) = get("anyOf") {
        return branches.iter().map(smallest).min().unwrap_or(0);
    }
    if let Some(value) = get("const") {
        return written(value);
    }
    if let Some(Value::Array(values)) = get("enum") {
        return values.iter().map(written).min().unwrap_or(0);
    }
    match get("type").and_then(Value::as_str) {
        Some("null") => 4,
        Some("boolean") => 4,
        Some("integer") => {
            let (low, high) = (int("minimum"), int("maximum"));
            match (low, high) {
                (Some(l), _) if l > 0 => l.to_string().len() as u64,
                (_, Some(h)) if h < 0 => h.to_string().len() as u64,
                _ => 1,
            }
        }
        Some("string") => 2 + int("minLength").unwrap_or(0) as u64,
        Some("array") => {
            let count = int("minItems").unwrap_or(0) as u64;
            if count == 0 {
                return 2;
            }
            let item = get("items").map_or(1, smallest);
            (2 + count.saturating_mul(item.saturating_add(1))).saturating_sub(1)
        }
        Some("object") => {
            let properties = get("properties").and_then(Value::as_object);
            let required: Vec<&str> = get("required")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let members = required.iter().fold(0u64, |sum, name| {
                let value = properties.and_then(|p| p.get(*name)).map_or(1, smallest);
                sum.saturating_add(written(&Value::String(name.to_string())) + 1)
                    .saturating_add(value)
            });
            2 + members + (required.len() as u64).saturating_sub(1)
        }
        // Anything: `0` is one byte.
        _ => 1,
    }
}

fn written(value: &Value) -> u64 {
    canonical::to_vec(value).map_or(u64::MAX, |bytes| bytes.len() as u64)
}

// ---------------------------------------------------------------------------
// Checking data
// ---------------------------------------------------------------------------

/// Checks `data` against a checked schema. Returns every place it does not fit, up to
/// [`MAX_ERRORS`], in document order; empty when it fits.
pub fn validate(schema: &Value, data: &Value) -> Vec<DataError> {
    let mut errors = Vec::new();
    walk(schema, data, "", &mut errors);
    errors.truncate(MAX_ERRORS);
    errors
}

/// Checks the data an outcome carries against `schema`.
pub fn fit(outcome: &str, schema: &Value, data: &Value) -> Result<(), Misfit> {
    let errors = validate(schema, data);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Misfit {
            outcome: outcome.to_string(),
            errors,
        })
    }
}

/// Checks a run's input against the workflow's input schema.
pub fn fit_input(schema: &Value, input: &Value) -> Result<(), InputMisfit> {
    let errors = validate(schema, input);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(InputMisfit { errors })
    }
}

fn walk(schema: &Value, data: &Value, path: &str, errors: &mut Vec<DataError>) {
    if errors.len() >= MAX_ERRORS {
        return;
    }
    let get = |key: &str| schema.get(key);
    let int = |key: &str| get(key).and_then(Value::as_i64);

    if let Some(Value::Array(branches)) = get("anyOf") {
        if !branches.iter().any(|b| validate(b, data).is_empty()) {
            push(
                errors,
                path,
                format!(
                    "fits none of the {} forms `anyOf` allows, got {}",
                    branches.len(),
                    describe(data)
                ),
            );
        }
        return;
    }
    if let Some(ty) = get("type").and_then(Value::as_str) {
        if type_of(data) != ty {
            push(
                errors,
                path,
                format!("expected type {ty:?}, got {}", describe(data)),
            );
            return;
        }
    }
    if let Some(expected) = get("const") {
        if expected != data {
            push(
                errors,
                path,
                format!("expected {expected}, got {}", describe(data)),
            );
        }
    }
    if let Some(Value::Array(values)) = get("enum") {
        if !values.contains(data) {
            let listed: Vec<String> = values.iter().map(ToString::to_string).collect();
            push(
                errors,
                path,
                format!(
                    "expected one of {}, got {}",
                    listed.join(", "),
                    describe(data)
                ),
            );
        }
    }
    match data {
        Value::String(text) => {
            let length = text.chars().count() as i64;
            if let Some(min) = int("minLength").filter(|min| length < *min) {
                push(
                    errors,
                    path,
                    format!("expected at least {min} characters, got {length}"),
                );
            }
            if let Some(max) = int("maxLength").filter(|max| length > *max) {
                push(
                    errors,
                    path,
                    format!("expected at most {max} characters, got {length}"),
                );
            }
        }
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                if let Some(min) = int("minimum").filter(|min| value < *min) {
                    push(
                        errors,
                        path,
                        format!("expected at least {min}, got {value}"),
                    );
                }
                if let Some(max) = int("maximum").filter(|max| value > *max) {
                    push(errors, path, format!("expected at most {max}, got {value}"));
                }
            }
        }
        Value::Array(items) => {
            let count = items.len() as i64;
            if let Some(min) = int("minItems").filter(|min| count < *min) {
                push(
                    errors,
                    path,
                    format!("expected at least {min} items, got {count}"),
                );
            }
            if let Some(max) = int("maxItems").filter(|max| count > *max) {
                push(
                    errors,
                    path,
                    format!("expected at most {max} items, got {count}"),
                );
            }
            if let Some(item_schema) = get("items") {
                for (i, item) in items.iter().enumerate() {
                    walk(item_schema, item, &step(path, &i.to_string()), errors);
                }
            }
        }
        Value::Object(members) => {
            let properties = get("properties").and_then(Value::as_object);
            for name in get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !members.contains_key(name) {
                    push(
                        errors,
                        &step(path, name),
                        "required, but missing".to_string(),
                    );
                }
            }
            let closed = get("additionalProperties") == Some(&Value::Bool(false));
            for (name, value) in members {
                match properties.and_then(|p| p.get(name)) {
                    Some(property) => walk(property, value, &step(path, name), errors),
                    None if closed => {
                        let allowed: Vec<&str> = properties
                            .map(|p| p.keys().map(String::as_str).collect())
                            .unwrap_or_default();
                        let allowed = if allowed.is_empty() {
                            "none".to_string()
                        } else {
                            allowed.join(", ")
                        };
                        push(
                            errors,
                            &step(path, name),
                            format!("not allowed; allowed properties: {allowed}"),
                        );
                    }
                    None => {}
                }
            }
        }
        _ => {}
    }
}

fn push(errors: &mut Vec<DataError>, path: &str, message: String) {
    errors.push(DataError {
        path: path.to_string(),
        message,
    });
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A value's type, and the value itself when it is short to show.
fn describe(value: &Value) -> String {
    const SHOWN: usize = 40;
    match value {
        Value::Null => "null".to_string(),
        Value::Array(_) | Value::Object(_) => type_of(value).to_string(),
        Value::String(text) if text.chars().count() > SHOWN => {
            let start: String = text.chars().take(SHOWN).collect();
            format!("string ({:?}…)", start)
        }
        other => format!("{} ({other})", type_of(other)),
    }
}
