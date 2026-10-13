//! `settings.json` as Rust types (spec/dot.md §4), the JSON Schema generated from
//! them, and its errors as diagnostics. The schema checks the shape, with every
//! error found at once; the build's own checks (`checks.rs`) do what spans fields
//! and files.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::diagnostic::{pointer, steps, Applicability, Arg, At, Diagnostic, Edit, Fix, Found};
use crate::near::closest;

/// Where the schema is published; `settings.json` may name it in `$schema`.
pub const SCHEMA_ADDRESS: &str =
    "https://raw.githubusercontent.com/Web-tree/runspore/main/spec/settings.schema.json";

/// `[a-z0-9][a-z0-9.-]{0,63}`: a fail node's error code (kernel.md W11).
const ERROR_CODE: &str = "^[a-z0-9][a-z0-9.-]{0,63}$";
/// `[A-Za-z0-9_.-]{1,64}`: a signal name (kernel.md W09).
const SIGNAL: &str = "^[A-Za-z0-9_.-]{1,64}$";
/// A count of milliseconds as a canonical decimal string.
const DECIMAL: &str = "^(0|[1-9][0-9]{0,18})$";

/// The settings of a workflow folder: the package's own shape minus its topology,
/// which `workflow.dot` holds.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Runspore workflow settings (settings.json)")]
pub struct Settings {
    /// This schema's address, for editors.
    #[serde(rename = "$schema", default)]
    #[schemars(with = "String")]
    pub schema: Option<String>,
    /// The package format; it agrees with the dialect of `workflow.dot`.
    pub format: Format,
    #[serde(default)]
    #[schemars(with = "Limits")]
    pub limits: Option<Limits>,
    /// Actions by ID: what a step runs.
    #[serde(default)]
    pub actions: BTreeMap<String, Action>,
    /// Steps by the ID `workflow.dot` gives them: their fields other than kind and
    /// arrows.
    #[serde(default)]
    pub nodes: BTreeMap<String, Step>,
    /// The shape a run's input must have: a schema object whose root is
    /// `"type": "object"`, or the path of a JSON file holding one.
    #[serde(default, rename = "inputSchema")]
    #[schemars(with = "SchemaOrFile")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum Format {
    #[serde(rename = "runspore.workflow/0.2")]
    V0_2,
}

/// Bounds on every loop.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Limits {
    /// Entries allowed into any one step. Default 16.
    #[serde(default)]
    #[schemars(with = "u32", range(min = 1, max = 1000))]
    pub max_visits_per_node: Option<u32>,
    /// Step entries allowed across the whole run. Default 256.
    #[serde(default)]
    #[schemars(with = "u32", range(min = 1, max = 10000))]
    pub max_activations: Option<u32>,
}

/// What an action does when a step runs it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[schemars(transform = kind_needs)]
pub struct Action {
    pub kind: Kind,
    /// Whether a step may run again after a crash: `pure`, `read-only` and
    /// `idempotent` may, `unsafe` waits for a person.
    pub effect: Effect,
    /// The outcomes a run of the action ends with. Default `["ok"]`.
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    pub outcomes: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(with = "Retry")]
    pub retry: Option<Retry>,
    /// Files of the folder the action uses, sealed into the workflow artifact.
    #[serde(default)]
    #[schemars(with = "Vec<FileRef>")]
    pub assets: Option<Vec<String>>,
    /// `command`: the program and its arguments. No shell: each item is one word.
    #[serde(default)]
    #[schemars(with = "Vec<String>", length(min = 1))]
    pub argv: Option<Vec<String>>,
    /// `command`: the folder to run in, relative to the run's base folder.
    #[serde(default)]
    #[schemars(with = "String")]
    pub cwd: Option<String>,
    /// `command`: extra environment variables.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, String>")]
    pub env: Option<BTreeMap<String, String>>,
    /// `command`: how long it may run, in milliseconds.
    #[serde(default)]
    #[schemars(with = "Millis")]
    pub timeout_ms: Option<Value>,
    /// `command`: how its output becomes the step's result. Default `text`.
    #[serde(default)]
    #[schemars(with = "Output")]
    pub output: Option<Output>,
    /// `command`: the outcome each exit code other than 0 ends with.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, String>")]
    pub exit_outcomes: Option<BTreeMap<String, String>>,
    /// `command`: an asynchronous action's probe, watch and stop programs.
    #[serde(default, rename = "async")]
    #[schemars(with = "BTreeMap<String, Value>")]
    pub asynchronous: Option<Value>,
    /// `native`: the function the host registered under this name.
    #[serde(default)]
    #[schemars(with = "String")]
    pub function: Option<String>,
}

