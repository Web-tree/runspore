//! The build of `spec/dot.md`: a workflow folder (`workflow.dot`, `settings.json` and
//! the files the settings name) to the package the kernel runs and the workflow
//! artifact that seals it with those files (`spec/artifact.md`).
//!
//! The build checks everything the kernel's own validation checks, and more, and
//! reports every problem at once as [`Diagnostic`]s: values only, worded by the
//! catalogue ([`catalogue`]) and shown by a [`Report`].
//!
//! Shared by the CLI and the host. It reads nothing outside the folder and needs no
//! Graphviz installation.

pub mod catalogue;
mod checks;
mod component;
mod compose;
pub mod diagnostic;
mod dot;
pub mod edit;
mod near;
pub mod report;
pub mod runner;
pub mod settings;
pub mod spans;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use runspore_types::artifact::{self, EntryKind, Manifest, Owner};
use runspore_types::capabilities::HostCapabilities;
use runspore_types::{canonical, digest};
use serde_json::Value;

pub use component::encode as encode_artifact;
pub use compose::{compose, ARTIFACT_INTERFACE, MACHINE_INTERFACE};
pub use diagnostic::{Applicability, Diagnostic, Severity};
pub use dot::KINDS;
pub use report::Report;

/// The one dialect this build reads.
pub const DIALECT: &str = "runspore/0.2";
/// The package format that agrees with [`DIALECT`].
pub const FORMAT: &str = runspore_types::model::WORKFLOW_FORMAT_0_2;
pub const DOT_FILE: &str = "workflow.dot";
pub const SETTINGS_FILE: &str = "settings.json";
/// The settings key naming the action spore runs when a run stops for a person.
pub const ON_INTERVENTION: &str = "onIntervention";

/// A folder the build cannot read at all, or an artifact it cannot seal.
/// `Display` gives `<file>: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub code: &'static str,
    pub file: PathBuf,
    pub message: String,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.message)
    }
}

impl std::error::Error for BuildError {}

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
        message: if e.kind() == std::io::ErrorKind::NotFound {
            "a workflow folder needs this file".to_string()
        } else {
            e.to_string()
        },
    })
}

/// The folder's own name, for the workflow name a report uses when the graph has none.
fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| {
            std::fs::canonicalize(folder)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "workflow".to_string())
}

/// The workflow's name: the graph id of the folder's `workflow.dot`.
pub fn name(folder: &Path) -> Result<String, BuildError> {
    let dot_path = folder.join(DOT_FILE);
    let text = read_file(&dot_path)?;
    let mut found = diagnostic::Found::default();
    match dot::read(&text, &folder_name(folder), &mut found) {
        dot::Read::Graph(graph) if is_name(&graph.name) => Ok(graph.name.clone()),
        _ => {
            let report = Report::new(
                folder.display().to_string(),
                folder_name(folder),
                text,
                String::new(),
                None,
                None,
                found.list,
            );
            let first = report.resolved().into_iter().next();
            Err(BuildError {
                code: "workflow.invalid",
                file: dot_path,
                message: first.map_or_else(String::new, |r| r.message),
            })
        }
    }
}

/// Where programs are looked up, which native functions exist, and what the host
/// offers, when the build checks what a run will need.
#[derive(Debug, Clone)]
pub struct Machine {
    pub programs: Programs,
    /// The native functions the host registered; `None` does not check them.
    pub functions: Option<Vec<String>>,
    /// The host's capability set (spec/host.md §2, The handshake); `None` does not
    /// check actions against it.
    pub host: Option<HostCapabilities>,
}

#[derive(Debug, Clone)]
pub enum Programs {
    /// Look programs up on this `PATH`.
    Path(OsString),
    /// These names exist; for tests and examples.
    Known(Vec<String>),
    /// Do not look.
    Unchecked,
}

