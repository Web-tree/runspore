//! What a check returns: a code, where, named arguments and fixes. No text: the words
//! come from the catalogue (`catalogue.rs`), and the report fills them in.

use serde_json::Value;

/// Whether a diagnostic stops the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Problem,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Problem => "problem",
            Severity::Warning => "warning",
        }
    }
}

/// How sure a fix is (rustc's and Ruff's applicability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    /// Cannot change what the workflow means beyond fixing the problem.
    Safe,
    /// Probably right; the author should look.
    NeedsReview,
    /// One of several alternatives; the author picks.
    NeedsChoice,
}

impl Applicability {
    pub fn as_str(self) -> &'static str {
        match self {
            Applicability::Safe => "safe",
            Applicability::NeedsReview => "needs-review",
            Applicability::NeedsChoice => "needs-choice",
        }
    }
}

/// A named value a template quotes. The report reads `Input` and `Key` from the
/// author's own file, so a value they show is the author's by construction.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// The value at a JSON Pointer into `settings.json`, quoted as JSON.
    Input(String),
    /// The key at a JSON Pointer into `settings.json`, quoted.
    Key(String),
    /// A JSON Pointer into `settings.json`, shown as a path: `"actions" → "upload"`.
    Place(String),
    /// A value the check worked out. A string is quoted; a list is shown as its items
    /// joined by ` | `.
    Computed(Value),
    /// Text in the author's own language (a DOT line, a key name), shown as it is.
    Code(String),
}

impl Arg {
    pub fn kind(&self) -> &'static str {
        match self {
            Arg::Input(_) => "input",
            Arg::Key(_) => "key",
            Arg::Place(_) => "place",
            Arg::Computed(_) => "computed",
            Arg::Code(_) => "code",
        }
    }

    pub fn computed(value: impl Into<Value>) -> Self {
        Arg::Computed(value.into())
    }
}

/// Which part of a JSON member a diagnostic marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Key,
    Value,
}

/// Which file of the folder a diagnostic or an edit is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum File {
    Dot,
    Settings,
}

/// Where a diagnostic points.
#[derive(Debug, Clone, PartialEq)]
pub enum At {
    /// A JSON Pointer into `settings.json`.
    Settings { pointer: String, part: Part },
    /// A spot in a file: a line, and the 1-based column and length in characters of
    /// what to mark (length 0: the whole line, unmarked). `step` names the step the
    /// spot belongs to, for the host name `{step}`.
    Text {
        file: File,
        line: usize,
        col: usize,
        len: usize,
        step: Option<String>,
    },
}

impl At {
    /// The key of a member, or an array item's value.
    pub fn key(pointer: impl Into<String>) -> Self {
        let pointer = pointer.into();
        let part = match pointer.rsplit('/').next() {
            Some(last) if !pointer.is_empty() && last.parse::<usize>().is_err() => Part::Key,
            _ => Part::Value,
        };
        At::Settings { pointer, part }
    }

    pub fn value(pointer: impl Into<String>) -> Self {
        At::Settings {
            pointer: pointer.into(),
            part: Part::Value,
        }
    }

    pub fn dot(line: usize, col: usize, len: usize) -> Self {
        At::Text {
            file: File::Dot,
            line,
            col,
            len,
            step: None,
        }
    }

    /// A spot in `settings.json` that no pointer names, such as a repeated key.
    pub fn settings_text(line: usize, col: usize, len: usize) -> Self {
        At::Text {
            file: File::Settings,
            line,
            col,
            len,
            step: None,
        }
    }

    pub fn dot_line(line: usize) -> Self {
        At::dot(line, 1, 0)
    }

    pub fn of_step(mut self, id: &str) -> Self {
        if let At::Text { step, .. } = &mut self {
            *step = Some(id.to_string());
        }
        self
    }
}

/// A replacement of the text between two 1-based (line, column) positions of
/// `workflow.dot`, the end excluded; equal positions insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub text: String,
}

/// One concrete change.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Sets the value at a pointer into `settings.json`, adding the member (and any
    /// missing parent object) when it is absent.
    Set { at: String, value: Value },
    /// Removes the member or item at a pointer into `settings.json`.
    Remove { at: String },
    /// Renames the key at a pointer into `settings.json`, keeping its value.
    Rename { at: String, to: String },
    /// Replaces the member at a pointer into `settings.json`, key and value, with
    /// another member in its place.
    Replace {
        at: String,
        key: String,
        value: Value,
    },
    /// A text edit of `workflow.dot`.
    Dot(TextEdit),
    /// A text edit of `settings.json`, for what no pointer names, such as the first of
    /// two members with one key.
    SettingsText(TextEdit),
}

/// A fix: edits applied together, how sure they are, and arguments the fix template
/// may quote besides the diagnostic's own.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub applicability: Applicability,
    pub edits: Vec<Edit>,
    pub args: Vec<(&'static str, Arg)>,
}

impl Fix {
    pub fn new(applicability: Applicability, edits: Vec<Edit>) -> Self {
        Self {
            applicability,
            edits,
            args: Vec::new(),
        }
    }

    pub fn safe(edits: Vec<Edit>) -> Self {
        Self::new(Applicability::Safe, edits)
    }

    pub fn review(edits: Vec<Edit>) -> Self {
        Self::new(Applicability::NeedsReview, edits)
    }

    pub fn choice(edits: Vec<Edit>) -> Self {
        Self::new(Applicability::NeedsChoice, edits)
    }

    pub fn arg(mut self, name: &'static str, arg: Arg) -> Self {
        self.args.push((name, arg));
        self
    }
}

/// The order in which the build checks, which is the order of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// `workflow.dot` alone.
    Dot,
    /// What joins the two files.
    Join,
    /// `settings.json` alone.
    Settings,
    /// Checks of the whole graph that run when everything before them passed.
    Graph,
}

/// What one check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub at: At,
    pub args: Vec<(&'static str, Arg)>,
    pub fixes: Vec<Fix>,
    pub phase: Phase,
}

impl Diagnostic {
    pub fn new(code: &'static str, at: At) -> Self {
        Self {
            code,
            at,
            args: Vec::new(),
            fixes: Vec::new(),
            phase: Phase::Dot,
        }
    }

    pub fn arg(mut self, name: &'static str, arg: Arg) -> Self {
        self.args.push((name, arg));
        self
    }

    pub fn fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
        self
    }

    pub fn fixes(mut self, fixes: impl IntoIterator<Item = Fix>) -> Self {
        self.fixes.extend(fixes);
        self
    }

    /// The JSON Pointer the diagnostic points at, for a `settings.json` one.
    pub fn pointer(&self) -> Option<&str> {
        match &self.at {
            At::Settings { pointer, .. } => Some(pointer),
            At::Text { .. } => None,
        }
    }
}

/// The diagnostics of one build, each tagged with the phase that found it.
#[derive(Debug, Default)]
pub struct Found {
    pub list: Vec<Diagnostic>,
    phase: Option<Phase>,
}

impl Found {
    pub fn phase(&mut self, phase: Phase) {
        self.phase = Some(phase);
    }

    pub fn push(&mut self, mut diagnostic: Diagnostic) {
        diagnostic.phase = self.phase.unwrap_or(Phase::Dot);
        self.list.push(diagnostic);
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// One JSON Pointer step (RFC 6901).
pub fn pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

/// The unescaped steps of a JSON Pointer.
pub fn steps(pointer: &str) -> Vec<String> {
    if pointer.is_empty() {
        return Vec::new();
    }
    pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}