/// A `command` action needs `argv`; a `native` one needs `function`.
fn kind_needs(schema: &mut schemars::Schema) {
    let needs = |kind: &str, field: &str| {
        json!({"if": {"properties": {"kind": {"const": kind}}, "required": ["kind"]},
               "then": {"required": [field]}})
    };
    schema.insert(
        "allOf".into(),
        json!([needs("command", "argv"), needs("native", "function")]),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Command,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    Pure,
    ReadOnly,
    Idempotent,
    Unsafe,
}

/// Every effect, in the order a fix offers them.
pub const EFFECTS: [&str; 4] = ["pure", "read-only", "idempotent", "unsafe"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Output {
    Text,
    Json,
    Result,
    None,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Retry {
    /// Tries in all, the first included. Default 1.
    #[serde(default)]
    #[schemars(with = "u32", range(min = 1, max = 100))]
    pub max_attempts: Option<u32>,
    /// The wait before the second try, doubled for each one after, in milliseconds.
    #[serde(default)]
    #[schemars(with = "Decimal")]
    pub backoff_ms: Option<String>,
    /// The longest wait between tries, in milliseconds. Default "60000".
    #[serde(default)]
    #[schemars(with = "Decimal")]
    pub max_backoff_ms: Option<String>,
}

/// A count of milliseconds written as a string of digits, such as `"500"`.
#[derive(JsonSchema)]
#[schemars(transform = decimal)]
#[allow(dead_code)]
struct Decimal(String);

fn decimal(schema: &mut schemars::Schema) {
    schema.insert("pattern".into(), json!(DECIMAL));
}

/// Milliseconds: a whole number, or one written as a string of digits.
#[derive(JsonSchema)]
#[schemars(transform = millis)]
#[allow(dead_code)]
struct Millis(u64);

fn millis(schema: &mut schemars::Schema) {
    *schema = schemars::json_schema!({
        "description": "Milliseconds: a whole number, or one written as a string of digits.",
        "type": ["integer", "string"],
        "minimum": 0,
        "pattern": DECIMAL
    });
}

/// A path to a file inside the workflow folder.
#[derive(JsonSchema)]
#[schemars(transform = file_ref)]
#[allow(dead_code)]
struct FileRef(String);

fn file_ref(schema: &mut schemars::Schema) {
    schema.insert("minLength".into(), json!(1));
}

/// A step's fields. Which it may carry depends on its kind in `workflow.dot`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Step {
    /// `activity`: the action it runs.
    #[serde(default)]
    #[schemars(with = "String")]
    pub action: Option<String>,
    /// `activity`: its input, a mapping over the run.
    #[serde(default)]
    pub input: Option<Value>,
    /// `activity`, `await-signal`: a file of instructions for whoever does the step.
    #[serde(default)]
    #[schemars(with = "FileRef")]
    pub instructions: Option<String>,
    /// `activity`, `await-signal`: by outcome, the schema its output fits, inline or
    /// as the path of a JSON file.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, SchemaOrFile>", extend("minProperties" = 1))]
    pub output_schemas: Option<BTreeMap<String, Value>>,
    /// `activity`, `await-signal`: by outcome, how many times in a row it may end
    /// with it before it ends with `exhausted`.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, LoopLimit>", extend("minProperties" = 1))]
    pub loop_limits: Option<BTreeMap<String, u32>>,
    /// `await-signal`: the signal it waits for.
    #[serde(default)]
    #[schemars(with = "Signal")]
    pub signal: Option<String>,
    /// `complete`: the run's output, a mapping over the run.
    #[serde(default)]
    pub output: Option<Value>,
    /// `fail`: the error the run ends with.
    #[serde(default)]
    #[schemars(with = "Failure")]
    pub error: Option<Value>,
}

/// A schema object, or the path of a JSON file holding one.
#[derive(JsonSchema)]
#[schemars(transform = schema_or_file)]
#[allow(dead_code)]
struct SchemaOrFile(Value);

fn schema_or_file(schema: &mut schemars::Schema) {
    *schema = schemars::json_schema!({
        "description": "A schema object, or the path of a JSON file holding one.",
        "type": ["object", "string"],
        "minLength": 1
    });
}

/// A loop limit: 1 to 999.
#[derive(JsonSchema)]
#[allow(dead_code)]
struct LoopLimit(#[schemars(range(min = 1, max = 999))] u32);

#[derive(JsonSchema)]
#[schemars(transform = signal)]
#[allow(dead_code)]
struct Signal(String);

fn signal(schema: &mut schemars::Schema) {
    schema.insert("pattern".into(), json!(SIGNAL));
}

/// The error a `fail` step ends the run with.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[allow(dead_code)]
struct Failure {
    #[schemars(regex(pattern = ERROR_CODE))]
    code: String,
    message: String,
    /// Always null here: the run fills it in.
    #[serde(default)]
    node_id: (),
    #[serde(default)]
    details: Option<Value>,
}

/// The schema, generated from [`Settings`].
pub fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut settings = schemars::generate::SchemaSettings::draft2020_12();
        // An optional field has no default worth stating; `null` would mislead.
        settings
            .transforms
            .push(Box::new(schemars::transform::RecursiveTransform(
                |schema: &mut schemars::Schema| {
                    if schema.get("default") == Some(&Value::Null) {
                        schema.remove("default");
                    }
                },
            )));
        let generator = settings.into_generator();
        let root = generator.into_root_schema_for::<Settings>();
        let mut value = root.to_value();
        if let Some(map) = value.as_object_mut() {
            map.insert("$id".into(), json!(SCHEMA_ADDRESS));
        }
        value
    })
}