impl Machine {
    /// This machine: its `PATH`, and no native functions (as the `spore` binary).
    pub fn here() -> Self {
        Self {
            programs: Programs::Path(std::env::var_os("PATH").unwrap_or_default()),
            functions: Some(Vec::new()),
            host: None,
        }
    }

    /// This machine, checking actions against what `host` offers.
    pub fn offering(self, host: HostCapabilities) -> Self {
        Self {
            host: Some(host),
            ..self
        }
    }

    /// Checks nothing about the machine.
    pub fn unchecked() -> Self {
        Self {
            programs: Programs::Unchecked,
            functions: None,
            host: None,
        }
    }

    /// The machine the catalogue's examples assume: the programs they name exist, and
    /// the host offers what `spore` does.
    pub fn examples() -> Self {
        Self {
            programs: Programs::Known(EXAMPLE_PROGRAMS.iter().map(|p| p.to_string()).collect()),
            functions: Some(Vec::new()),
            host: Some(HostCapabilities::of(["command", "native"])),
        }
    }
}

/// The programs the catalogue's examples may run without a warning.
pub const EXAMPLE_PROGRAMS: [&str; 6] = ["aws", "pg_dump", "make", "true", "echo", "sh"];

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

/// What checking a folder gives: every diagnostic, and the build when no problem
/// stopped it.
#[derive(Debug)]
pub struct Checked {
    pub report: Report,
    pub built: Option<Built>,
    /// The package the folder would give, problems and all, with `format` as the
    /// settings give it; `None` when either file cannot be read as a whole. For tests
    /// that show the kernel refuses what the build reports.
    pub draft: Option<Value>,
}

/// Checks the folder and builds it when nothing is wrong. Only a folder that cannot be
/// read at all (a missing `workflow.dot` or `settings.json`) is an error; everything
/// else is in the report.
pub fn check(folder: &Path, machine: &Machine) -> Result<Checked, BuildError> {
    let dot_path = folder.join(DOT_FILE);
    let settings_path = folder.join(SETTINGS_FILE);
    let dot_text = read_file(&dot_path)?;
    let settings_text = read_file(&settings_path)?;
    let own_name = folder_name(folder);
    let mut found = diagnostic::Found::default();
    found.phase(diagnostic::Phase::Dot);
    let read = dot::read(&dot_text, &own_name, &mut found);
    let parsed: Result<Value, serde_json::Error> = serde_json::from_str(&settings_text);
    let spans = parsed
        .as_ref()
        .ok()
        .map(|_| spans::Spans::scan(&settings_text, canonical::MAX_DEPTH));
    let graph = match &read {
        dot::Read::Graph(graph) => Some(&**graph),
        _ => None,
    };
    let workflow = graph.map_or(own_name, |g| g.name.clone());
    let mut built = None;
    let mut draft = None;
    let mut files = BTreeMap::new();
    if !matches!(read, dot::Read::Stop) {
        match &parsed {
            Err(e) => {
                found.phase(diagnostic::Phase::Settings);
                let (line, col) = json_error_spot(&settings_text, e);
                found.push(diagnostic::Diagnostic::new(
                    "settings.not-json",
                    diagnostic::At::settings_text(line, col, 1),
                ));
            }
            Ok(value) => {
                let spans = spans.as_ref().expect("scanned with the parse");
                let mut checks = checks::Checks::new(
                    folder,
                    &dot_text,
                    graph,
                    value,
                    spans,
                    &settings_text,
                    &mut found,
                    machine,
                );
                checks.run();
                let package = graph.map(|g| checks.package(g));
                draft = package.clone().map(|mut p| {
                    if let Some(format) = value.get("format").filter(|f| f.is_string()) {
                        p["format"] = format.clone();
                    }
                    p
                });
                files = checks.files.into_inner();
                let problems = found.list.iter().any(|d| {
                    catalogue::catalogue()
                        .get(d.code)
                        .is_none_or(|e| e.severity == Severity::Problem)
                });
                if let (false, Some(package)) = (problems, package) {
                    built = Some(package);
                }
            }
        }
    }
    let built = built.map(|package| {
        for (name, path, text) in [
            (DOT_FILE, &dot_path, &dot_text),
            (SETTINGS_FILE, &settings_path, &settings_text),
        ] {
            let file = File {
                path: name.to_string(),
                kind: EntryKind::Source,
                owner: None,
                executable: executable(path),
                bytes: text.clone().into_bytes(),
            };
            files.insert((name.to_string(), None), file);
        }
        Built {
            package,
            files: std::mem::take(&mut files).into_values().collect(),
            folder: folder.to_path_buf(),
        }
    });
    let report = Report::new(
        folder.display().to_string(),
        workflow,
        dot_text,
        settings_text,
        parsed.ok(),
        spans,
        found.list,
    );
    Ok(Checked {
        report,
        built,
        draft,
    })
}

