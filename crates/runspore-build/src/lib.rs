//! The build of `spec/dot.md`: a workflow folder (`workflow.dot`, `settings.json` and
//! the files the settings name) to the package the kernel runs and the workflow
//! artifact that seals it with those files (`spec/artifact.md`).
//!
//! Shared by the CLI and the host. It reads nothing outside the folder and needs no
//! Graphviz installation.

mod component;
mod dot;
mod json_lines;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use runspore_types::artifact::{self, EntryKind, Manifest, Owner};
use runspore_types::model::{OUTCOME_EXHAUSTED, OUTCOME_FAILED};
use runspore_types::{canonical, digest};
use serde_json::{Map, Value};

pub use component::encode as encode_artifact;
pub use dot::KINDS;

/// The one dialect this build reads.
pub const DIALECT: &str = "runspore/0.2";
/// The package format that agrees with [`DIALECT`].
pub const FORMAT: &str = runspore_types::model::WORKFLOW_FORMAT_0_2;
pub const DOT_FILE: &str = "workflow.dot";
pub const SETTINGS_FILE: &str = "settings.json";

/// A build error (spec/dot.md §6). `Display` gives `<file>:<line>: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub code: &'static str,
    pub file: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

impl std::error::Error for BuildError {}

fn error(code: &'static str, file: &Path, line: usize, message: impl Into<String>) -> BuildError {
    BuildError {
        code,
        file: file.to_path_buf(),
        line: Some(line),
        message: message.into(),
    }
}

/// What a workflow argument names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A workflow folder, given as the folder or its `workflow.dot`.
    Folder(PathBuf),
    /// Any other file: an artifact, or something `spore` does not read. The caller
    /// decides.
    Document(PathBuf),
}

/// Resolves a workflow argument: a folder, its `workflow.dot`, or another file.
/// Any other `.dot` file is refused, since a folder's file names are fixed.
pub fn entry(path: &Path) -> Result<Entry, BuildError> {
    if path.is_dir() {
        return Ok(Entry::Folder(path.to_path_buf()));
    }
    if path.file_name().is_some_and(|n| n == DOT_FILE) {
        let folder = path.parent().unwrap_or(Path::new(""));
        let folder = if folder.as_os_str().is_empty() {
            Path::new(".")
        } else {
            folder
        };
        return Ok(Entry::Folder(folder.to_path_buf()));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dot"))
    {
        return Err(BuildError {
            code: "workflow.folder",
            file: path.to_path_buf(),
            line: None,
            message: format!(
                "a workflow's DOT file is named `{DOT_FILE}` inside its workflow folder"
            ),
        });
    }
    Ok(Entry::Document(path.to_path_buf()))
}

fn read_file(path: &Path) -> Result<String, BuildError> {
    std::fs::read_to_string(path).map_err(|e| BuildError {
        code: "workflow.folder",
        file: path.to_path_buf(),
        line: None,
        message: if e.kind() == std::io::ErrorKind::NotFound {
            "a workflow folder needs this file".to_string()
        } else {
            e.to_string()
        },
    })
}

fn read_graph(dot_path: &Path) -> Result<dot::Graph, BuildError> {
    parse_graph(&read_file(dot_path)?, dot_path)
}

fn parse_graph(text: &str, dot_path: &Path) -> Result<dot::Graph, BuildError> {
    dot::read(text).map_err(|(code, line, message)| error(code, dot_path, line, message))
}

/// The workflow's name: the graph id of the folder's `workflow.dot`.
pub fn name(folder: &Path) -> Result<String, BuildError> {
    Ok(read_graph(&folder.join(DOT_FILE))?.name)
}

/// One file of the folder that goes into the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Folder-relative, with `.` and `..` resolved by name.
    pub path: String,
    pub kind: EntryKind,
    pub owner: Option<Owner>,
    pub executable: bool,
    pub bytes: Vec<u8>,
}

/// What a build produces: the package and the files sealed beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    /// The package, a JSON value ready for the kernel's validation.
    pub package: Value,
    /// `workflow.dot`, `settings.json` and every file the settings name, by path.
    pub files: Vec<File>,
    folder: PathBuf,
}