/// The schema as the file `spec/settings.schema.json` holds it.
pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(schema()).expect("a schema serializes");
    text.push('\n');
    text
}

fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        jsonschema::draft202012::new(schema()).expect("the generated schema compiles")
    })
}

/// The fields a definition of the schema allows, by its name in `$defs` (`""` for
/// the top level).
pub fn fields(def: &str) -> Vec<String> {
    let schema = schema();
    let object = if def.is_empty() {
        Some(schema)
    } else {
        schema.get("$defs").and_then(|d| d.get(def))
    };
    object
        .and_then(|o| o.get("properties"))
        .and_then(Value::as_object)
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default()
}

/// The definition an instance pointer of `settings.json` falls under.
fn def_of(at: &str) -> &'static str {
    match steps(at).iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => "",
        ["limits"] => "Limits",
        ["actions", _] => "Action",
        ["actions", _, "retry"] => "Retry",
        ["nodes", _] => "Step",
        ["nodes", _, "error"] => "Failure",
        _ => "?",
    }
}

/// The JSON type of a value, as a message names it.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

fn type_name(t: &str) -> &'static str {
    match t {
        "object" => "an object",
        "array" => "a list",
        "string" => "a string",
        "integer" => "a whole number",
        "number" => "a number",
        "boolean" => "true or false",
        "null" => "null",
        _ => "another type",
    }
}

/// What the schema found, by pointer, so the build's own checks skip broken parts.
#[derive(Debug, Default)]
pub struct Broken(BTreeSet<String>);