/// Where a JSON parse error is, as a line of the text and a column in characters: an
/// error at the end of the file is placed at the end of its last line.
fn json_error_spot(text: &str, e: &serde_json::Error) -> (usize, usize) {
    let lines = spans::Lines::new(text);
    let count = lines.count().max(1);
    if e.line() > count || e.line() == 0 {
        return (count, lines.line(count).chars().count() + 1);
    }
    let line = lines.line(e.line());
    let byte = e.column().saturating_sub(1).min(line.len());
    let byte = (0..=byte)
        .rev()
        .find(|b| line.is_char_boundary(*b))
        .unwrap_or(0);
    (e.line(), line[..byte].chars().count() + 1)
}

/// Builds the folder into its package and the files it declares, without looking at
/// the machine. A folder with problems is `workflow.invalid`, with one line per
/// problem.
pub fn build(folder: &Path) -> Result<Built, BuildError> {
    let checked = check(folder, &Machine::unchecked())?;
    checked.built.ok_or_else(|| BuildError {
        code: "workflow.invalid",
        file: folder.to_path_buf(),
        message: checked.report.short().trim_end().to_string(),
    })
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

// ---------------------------------------------------------------------------
// The catalogue's examples, and the help page of each code
// ---------------------------------------------------------------------------

/// Every file under `catalogue/examples/` and `catalogue/fixed/`, by path.
pub mod examples {
    include!(concat!(env!("OUT_DIR"), "/catalogue_files.rs"));

    /// The files of one example or fixed folder (`examples/<name>` or
    /// `fixed/<code>`), by their path inside it.
    pub fn folder(prefix: &str) -> Vec<(&'static str, &'static [u8])> {
        let prefix = format!("{prefix}/");
        FILES
            .iter()
            .filter_map(|(path, bytes)| path.strip_prefix(prefix.as_str()).map(|p| (p, *bytes)))
            .collect()
    }
}

/// A folder written to a temporary directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, files: &[(&str, &[u8])]) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("spore-explain-{}-{n}", std::process::id()));
        let dir = root.join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir)?;
        for (path, bytes) in files {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, bytes)?;
        }
        Ok(Self(root))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A code's example, checked: the example's name, its report, the block of the code
/// itself, and the fix as a diff between the example and its fixed folder.
pub struct Example {
    pub name: String,
    pub report: Report,
    pub block: String,
    pub diff: String,
}