/// A sealed workflow artifact (spec/artifact.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// The component file.
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    /// Canonical package bytes; their digest is `manifest.main`.
    pub package: Vec<u8>,
}

impl Built {
    /// Seals the package and the files into a workflow artifact. The same folder
    /// always gives the same bytes.
    pub fn seal(&self) -> Result<Sealed, BuildError> {
        let package = canonical::to_vec(&self.package).map_err(|e| BuildError {
            code: e.code(),
            file: self.folder.clone(),
            line: None,
            message: e.to_string(),
        })?;
        let manifest = Manifest {
            format: artifact::FORMAT.to_string(),
            dialect: DIALECT.to_string(),
            name: self.package["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            main: digest::package(&package),
            entries: self
                .files
                .iter()
                .map(|f| artifact::Entry {
                    path: f.path.clone(),
                    digest: digest::blob(&f.bytes),
                    kind: f.kind,
                    owner: f.owner.clone(),
                    executable: f.executable,
                })
                .collect(),
        };
        let mut blobs: Vec<(String, &[u8])> = vec![(manifest.main.clone(), &package)];
        for (entry, file) in manifest.entries.iter().zip(&self.files) {
            if !blobs.iter().any(|(d, _)| *d == entry.digest) {
                blobs.push((entry.digest.clone(), &file.bytes));
            }
        }
        let bytes = encode_artifact(&manifest.encode(), &blobs).map_err(|message| BuildError {
            code: "artifact.encode",
            file: self.folder.clone(),
            line: None,
            message,
        })?;
        Ok(Sealed {
            bytes,
            manifest,
            package,
        })
    }
}

/// Whether the file has any execute bit, as git records it.
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

/// Builds the folder into its package and the files it declares.
pub fn build(folder: &Path) -> Result<Built, BuildError> {
    let dot_path = folder.join(DOT_FILE);
    let settings_path = folder.join(SETTINGS_FILE);
    let dot_text = read_file(&dot_path)?;
    let graph = parse_graph(&dot_text, &dot_path)?;
    let settings_text = read_file(&settings_path)?;
    let settings = Settings {
        path: &settings_path,
        dot_path: &dot_path,
        text: &settings_text,
        folder,
        files: RefCell::new(BTreeMap::new()),
    };
    let package = settings.merge(&graph)?;
    let mut files = settings.files.into_inner();
    for (name, path, text) in [
        (DOT_FILE, &dot_path, dot_text),
        (SETTINGS_FILE, &settings_path, settings_text),
    ] {
        let file = File {
            path: name.to_string(),
            kind: EntryKind::Source,
            owner: None,
            executable: executable(path),
            bytes: text.into_bytes(),
        };
        files.insert((name.to_string(), None), file);
    }
    Ok(Built {
        package,
        files: files.into_values().collect(),
        folder: folder.to_path_buf(),
    })
}

struct Settings<'a> {
    path: &'a Path,
    dot_path: &'a Path,
    text: &'a str,
    folder: &'a Path,
    /// The files the settings name, by manifest path and owner: a file two owners
    /// name is an entry of each.
    files: RefCell<BTreeMap<(String, Option<Owner>), File>>,
}

const TOP_LEVEL: [&str; 4] = ["format", "limits", "actions", "nodes"];
/// The build-only marker a node's input uses for a copy of its schemas.
const MARKER: &str = "$outputSchemas";
/// The level of a node's `input` in the package: the package, `nodes`, the node, `input`.
const INPUT_LEVEL: usize = 4;