impl Broken {
    pub fn add(&mut self, at: &str) {
        self.0.insert(at.to_string());
    }

    /// Whether `at`, a part of it, or something it is part of has a problem.
    pub fn touches(&self, at: &str) -> bool {
        self.0.iter().any(|b| {
            b == at
                || (b.starts_with(at) && b[at.len()..].starts_with('/'))
                || (at.starts_with(b.as_str()) && at[b.len()..].starts_with('/'))
                || b.is_empty()
        })
    }

    /// Whether `at` itself, or something it is part of, has a problem.
    pub fn within(&self, at: &str) -> bool {
        self.0.iter().any(|b| {
            b == at
                || b.is_empty()
                || (at.starts_with(b.as_str()) && at[b.len()..].starts_with('/'))
        })
    }
}

/// Checks `value` against the schema. Each error becomes a diagnostic, except one
/// that follows from another: a value the build already found broken, or a required
/// key missing because it is there under a misspelt name.
pub fn shape(value: &Value, found: &mut Found, broken: &mut Broken) {
    use jsonschema::error::ValidationErrorKind as K;
    let mut seen = BTreeSet::new();
    let mut errors: Vec<_> = validator().iter_errors(value).collect();
    errors.sort_by_key(|e| e.instance_path().as_str().to_string());
    let mut misspelt: BTreeSet<(String, String)> = BTreeSet::new();
    for e in &errors {
        if let K::AdditionalProperties { unexpected } = e.kind() {
            let at = e.instance_path().as_str();
            let def =
                schema_def(e.schema_path().as_str()).unwrap_or_else(|| def_of(at).to_string());
            let allowed = fields(&def);
            for key in unexpected {
                if let Some(near) = closest(key, allowed.iter().map(String::as_str)) {
                    misspelt.insert((at.to_string(), near.to_string()));
                }
            }
        }
    }
    let already = |at: &str, broken: &Broken| broken.within(at);
    for e in errors {
        let at = e.instance_path().as_str().to_string();
        if let K::Required { property } = e.kind() {
            let field = property.as_str().unwrap_or_default().to_string();
            if misspelt.contains(&(at.clone(), field)) {
                continue;
            }
        } else if already(&at, broken) {
            continue;
        }
        let schema_path = e.schema_path().as_str().to_string();
        for d in diagnose(value, &at, &schema_path, e.kind(), broken) {
            if seen.insert((d.code, d.pointer().unwrap_or_default().to_string())) {
                found.push(d);
            }
        }
    }
}