/// Checks the example of `code` as `spore validate <example>` would show it; `None`
/// for a code with no example.
pub fn example(code: &str) -> Option<Example> {
    let entry = catalogue::catalogue().get(code)?;
    let name = entry.example.clone()?;
    let wrong = examples::folder(&format!("examples/{name}"));
    let scratch = Scratch::new(&name, &wrong).ok()?;
    let mut checked = check(&scratch.0.join(&name), &Machine::examples()).ok()?;
    checked.report.folder = name.clone();
    let resolved = checked.report.resolved();
    let block = resolved
        .iter()
        .find(|r| r.code == code)
        .map(|r| checked.report.block(r))
        .unwrap_or_default();
    let fixed = examples::folder(&format!("fixed/{code}"));
    let mut diff = String::new();
    for (path, after) in &fixed {
        let before = wrong
            .iter()
            .find(|(p, _)| p == path)
            .map_or(&b""[..], |(_, b)| *b);
        if before == *after {
            continue;
        }
        let (before, after) = (
            String::from_utf8_lossy(before),
            String::from_utf8_lossy(after),
        );
        diff.push_str(&format!("{name}/{path}:\n"));
        diff.push_str(&edit::line_diff(&before, &after));
    }
    Some(Example {
        name,
        report: checked.report,
        block,
        diff,
    })
}

/// What a severity means, for a help page.
fn severity(severity: Severity) -> &'static str {
    match severity {
        Severity::Problem => "A problem: the workflow does not build, and nothing runs.",
        Severity::Warning => {
            "A warning: the workflow still builds and runs; `spore validate --strict` fails on it."
        }
    }
}

/// The help page of a code, as `spore explain <code>` prints it.
pub fn explain(code: &str) -> Option<String> {
    let entry = catalogue::catalogue().get(code)?;
    let mut out = format!(
        "{}: {} ({})\n\n{}\n",
        entry.code,
        entry.title,
        entry.severity.as_str(),
        severity(entry.severity)
    );
    if let Some(explanation) = &entry.explanation {
        out.push_str(&format!("\n{}\n", explanation.trim()));
    }
    match example(code) {
        Some(ex) => {
            out.push_str(&format!(
                "\nFor example, `spore validate {}` reports:\n\n{}",
                ex.name, ex.block
            ));
            if !ex.diff.is_empty() {
                out.push_str(&format!("\nThe fix changes:\n\n{}", ex.diff));
            }
        }
        None => out.push_str(&format!("\n{}\nTo fix: {}\n", entry.message, entry.fix)),
    }
    Some(out)
}

/// The help page of a code as Markdown, for `docs/codes/<code>.md`.
pub fn page(code: &str) -> Option<String> {
    let entry = catalogue::catalogue().get(code)?;
    let mut out = format!(
        "# `{}`: {}\n\n<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->\n\n{}\n",
        entry.code,
        entry.title,
        severity(entry.severity)
    );
    if let Some(explanation) = &entry.explanation {
        out.push_str(&format!("\n{}\n", explanation.trim()));
    }
    match example(code) {
        Some(ex) => {
            out.push_str(&format!(
                "\n## Example\n\n`spore validate {}` reports:\n\n```text\n{}```\n",
                ex.name, ex.block
            ));
            if !ex.diff.is_empty() {
                out.push_str(&format!("\nThe fix changes:\n\n```text\n{}```\n", ex.diff));
            }
            out.push_str(&format!(
                "\nThe example folder is [`catalogue/examples/{0}`](../../crates/runspore-build/catalogue/examples/{0}).\n",
                ex.name
            ));
        }
        None => out.push_str(&format!("\n{}\n\nTo fix: {}\n", entry.message, entry.fix)),
    }
    Some(out)
}

/// The index of every code's page, for `docs/codes/README.md`.
pub fn index() -> String {
    let mut out = String::from(
        "# Validation codes\n\n<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->\n\nEvery problem and warning `spore validate` can report, with its help page. `spore explain <code>` prints the same page. The [validation guide](../validation.md) walks through one workflow, broken and fixed.\n\n| Code | Severity | Title |\n| --- | --- | --- |\n",
    );
    let mut entries: Vec<&catalogue::Entry> = catalogue::catalogue().entries.iter().collect();
    entries.sort_by(|a, b| a.code.cmp(&b.code));
    for e in entries {
        out.push_str(&format!(
            "| [`{0}`]({0}.md) | {1} | {2} |\n",
            e.code,
            e.severity.as_str(),
            e.title.replace('|', "\\|")
        ));
    }
    out
}