impl Settings<'_> {
    /// An error in `settings.json` at the line of `path` (or its nearest parent).
    fn at(&self, code: &'static str, path: &[&str], message: impl Into<String>) -> BuildError {
        error(
            code,
            self.path,
            json_lines::line_near(self.text, path),
            message,
        )
    }

    fn invalid(&self, path: &[&str], message: impl Into<String>) -> BuildError {
        let message = message.into();
        let message = if path.is_empty() {
            message
        } else {
            format!("{}: {message}", path.join("."))
        };
        self.at("settings.invalid", path, message)
    }

    fn dot_line(&self, line: usize) -> String {
        format!("{}:{line}", self.dot_path.display())
    }

    fn merge(&self, graph: &dot::Graph) -> Result<Value, BuildError> {
        let parsed: Value = serde_json::from_str(self.text).map_err(|e| {
            error(
                "settings.invalid",
                self.path,
                e.line().max(1),
                format!("not JSON: {e}"),
            )
        })?;
        runspore_types::canonical::parse(self.text.as_bytes())
            .map_err(|e| self.invalid(&[], e.to_string()))?;
        let Value::Object(top) = parsed else {
            return Err(self.invalid(&[], "settings must be a JSON object"));
        };
        for key in top.keys() {
            if !TOP_LEVEL.contains(&key.as_str()) {
                let why = match key.as_str() {
                    "name" | "start" => format!("comes from {}", self.dot_path.display()),
                    _ => format!("unknown field; allowed: {}", TOP_LEVEL.join(", ")),
                };
                return Err(self.invalid(&[key], why));
            }
        }
        match top.get("format") {
            None => return Err(self.invalid(&[], format!("`format` is required: `{FORMAT}`"))),
            Some(Value::String(f)) if f == FORMAT => {}
            Some(other) => {
                return Err(self.at(
                    "dot.dialect",
                    &["format"],
                    format!(
                        "format {other} does not agree with dialect `{DIALECT}` ({}); use \
                         `{FORMAT}`",
                        self.dot_line(graph.dialect_line)
                    ),
                ))
            }
        }
        let mut actions = self.object(&top, "actions")?;
        let mut nodes = self.object(&top, "nodes")?;

        for (id, action) in &mut actions {
            let Value::Object(action) = action else {
                return Err(self.invalid(&["actions", id], "an action is a JSON object"));
            };
            if let Some(assets) = action.remove("assets") {
                let Value::Array(list) = assets else {
                    return Err(self.invalid(
                        &["actions", id, "assets"],
                        "a list of paths inside the workflow folder",
                    ));
                };
                for asset in &list {
                    self.reference(
                        &["actions", id, "assets"],
                        asset,
                        EntryKind::Asset,
                        Some(Owner::Action(id.clone())),
                    )?;
                }
            }
            self.bundled_programs(id, action)?;
        }

        for id in nodes.keys() {
            if !graph.nodes.contains_key(id) {
                return Err(self.at(
                    "settings.mismatch",
                    &["nodes", id],
                    format!("node `{id}` is not in {}", self.dot_path.display()),
                ));
            }
        }

        let mut package_nodes = Map::new();
        for (id, node) in &graph.nodes {
            let fields = match nodes.remove(id) {
                Some(Value::Object(fields)) => Some(fields),
                Some(_) => return Err(self.invalid(&["nodes", id], "a node is a JSON object")),
                None => None,
            };
            let mut fields = self.node_fields(id, node, fields)?;
            fields.insert("kind".into(), Value::String(node.kind.into()));
            if matches!(node.kind, "activity" | "await-signal") {
                let outcomes: Map<String, Value> = graph
                    .edges
                    .iter()
                    .filter(|e| &e.from == id)
                    .map(|e| (e.on.clone(), Value::String(e.to.clone())))
                    .collect();
                fields.insert("outcomes".into(), Value::Object(outcomes));
            }
            if node.kind == "activity" {
                self.routes(graph, id, node, &fields, &actions)?;
            }
            package_nodes.insert(id.clone(), Value::Object(fields));
        }

        let mut package = Map::new();
        package.insert("format".into(), Value::String(FORMAT.into()));
        package.insert("name".into(), Value::String(graph.name.clone()));
        package.insert("start".into(), Value::String(graph.start.clone()));
        if let Some(limits) = top.get("limits") {
            package.insert("limits".into(), limits.clone());
        }
        package.insert("actions".into(), Value::Object(actions));
        package.insert("nodes".into(), Value::Object(package_nodes));
        Ok(Value::Object(package))
    }

    fn object(
        &self,
        top: &Map<String, Value>,
        key: &str,
    ) -> Result<Map<String, Value>, BuildError> {
        match top.get(key) {
            None => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(self.invalid(&[key], "must be a JSON object keyed by ID")),
        }
    }

    /// Checks a node's settings against its DOT kind (spec/dot.md §4) and strips the
    /// build-only fields.
    fn node_fields(
        &self,
        id: &str,
        node: &dot::Node,
        fields: Option<Map<String, Value>>,
    ) -> Result<Map<String, Value>, BuildError> {
        let (allowed, required): (&[&str], Option<&str>) = match node.kind {
            "activity" => (
                &[
                    "action",
                    "input",
                    "instructions",
                    "outputSchemas",
                    "loopLimits",
                ],
                Some("action"),
            ),
            "await-signal" => (
                &["signal", "instructions", "outputSchemas", "loopLimits"],
                Some("signal"),
            ),
            "complete" => (&["output"], None),
            _ => (&["error"], Some("error")),
        };
        let kind = node.kind;
        let Some(mut fields) = fields else {
            return match required {
                None => Ok(Map::new()),
                Some(field) => Err(error(
                    "settings.mismatch",
                    self.dot_path,
                    node.line,
                    format!(
                        "`{kind}` node `{id}` has no entry in {}; it needs `nodes.{id}.{field}`",
                        self.path.display()
                    ),
                )),
            };
        };
        for key in fields.keys() {
            if !allowed.contains(&key.as_str()) {
                let why = match key.as_str() {
                    "kind" | "outcomes" => format!("comes from {}", self.dot_path.display()),
                    _ => format!(
                        "not allowed on a `{kind}` node ({}); allowed: {}",
                        self.dot_line(node.line),
                        allowed.join(", ")
                    ),
                };
                return Err(self.invalid(&["nodes", id, key], why));
            }
        }
        if let Some(field) = required {
            if !fields.contains_key(field) {
                return Err(
                    self.invalid(&["nodes", id], format!("a `{kind}` node needs `{field}`"))
                );
            }
        }
        if let Some(instructions) = fields.remove("instructions") {
            self.reference(
                &["nodes", id, "instructions"],
                &instructions,
                EntryKind::Instructions,
                Some(Owner::Node(id.to_string())),
            )?;
        }
        let schemas = match fields.remove("outputSchemas") {
            Some(schemas) => Some(self.output_schemas(id, &schemas)?),
            None => None,
        };
        if let Some(input) = fields.get_mut("input") {
            self.markers(id, input, schemas.as_ref(), INPUT_LEVEL)?;
        }
        if let Some(schemas) = schemas {
            fields.insert("outputSchemas".into(), Value::Object(schemas));
        }
        Ok(fields)
    }

    /// A node's `outputSchemas` in package form: files read, `$ref`s inlined, each
    /// schema checked against the subset (spec/schema.md).
    fn output_schemas(&self, id: &str, value: &Value) -> Result<Map<String, Value>, BuildError> {
        let Value::Object(schemas) = value else {
            return Err(self.invalid(
                &["nodes", id, "outputSchemas"],
                "an object keyed by outcome",
            ));
        };
        let mut packaged = Map::new();
        for (outcome, schema) in schemas {
            let path = ["nodes", id, "outputSchemas", outcome];
            let mut place = path.join(".");
            let authored = match schema {
                Value::Object(_) => schema.clone(),
                Value::String(file) => {
                    let target = self.reference(&path, schema, EntryKind::Source, None)?;
                    place = format!("{place} ({file})");
                    let bytes = std::fs::read(&target).map_err(|e| {
                        self.at(
                            runspore_schema::INVALID_SCHEMA,
                            &path,
                            format!("{place}: {e}"),
                        )
                    })?;
                    runspore_types::canonical::parse(&bytes).map_err(|e| {
                        self.at(
                            runspore_schema::INVALID_SCHEMA,
                            &path,
                            format!("{place}: not JSON in the canonical domain: {e}"),
                        )
                    })?
                }
                _ => {
                    return Err(self.invalid(
                        &path,
                        "a schema object, or the path of a JSON file holding one",
                    ))
                }
            };
            let schema = runspore_schema::inline(&authored)
                .and_then(|schema| {
                    runspore_schema::check(&schema, runspore_schema::ROOT_LEVEL).map(|()| schema)
                })
                .map_err(|e| {
                    self.at(
                        runspore_schema::INVALID_SCHEMA,
                        &path,
                        format!("{place}: {e}"),
                    )
                })?;
            packaged.insert(outcome.clone(), schema);
        }
        Ok(packaged)
    }

    /// Replaces each `{"$outputSchemas": {}}` in an evaluated position of a node's
    /// input, at package `level`, with a `$literal` copy of the node's schemas.
    fn markers(
        &self,
        id: &str,
        value: &mut Value,
        schemas: Option<&Map<String, Value>>,
        level: usize,
    ) -> Result<(), BuildError> {
        let path = ["nodes", id, "input"];
        match value {
            Value::Object(map) if map.contains_key(MARKER) => {
                if map.len() != 1 || map[MARKER] != Value::Object(Map::new()) {
                    return Err(self.invalid(
                        &path,
                        format!("the marker is written {{\"{MARKER}\": {{}}}} and stands alone"),
                    ));
                }
                let Some(schemas) = schemas else {
                    return Err(self.invalid(
                        &path,
                        format!("`{MARKER}` copies the node's outputSchemas, and it has none"),
                    ));
                };
                let copy = Value::Object(schemas.clone());
                let depth = level + runspore_schema::nesting(&copy);
                if depth > runspore_types::canonical::MAX_DEPTH {
                    return Err(self.at(
                        runspore_schema::INVALID_SCHEMA,
                        &path,
                        format!(
                            "{}: the copy of outputSchemas that `{MARKER}` makes here reaches \
                             level {depth} of the package, past its limit of 32",
                            path.join(".")
                        ),
                    ));
                }
                *value = serde_json::json!({ "$literal": copy });
            }
            // `$literal`, `$get` and any other operation: their contents are data.
            Value::Object(map) if map.keys().any(|k| k.starts_with('$')) => {}
            Value::Object(map) => {
                for item in map.values_mut() {
                    self.markers(id, item, schemas, level + 1)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.markers(id, item, schemas, level + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// An async probe, watch or stop named `./<path>` is a file of the folder, bundled
    /// as the action's asset (spec/dot.md §4). A bare name is looked up when it runs.
    fn bundled_programs(&self, id: &str, action: &Map<String, Value>) -> Result<(), BuildError> {
        let Some(Value::Object(spec)) = action.get("async") else {
            return Ok(());
        };
        for program in ["probe", "watch", "stop"] {
            let argv0 = spec
                .get(program)
                .and_then(|p| p.get("command"))
                .and_then(|c| c.get("argv"))
                .and_then(|a| a.get(0));
            if let Some(Value::String(argv0)) = argv0 {
                if let Some(path) = argv0.strip_prefix("./") {
                    self.reference(
                        &["actions", id, "async", program, "command", "argv"],
                        &Value::String(path.to_string()),
                        EntryKind::Asset,
                        Some(Owner::Action(id.to_string())),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Step 4 of spec/dot.md §5 for one activity node: its action exists, each declared
    /// outcome has an edge, and each edge routes a declared outcome, `failed`, or
    /// `exhausted` when the node has `loopLimits`.
    fn routes(
        &self,
        graph: &dot::Graph,
        id: &str,
        node: &dot::Node,
        fields: &Map<String, Value>,
        actions: &Map<String, Value>,
    ) -> Result<(), BuildError> {
        let Some(Value::String(action_id)) = fields.get("action") else {
            return Err(self.invalid(&["nodes", id, "action"], "the ID of an action"));
        };
        let Some(action) = actions.get(action_id) else {
            return Err(self.invalid(
                &["nodes", id, "action"],
                format!("names action `{action_id}`, which `actions` does not define"),
            ));
        };
        let declared: Vec<&str> = match action.get("outcomes") {
            None => vec![runspore_types::model::OUTCOME_OK],
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            // A malformed list is the kernel's W07 to report.
            Some(_) => return Ok(()),
        };
        let edges: Vec<&dot::Edge> = graph.edges.iter().filter(|e| e.from == id).collect();
        let limited = fields.contains_key("loopLimits");
        for outcome in &declared {
            if !edges.iter().any(|e| e.on == *outcome) {
                return Err(error(
                    "dot.graph",
                    self.dot_path,
                    node.line,
                    format!(
                        "node `{id}` has no edge for outcome `{outcome}`, which its action \
                         `{action_id}` declares in {}",
                        self.path.display()
                    ),
                ));
            }
        }
        for e in edges {
            if e.on == OUTCOME_EXHAUSTED && !limited {
                return Err(error(
                    "dot.graph",
                    self.dot_path,
                    e.line,
                    format!(
                        "edge `{} -> {}` routes `{OUTCOME_EXHAUSTED}`, which only a node with \
                         `nodes.{id}.loopLimits` in {} routes",
                        e.from,
                        e.to,
                        self.path.display()
                    ),
                ));
            }
            if e.on != OUTCOME_FAILED
                && e.on != OUTCOME_EXHAUSTED
                && !declared.contains(&e.on.as_str())
            {
                return Err(error(
                    "dot.graph",
                    self.dot_path,
                    e.line,
                    format!(
                        "edge `{} -> {}` routes `{}`, which action `{action_id}` does not declare \
                         in {}; declared: {}",
                        e.from,
                        e.to,
                        e.on,
                        self.path.display(),
                        declared.join(", ")
                    ),
                ));
            }
        }
        Ok(())
    }

    /// A file reference: a relative path to a regular file that stays inside the folder
    /// once `..` and symbolic links are resolved (spec/dot.md §4). The file joins the
    /// artifact as an entry of `kind` with `owner`.
    fn reference(
        &self,
        path: &[&str],
        value: &Value,
        kind: EntryKind,
        owner: Option<Owner>,
    ) -> Result<PathBuf, BuildError> {
        let refuse = |why: String| self.at("settings.reference", path, why);
        let Some(reference) = value.as_str().filter(|s| !s.is_empty()) else {
            return Err(self.invalid(path, "a file reference is a non-empty relative path"));
        };
        if Path::new(reference).is_absolute() {
            return Err(refuse(format!(
                "`{reference}` is an absolute path; name a file inside the workflow folder"
            )));
        }
        if reference.contains('\\') {
            return Err(refuse(format!(
                "`{reference}` contains a backslash; separate folders with `/`"
            )));
        }
        let Some(entry_path) = normalize(reference) else {
            return Err(refuse(format!(
                "`{reference}` resolves outside the workflow folder"
            )));
        };
        if entry_path == DOT_FILE || entry_path == SETTINGS_FILE {
            return Err(refuse(format!(
                "`{reference}` names `{entry_path}`, which every build includes as a source"
            )));
        }
        let folder = std::fs::canonicalize(self.folder)
            .map_err(|e| refuse(format!("the workflow folder: {e}")))?;
        let Ok(target) = std::fs::canonicalize(self.folder.join(reference)) else {
            return Err(refuse(format!(
                "`{reference}` names no file in the workflow folder"
            )));
        };
        if !target.starts_with(&folder) {
            return Err(refuse(format!(
                "`{reference}` resolves outside the workflow folder"
            )));
        }
        if !target.is_file() {
            return Err(refuse(format!("`{reference}` is not a regular file")));
        }
        let key = (entry_path, owner);
        let mut files = self.files.borrow_mut();
        if files.contains_key(&key) {
            return Ok(target);
        }
        let bytes = std::fs::read(&target).map_err(|e| refuse(format!("`{reference}`: {e}")))?;
        let file = File {
            path: key.0.clone(),
            kind,
            owner: key.1.clone(),
            executable: executable(&target),
            bytes,
        };
        files.insert(key, file);
        Ok(target)
    }
}

/// A folder-relative reference with empty and `.` segments dropped and `..` resolved
/// by name; `None` when a `..` climbs above the folder.
fn normalize(reference: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in reference.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    let path = segments.join("/");
    artifact::check_path(&path).ok().map(|()| path)
}

/// `name`: W02 of spec/kernel.md.
pub(crate) fn is_name(s: &str) -> bool {
    (1..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Node IDs: W03 of spec/kernel.md.
pub(crate) fn is_node_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Outcomes: `[a-z0-9][a-z0-9-]{0,63}`.
pub(crate) fn is_outcome(s: &str) -> bool {
    let lower = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (1..=64).contains(&s.len())
        && s.bytes().next().is_some_and(lower)
        && s.bytes().all(|b| lower(b) || b == b'-')
}