fn diagnose(
    value: &Value,
    at: &str,
    schema_path: &str,
    kind: &jsonschema::error::ValidationErrorKind,
    broken: &mut Broken,
) -> Vec<Diagnostic> {
    use jsonschema::error::ValidationErrorKind as K;
    let here = value.pointer(at).cloned().unwrap_or(Value::Null);
    let path: Vec<String> = steps(at);
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let input = || Arg::Input(at.to_string());
    let place = || Arg::Place(at.to_string());
    match kind {
        K::AdditionalProperties { unexpected } => unexpected
            .iter()
            .map(|key| {
                let child = pointer(at, key);
                broken.add(&child);
                unexpected_key(value, at, key, schema_path)
            })
            .collect(),
        K::Required { property } => {
            let field = property.as_str().unwrap_or_default();
            broken.add(&pointer(at, field));
            vec![missing(value, at, field)]
        }
        K::Enum { options }
        | K::Constant {
            expected_value: options,
        } => {
            broken.add(at);
            let choices: Vec<String> = match options {
                Value::Array(list) => list
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            vec![not_allowed(at, &path, &here, &choices)]
        }
        K::Type { .. } => {
            broken.add(at);
            if at.is_empty() {
                return vec![Diagnostic::new("settings.not-object", At::value(""))
                    .arg("value", Arg::Input(String::new()))];
            }
            let wanted = schema_type(schema_path);
            vec![Diagnostic::new("settings.wrong-type", At::value(at))
                .arg("value", input())
                .arg("place", place())
                .arg("expected", Arg::Code(wanted))
                .arg("actual", Arg::Code(kind_of(&here).into()))]
        }
        K::Minimum { limit } | K::Maximum { limit } => {
            broken.add(at);
            let code = if matches!(kind, K::Minimum { .. }) {
                "settings.too-small"
            } else {
                "settings.too-large"
            };
            vec![Diagnostic::new(code, At::value(at))
                .arg("value", input())
                .arg("place", place())
                .fix(
                    Fix::review(vec![Edit::Set {
                        at: at.to_string(),
                        value: limit.clone(),
                    }])
                    .arg("limit", Arg::Computed(limit.clone())),
                )]
        }
        K::MinItems { .. } | K::MinProperties { .. } | K::MinLength { .. } => {
            broken.add(at);
            vec![Diagnostic::new("settings.empty", At::value(at)).arg("place", place())]
        }
        K::Pattern { pattern } => {
            broken.add(at);
            let code = match pattern.as_str() {
                SIGNAL => "node.bad-signal",
                ERROR_CODE => "node.bad-error-code",
                _ => "settings.not-decimal",
            };
            let mut d = Diagnostic::new(code, At::value(at)).arg("value", input());
            if code == "settings.not-decimal" {
                d = d.arg("place", place());
                if let Some(n) = here
                    .as_u64()
                    .or_else(|| here.as_str().and_then(|s| s.trim().parse().ok()))
                {
                    d = d.fix(Fix::safe(vec![Edit::Set {
                        at: at.to_string(),
                        value: json!(n.to_string()),
                    }]));
                }
            }
            vec![d]
        }
        _ => {
            broken.add(at);
            vec![Diagnostic::new("spore.unworded-shape", At::value(at)).arg("place", place())]
        }
    }
}

/// The type a schema path's `type` keyword asks for, as words.
fn schema_type(schema_path: &str) -> String {
    let mut node = schema();
    for step in steps(schema_path) {
        node = match node
            .get(&step)
            .or_else(|| step.parse::<usize>().ok().and_then(|i| node.get(i)))
        {
            Some(n) => n,
            None => break,
        };
    }
    match node {
        Value::String(t) => type_name(t).to_string(),
        Value::Array(ts) => ts
            .iter()
            .filter_map(Value::as_str)
            .map(type_name)
            .collect::<Vec<_>>()
            .join(" or "),
        _ => "another type".to_string(),
    }
}

/// A field the schema requires is missing from the object at `at`.
fn missing(value: &Value, at: &str, field: &str) -> Diagnostic {
    let path = steps(at);
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let target = pointer(at, field);
    match (&path[..], field) {
        ([], "format") => {
            Diagnostic::new("settings.no-format", At::value("")).fix(Fix::safe(vec![Edit::Set {
                at: target,
                value: json!(crate::FORMAT),
            }]))
        }
        (["actions", _], "effect") => {
            let suggested = suggested_effect(value, at);
            let mut order = vec![suggested];
            order.extend(EFFECTS.iter().copied().filter(|e| *e != suggested));
            let fixes = order.iter().enumerate().map(|(i, effect)| {
                let fix = Fix::choice(vec![Edit::Set {
                    at: target.clone(),
                    value: json!(effect),
                }]);
                if i == 0 {
                    fix.arg("choices", Arg::computed(EFFECTS.to_vec()))
                } else {
                    fix
                }
            });
            Diagnostic::new("action.no-effect", At::key(at)).fixes(fixes)
        }
        (["actions", _], "kind") => {
            Diagnostic::new("action.no-kind", At::key(at)).fix(Fix::review(vec![Edit::Set {
                at: target,
                value: json!("command"),
            }]))
        }
        (["actions", _], "argv") => Diagnostic::new("command.no-argv", At::key(at)),
        _ => Diagnostic::new("settings.missing-field", At::key(at))
            .arg("field", Arg::computed(field.to_string())),
    }
}

/// The effect most other actions have, else `unsafe`, the one that never runs a step
/// twice.
fn suggested_effect(value: &Value, at: &str) -> &'static str {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for (id, action) in value
        .get("actions")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if pointer("/actions", id) == at {
            continue;
        }
        if let Some(effect) = action.get("effect").and_then(Value::as_str) {
            if let Some(known) = EFFECTS.iter().find(|e| **e == effect) {
                *count.entry(known).or_default() += 1;
            }
        }
    }
    count
        .into_iter()
        .max_by_key(|(e, n)| (*n, std::cmp::Reverse(EFFECTS.iter().position(|x| x == e))))
        .map_or("unsafe", |(e, _)| e)
}

