//! The build of `spec/dot.md`: a workflow folder (`workflow.dot`, `settings.json` and
//! the files the settings name) to the package the kernel runs.
//!
//! Shared by the CLI and the host. It reads nothing outside the folder and needs no
//! Graphviz installation.

mod dot;
mod json_lines;

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

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
    /// Any other file: a JSON workflow document.
    Document(PathBuf),
}

/// Resolves a workflow argument: a folder, its `workflow.dot`, or a JSON document.
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
    dot::read(&read_file(dot_path)?)
        .map_err(|(code, line, message)| error(code, dot_path, line, message))
}

/// The workflow's name: the graph id of the folder's `workflow.dot`.
pub fn name(folder: &Path) -> Result<String, BuildError> {
    Ok(read_graph(&folder.join(DOT_FILE))?.name)
}

/// Builds the folder into its package, a JSON value ready for the kernel's validation.
pub fn build(folder: &Path) -> Result<Value, BuildError> {
    let dot_path = folder.join(DOT_FILE);
    let settings_path = folder.join(SETTINGS_FILE);
    let graph = read_graph(&dot_path)?;
    let settings_text = read_file(&settings_path)?;
    let settings = Settings {
        path: &settings_path,
        dot_path: &dot_path,
        text: &settings_text,
        folder,
    };
    settings.merge(&graph)
}

struct Settings<'a> {
    path: &'a Path,
    dot_path: &'a Path,
    text: &'a str,
    folder: &'a Path,
}

const TOP_LEVEL: [&str; 4] = ["format", "limits", "actions", "nodes"];

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
                    self.reference(&["actions", id, "assets"], asset)?;
                }
            }
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
                &["action", "input", "instructions", "outputSchemas"],
                Some("action"),
            ),
            "await-signal" => (&["signal", "instructions", "outputSchemas"], Some("signal")),
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
            self.reference(&["nodes", id, "instructions"], &instructions)?;
        }
        if let Some(schemas) = fields.remove("outputSchemas") {
            if !schemas.is_object() {
                return Err(self.invalid(
                    &["nodes", id, "outputSchemas"],
                    "an object keyed by outcome",
                ));
            }
        }
        Ok(fields)
    }

    /// Step 4 of spec/dot.md §5 for one activity node: its action exists, each declared
    /// outcome has an edge, and each edge routes a declared outcome or `failed`.
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
            if e.on != runspore_types::model::OUTCOME_FAILED && !declared.contains(&e.on.as_str()) {
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
    /// once `..` and symbolic links are resolved (spec/dot.md §4).
    fn reference(&self, path: &[&str], value: &Value) -> Result<(), BuildError> {
        let refuse = |why: String| self.at("settings.reference", path, why);
        let Some(reference) = value.as_str().filter(|s| !s.is_empty()) else {
            return Err(self.invalid(path, "a file reference is a non-empty relative path"));
        };
        if Path::new(reference).is_absolute() {
            return Err(refuse(format!(
                "`{reference}` is an absolute path; name a file inside the workflow folder"
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
        Ok(())
    }
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