fn not_allowed(at: &str, path: &[&str], here: &Value, choices: &[String]) -> Diagnostic {
    let given = here.as_str().unwrap_or_default();
    let near = closest(given, choices.iter().map(String::as_str));
    let mut order: Vec<&str> = near.into_iter().collect();
    order.extend(
        choices
            .iter()
            .map(String::as_str)
            .filter(|c| Some(*c) != near),
    );
    let fixes = |choices: &[String]| -> Vec<Fix> {
        order
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let applicability = if i == 0 && near.is_some() {
                    Applicability::NeedsReview
                } else {
                    Applicability::NeedsChoice
                };
                let fix = Fix::new(
                    applicability,
                    vec![Edit::Set {
                        at: at.to_string(),
                        value: json!(c),
                    }],
                );
                if i == 0 {
                    fix.arg("choices", Arg::computed(choices.to_vec()))
                } else {
                    fix
                }
            })
            .collect()
    };
    let input = Arg::Input(at.to_string());
    match path {
        ["format"] => Diagnostic::new("settings.wrong-format", At::value(at))
            .arg("format", input)
            .fix(Fix::review(vec![Edit::Set {
                at: at.to_string(),
                value: json!(crate::FORMAT),
            }])),
        ["actions", _, "effect"] if given == "reconcilable" => {
            let effects: Vec<String> = EFFECTS.iter().map(|e| e.to_string()).collect();
            Diagnostic::new("action.unsupported-effect", At::value(at))
                .arg("effect", input)
                .fixes(fixes(&effects).into_iter().map(|mut f| {
                    f.applicability = Applicability::NeedsChoice;
                    f
                }))
        }
        ["actions", _, "effect"] => Diagnostic::new("action.unknown-effect", At::value(at))
            .arg("effect", input)
            .fixes(fixes(choices)),
        ["actions", _, "kind"] => Diagnostic::new("action.unknown-kind", At::value(at))
            .arg("kind", input)
            .fixes(fixes(choices)),
        _ => Diagnostic::new("settings.not-allowed", At::value(at))
            .arg("value", input)
            .arg("place", Arg::Place(at.to_string()))
            .fixes(fixes(choices)),
    }
}

/// A key the schema does not allow at `at`: one that belongs elsewhere names where,
/// a typo names the closest field, anything else is unknown.
fn unexpected_key(value: &Value, at: &str, key: &str, schema_path: &str) -> Diagnostic {
    let child = pointer(at, key);
    let path = steps(at);
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    let def = schema_def(schema_path).unwrap_or_else(|| def_of(at).to_string());
    let allowed = fields(&def);
    let action_fields = fields("Action");
    let step_fields = fields("Step");
    let remove = || Fix::review(vec![Edit::Remove { at: child.clone() }]);
    let k = key.to_string();
    match (&path[..], key) {
        ([], "name" | "start") => Diagnostic::new("settings.from-dot", At::key(&child))
            .arg("key", Arg::Key(child.clone()))
            .fix(remove()),
        (["nodes", _], "kind" | "outcomes") => Diagnostic::new("node.from-dot", At::key(&child))
            .arg("key", Arg::Key(child.clone()))
            .fix(remove()),
        (["actions", action], _) if step_fields.contains(&k) => {
            Diagnostic::new("action.step-field", At::key(&child))
                .arg("key", Arg::Key(child.clone()))
                .fixes(move_to_steps_running(value, action, key, &child))
        }
        (["nodes", step], _) if action_fields.contains(&k) => {
            let target = value
                .pointer(&pointer(&pointer("/nodes", step), "action"))
                .and_then(Value::as_str)
                .map(|a| pointer(&pointer("/actions", a), key));
            let mut d = Diagnostic::new("node.action-field", At::key(&child))
                .arg("key", Arg::Key(child.clone()));
            if let (Some(target), Some(moved)) = (target, value.pointer(&child)) {
                d = d.fix(Fix::review(vec![
                    Edit::Remove { at: child.clone() },
                    Edit::Set {
                        at: target,
                        value: moved.clone(),
                    },
                ]));
            }
            d
        }
        _ => match closest(key, allowed.iter().map(String::as_str)) {
            Some(near) => Diagnostic::new("settings.typo-key", At::key(&child))
                .arg("key", Arg::Key(child.clone()))
                .arg("place", Arg::Place(at.to_string()))
                .fix(
                    Fix::review(vec![Edit::Rename {
                        at: child.clone(),
                        to: near.to_string(),
                    }])
                    .arg("closest", Arg::computed(near.to_string())),
                ),
            None => Diagnostic::new("settings.unknown-key", At::key(&child))
                .arg("key", Arg::Key(child.clone()))
                .arg("place", Arg::Place(at.to_string()))
                .fix(remove().arg("allowed", Arg::computed(allowed.clone()))),
        },
    }
}

/// Moves a step field from an action to each step that runs the action.
fn move_to_steps_running(value: &Value, action: &str, key: &str, child: &str) -> Vec<Fix> {
    let Some(moved) = value.pointer(child) else {
        return Vec::new();
    };
    let steps: Vec<&String> = value
        .get("nodes")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, s)| s.get("action").and_then(Value::as_str) == Some(action))
        .map(|(id, _)| id)
        .collect();
    steps
        .iter()
        .map(|step| {
            let target = pointer(&pointer("/nodes", step), key);
            Fix::choice(vec![
                Edit::Remove {
                    at: child.to_string(),
                },
                Edit::Set {
                    at: target,
                    value: moved.clone(),
                },
            ])
        })
        .collect()
}

/// The `$defs` name a schema path is inside, if any.
fn schema_def(schema_path: &str) -> Option<String> {
    let steps = steps(schema_path);
    let i = steps.iter().position(|s| s == "$defs")?;
    steps.get(i + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_names_its_fields() {
        assert!(fields("").contains(&"$schema".to_string()));
        assert!(fields("Action").contains(&"effect".to_string()));
        assert!(fields("Step").contains(&"loopLimits".to_string()));
        assert_eq!(fields("Limits"), ["maxActivations", "maxVisitsPerNode"]);
    }

    #[test]
    fn every_shape_error_is_a_diagnostic() {
        let value = json!({
            "format": "runspore.workflow/0.1",
            "actions": {"a": {"kind": "command", "argv": ["x"], "efect": "pure"},
                        "b": {"kind": "comand", "argv": [], "effect": "idempotent"}},
            "nodes": {"s": {"action": 5, "argv": ["y"]}}
        });
        let mut found = Found::default();
        let mut broken = Broken::default();
        shape(&value, &mut found, &mut broken);
        let codes: BTreeSet<&str> = found.list.iter().map(|d| d.code).collect();
        let expected: BTreeSet<&str> = [
            "settings.wrong-format",
            "settings.typo-key",
            "action.unknown-kind",
            "settings.empty",
            "settings.wrong-type",
            "node.action-field",
        ]
        .into();
        assert_eq!(codes, expected, "{:#?}", found.list);
        // "efect" is reported as a typo, not also "effect" as missing.
        assert!(broken.touches("/actions/a/efect"));
        assert!(!broken.touches("/actions/a/outcomes"));
    }
}
