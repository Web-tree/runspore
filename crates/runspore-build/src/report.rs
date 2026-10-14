//! A build's diagnostics with the files they point into, and the three ways spore
//! shows them: blocks of text for people, one line each (`--short`), and JSON for
//! scripts and editors.

use serde_json::{json, Map, Value};

use crate::catalogue::{self, catalogue, ArgKind, Entry, HOST_NAMES};
use crate::diagnostic::{steps, Applicability, Arg, At, Diagnostic, Edit, Part, Severity};
use crate::edit::{self, File, Region, Replacement, Texts};
use crate::spans::{Lines, Spans};
use crate::{DOT_FILE, SETTINGS_FILE};

/// How many blocks the text output shows before it stops.
pub const MAX_BLOCKS: usize = 20;
/// A heading of up to this many characters is followed by this many dashes.
const RULE: usize = 18;

/// What a build found, with what it needs to show it.
#[derive(Debug, Clone)]
pub struct Report {
    /// The folder as it was named, for the paths the report prints.
    pub folder: String,
    /// The workflow's name: the graph id, else the folder's name.
    pub workflow: String,
    pub dot: String,
    pub settings: String,
    settings_value: Option<Value>,
    spans: Option<Spans>,
    /// Sorted: problems before warnings, then in the order the build checks.
    pub diagnostics: Vec<Diagnostic>,
}

/// A diagnostic with its words filled in and its place found.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub severity: Severity,
    pub code: String,
    pub title: String,
    pub heading: String,
    pub message: String,
    pub fix: String,
    pub why: Option<String>,
    pub more: bool,
    pub show_fix: bool,
    pub file: File,
    /// 1-based (line, column) of the start and the end, the end excluded.
    pub start: (usize, usize),
    pub end: (usize, usize),
    /// The column and length to mark on the first line, if any.
    pub mark: Option<(usize, usize)>,
    /// The lines to show, in order.
    pub lines: Vec<usize>,
    pub path: Option<String>,
    pub args: Vec<ResolvedArg>,
    pub fixes: Vec<ResolvedFix>,
}

#[derive(Debug, Clone)]
pub struct ResolvedArg {
    pub name: String,
    pub kind: &'static str,
    pub value: Value,
    pub pointer: Option<String>,
    pub shown: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedFix {
    pub applicability: Applicability,
    pub edits: Vec<(Edit, Replacement)>,
}

impl Report {
    pub fn new(
        folder: String,
        workflow: String,
        dot: String,
        settings: String,
        settings_value: Option<Value>,
        spans: Option<Spans>,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        let mut report = Self {
            folder,
            workflow,
            dot,
            settings,
            settings_value,
            spans,
            diagnostics,
        };
        report.sort();
        report
    }

    /// Adds a diagnostic found after the build, such as one the kernel's own checks
    /// gave rise to.
    pub fn add(&mut self, d: Diagnostic) {
        self.diagnostics.push(d);
        self.sort();
    }

    fn sort(&mut self) {
        let diagnostics = std::mem::take(&mut self.diagnostics);
        let mut keyed: Vec<_> = diagnostics
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                let r = self.resolve(&d);
                let severity = catalogue()
                    .get(d.code)
                    .map_or(Severity::Problem, |e| e.severity);
                ((severity, d.phase, r.file, r.start, i), d)
            })
            .collect();
        keyed.sort_by_key(|k| k.0);
        self.diagnostics = keyed.into_iter().map(|(_, d)| d).collect();
    }

    pub fn problems(&self) -> usize {
        self.resolved()
            .iter()
            .filter(|r| r.severity == Severity::Problem)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.resolved()
            .iter()
            .filter(|r| r.severity == Severity::Warning)
            .count()
    }

    pub fn codes(&self) -> Vec<String> {
        self.resolved().into_iter().map(|r| r.code).collect()
    }

    pub fn resolved(&self) -> Vec<Resolved> {
        self.diagnostics.iter().map(|d| self.resolve(d)).collect()
    }

    /// The diagnostic in words, or spore's own message that it made a broken one.
    pub fn resolve(&self, d: &Diagnostic) -> Resolved {
        self.try_resolve(d).unwrap_or_else(|_| {
            let broken = Diagnostic::new("spore.broken-message", self.fallback_at(&d.at))
                .arg("code", Arg::Code(d.code.to_string()));
            self.try_resolve(&broken)
                .expect("the broken-message entry always resolves")
        })
    }

    /// The diagnostic's own place if spore can show it, else the first line of its file.
    fn fallback_at(&self, at: &At) -> At {
        match at {
            At::Settings { pointer, .. } if self.span(pointer).is_some() => at.clone(),
            At::Settings { .. } if self.spans.is_some() => At::value(""),
            At::Text {
                file, line, step, ..
            } if *line >= 1 && *line <= Lines::new(self.source(*file)).count() => At::Text {
                file: *file,
                line: *line,
                col: 1,
                len: 0,
                step: step.clone(),
            },
            _ => At::dot_line(1),
        }
    }

    fn span(&self, pointer: &str) -> Option<&crate::spans::Span> {
        self.spans.as_ref()?.get(pointer)
    }

    fn try_resolve(&self, d: &Diagnostic) -> Result<Resolved, String> {
        let entry: &Entry = catalogue()
            .get(d.code)
            .ok_or_else(|| format!("no catalogue entry for {}", d.code))?;
        let place = self.place(&d.at)?;
        let mut args = Vec::new();
        for (name, arg) in &d.args {
            args.push(self.arg(entry, name, arg)?);
        }
        let mut fixes = Vec::new();
        let texts = Texts {
            dot: &self.dot,
            settings: &self.settings,
            spans: self.spans.as_ref(),
        };
        for fix in &d.fixes {
            let mut edits = Vec::new();
            for e in &fix.edits {
                let r = edit::replacement(e, &texts).ok_or("an edit does not apply")?;
                edits.push((e.clone(), r));
            }
            for file in [File::Dot, File::Settings] {
                let rs: Vec<&Replacement> = edits
                    .iter()
                    .map(|(_, r)| r)
                    .filter(|r| r.file == file)
                    .collect();
                edit::apply(self.source(file), &rs).ok_or("the edits of a fix overlap")?;
            }
            fixes.push(ResolvedFix {
                applicability: fix.applicability,
                edits,
            });
        }
        let mut fix_args = args.clone();
        if let Some(first) = d.fixes.first() {
            for (name, arg) in &first.args {
                let resolved = self.arg(entry, name, arg)?;
                fix_args.retain(|a| a.name != resolved.name);
                fix_args.push(resolved);
            }
        }
        let host = |name: &str| self.host(name, d, place.file);
        let lookup = |args: &[ResolvedArg], name: &str| {
            args.iter()
                .find(|a| a.name == name)
                .map(|a| a.shown.clone())
                .or_else(|| host(name))
        };
        let fill = |template: &str, args: &[ResolvedArg]| {
            catalogue::fill(template, |name| lookup(args, name))
                .ok_or_else(|| format!("a placeholder of {template:?} has no value"))
        };
        let heading = fill(entry.heading.as_ref().unwrap_or(&entry.title), &args)?;
        let message = fill(&entry.message, &args)?;
        let fix = fill(&entry.fix, &fix_args)?;
        let why = entry.why.as_ref().map(|w| fill(w, &args)).transpose()?;
        Ok(Resolved {
            severity: entry.severity,
            code: entry.code.clone(),
            title: entry.title.clone(),
            heading,
            message,
            fix,
            why,
            more: entry.explanation.is_some(),
            show_fix: entry.show_fix,
            file: place.file,
            start: place.start,
            end: place.end,
            mark: place.mark,
            lines: place.lines,
            path: d.pointer().map(str::to_string),
            args: fix_args,
            fixes,
        })
    }

    fn source(&self, file: File) -> &str {
        match file {
            File::Dot => &self.dot,
            File::Settings => &self.settings,
        }
    }

    fn place(&self, at: &At) -> Result<Place, String> {
        match at {
            At::Settings { pointer, part } => {
                let span = self
                    .span(pointer)
                    .ok_or_else(|| format!("pointer {pointer:?} does not resolve"))?;
                let range = match part {
                    Part::Key => span.key.clone().unwrap_or(span.value.clone()),
                    Part::Value => span.value.clone(),
                };
                let lines = Lines::new(&self.settings);
                let (start, end) = (lines.position(range.start), lines.position(range.end));
                let first = lines.line(start.0);
                let on_line = if end.0 == start.0 {
                    end.1 - start.1
                } else {
                    first.chars().count() + 1 - start.1
                };
                let mut shown = vec![start.0];
                // A key's previous sibling on the line above shows which object holds it.
                if *part == Part::Key && span.key.is_some() {
                    if let Some(previous) = self.previous_sibling(pointer) {
                        if lines.position(previous).0 + 1 == start.0 {
                            shown.insert(0, start.0 - 1);
                        }
                    }
                }
                Ok(Place {
                    file: File::Settings,
                    start,
                    end,
                    mark: Some((start.1, on_line.max(1))),
                    lines: shown,
                })
            }
            At::Text {
                file,
                line,
                col,
                len,
                ..
            } => {
                let lines = Lines::new(self.source(*file));
                if *line == 0 || *line > lines.count().max(1) {
                    return Err(format!("line {line} of {file:?} does not exist"));
                }
                let text = lines.line(*line);
                let (start, end, mark) = if *len == 0 {
                    ((*line, 1), (*line, text.chars().count() + 1), None)
                } else {
                    ((*line, *col), (*line, col + len), Some((*col, *len)))
                };
                Ok(Place {
                    file: *file,
                    start,
                    end,
                    mark,
                    lines: vec![*line],
                })
            }
        }
    }

    /// Where the member before `pointer` starts, if it has one.
    fn previous_sibling(&self, pointer: &str) -> Option<usize> {
        let spans = self.spans.as_ref()?;
        let parent = &pointer[..pointer.rfind('/')?];
        let siblings = spans.children(parent);
        let i = siblings.iter().position(|s| s == pointer)?;
        let previous = spans.get(siblings.get(i.checked_sub(1)?)?)?;
        Some(
            previous
                .key
                .as_ref()
                .map_or(previous.value.start, |k| k.start),
        )
    }

    fn host(&self, name: &str, d: &Diagnostic, file: File) -> Option<String> {
        let under = |root: &str| {
            let steps = steps(d.pointer()?);
            (steps.first().map(String::as_str) == Some(root))
                .then(|| steps.get(1).cloned())
                .flatten()
        };
        match name {
            "action" => under("actions").map(|a| quote(&a)),
            "step" => match &d.at {
                At::Text { step, .. } => step.as_ref().map(|s| quote(s)),
                At::Settings { .. } => under("nodes").map(|s| quote(&s)),
            },
            "owner" => match &d.at {
                At::Text { step, .. } => step.as_ref().map(|s| format!("step {}", quote(s))),
                At::Settings { .. } => under("nodes")
                    .map(|s| format!("step {}", quote(&s)))
                    .or_else(|| under("actions").map(|a| format!("action {}", quote(&a)))),
            },
            "file" => Some(file_name(file).to_string()),
            "workflow" => Some(quote(&self.workflow)),
            _ => None,
        }
    }

    fn arg(&self, entry: &Entry, name: &str, arg: &Arg) -> Result<ResolvedArg, String> {
        let declared = entry.args.get(name).copied();
        let kind = arg.kind();
        if !HOST_NAMES.contains(&name) && declared.map(ArgKind::as_str) != Some(kind) {
            return Err(format!("{{{name}}} is not declared as {kind}"));
        }
        let (value, pointer, shown) = match arg {
            Arg::Input(p) => {
                let value = self
                    .settings_value
                    .as_ref()
                    .and_then(|v| v.pointer(p))
                    .ok_or_else(|| format!("pointer {p:?} does not resolve"))?;
                (value.clone(), Some(p.clone()), edit::inline(value))
            }
            Arg::Key(p) => {
                self.span(p)
                    .and_then(|s| s.key.as_ref())
                    .ok_or_else(|| format!("pointer {p:?} names no key"))?;
                let key = steps(p).pop().unwrap_or_default();
                (Value::String(key.clone()), Some(p.clone()), quote(&key))
            }
            Arg::Place(p) => {
                let shown = path(p);
                (Value::String(p.clone()), Some(p.clone()), shown)
            }
            Arg::Computed(v) => (v.clone(), None, computed(v)),
            Arg::Code(text) => (Value::String(text.clone()), None, text.clone()),
        };
        Ok(ResolvedArg {
            name: name.to_string(),
            kind,
            value,
            pointer,
            shown,
        })
    }

    /// The path of a file of the folder as this report prints it.
    pub fn path_of(&self, file: File) -> String {
        let folder = self.folder.trim_end_matches('/');
        if folder.is_empty() {
            file_name(file).to_string()
        } else {
            format!("{folder}/{}", file_name(file))
        }
    }

    // ------------------------------------------------------------------
    // Text
    // ------------------------------------------------------------------

    /// One block per diagnostic, at most [`MAX_BLOCKS`], then the count. `command`
    /// is how the caller lists every one (`spore validate --short <folder>`).
    pub fn text(&self, command: &str) -> String {
        let resolved = self.resolved();
        let mut blocks: Vec<String> = resolved
            .iter()
            .take(MAX_BLOCKS)
            .map(|r| self.block(r))
            .collect();
        if resolved.len() > MAX_BLOCKS {
            blocks.push(format!(
                "… and {} more. {command} lists them all.\n",
                resolved.len() - MAX_BLOCKS
            ));
        }
        blocks.push(format!("{}\n", self.count()));
        blocks.join("\n")
    }

    /// The last line: how many problems and warnings, and that nothing ran.
    pub fn count(&self) -> String {
        let (p, w) = (self.problems(), self.warnings());
        let problems = plural(p, "problem");
        let warnings = plural(w, "warning");
        let workflow = &self.workflow;
        match (p, w) {
            (0, 0) => format!("No problems in {workflow}."),
            (_, 0) => format!("{problems} in {workflow}. Nothing was run."),
            (0, _) => format!("{warnings} in {workflow}."),
            _ => format!("{problems} and {warnings} in {workflow}. Nothing was run."),
        }
    }

    /// The block of one diagnostic.
    pub fn block(&self, r: &Resolved) -> String {
        let heading = match r.severity {
            Severity::Problem => r.heading.clone(),
            Severity::Warning => format!("{} (warning)", r.heading),
        };
        let dashes = if heading.chars().count() <= RULE {
            RULE
        } else {
            2
        };
        let mut out = format!(
            "── {heading} {} {}:{}  {}\n{}\n\n",
            "─".repeat(dashes),
            self.path_of(r.file),
            r.start.0,
            r.code,
            r.message
        );
        let lines = Lines::new(self.source(r.file));
        let width = gutter(r.lines.iter().copied());
        for &n in &r.lines {
            out.push_str(&row(width, n, ' ', lines.line(n)));
        }
        if let Some((col, len)) = r.mark {
            let line = lines.line(r.start.0);
            let pad: String = line
                .chars()
                .take(col - 1)
                .map(|c| if c == '\t' { '\t' } else { ' ' })
                .collect();
            out.push_str(&format!("{:>width$} │ {pad}{}\n", "", "^".repeat(len)));
        }
        out.push_str(&format!("\nTo fix: {}\n", r.fix));
        let shown = if r.show_fix { self.fix_lines(r) } else { None };
        let more = r.why.is_some() || r.more;
        if let Some(rows) = shown {
            out.push('\n');
            out.push_str(&rows);
            if more {
                out.push('\n');
            }
        }
        if let Some(why) = &r.why {
            out.push_str(&format!("Why: {why}\n"));
        }
        if r.more {
            out.push_str(&format!("More: spore explain {}\n", r.code));
        }
        out
    }

    /// The first fix applied to the author's lines.
    fn fix_lines(&self, r: &Resolved) -> Option<String> {
        let fix = r.fixes.first()?;
        let mut out = String::new();
        for file in [File::Dot, File::Settings] {
            let rs: Vec<&Replacement> = fix
                .edits
                .iter()
                .map(|(_, r)| r)
                .filter(|r| r.file == file)
                .collect();
            if rs.is_empty() {
                continue;
            }
            for region in edit::changed(self.source(file), &rs)? {
                let lines = region.result();
                let width = gutter(lines.iter().map(|l| l.0));
                for (n, mark, text) in lines {
                    out.push_str(&row(width, n, mark, text));
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// One line per diagnostic: `file:line:col: code: message`.
    pub fn short(&self) -> String {
        self.resolved()
            .iter()
            .map(|r| {
                format!(
                    "{}:{}:{}: {}: {}\n",
                    self.path_of(r.file),
                    r.start.0,
                    r.start.1,
                    r.code,
                    r.message
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // JSON
    // ------------------------------------------------------------------

    /// `{valid, problems, warnings, diagnostics}`; each diagnostic maps onto an LSP
    /// `Diagnostic` and its fixes onto `CodeAction`s.
    pub fn json(&self) -> Value {
        let resolved = self.resolved();
        let diagnostics: Vec<Value> = resolved.iter().map(|r| self.json_one(r)).collect();
        let problems = self.problems();
        json!({
            "valid": problems == 0,
            "problems": problems.to_string(),
            "warnings": self.warnings().to_string(),
            "diagnostics": diagnostics,
        })
    }

    fn json_one(&self, r: &Resolved) -> Value {
        let args: Vec<Value> = r
            .args
            .iter()
            .map(
                |a| json!({"name": a.name, "kind": a.kind, "value": a.value, "pointer": a.pointer}),
            )
            .collect();
        let fixes: Vec<Value> = r
            .fixes
            .iter()
            .map(|f| {
                let edits: Vec<Value> = f
                    .edits
                    .iter()
                    .map(|(e, rep)| self.json_edit(e, rep))
                    .collect();
                json!({"applicability": f.applicability.as_str(), "edits": edits})
            })
            .collect();
        json!({
            "severity": r.severity.as_str(),
            "code": r.code,
            "title": r.title,
            "message": r.message,
            "fix": r.fix,
            "why": r.why,
            "file": self.path_of(r.file),
            "range": range(r.start, r.end),
            "path": r.path,
            "args": args,
            "fixes": fixes,
            "docs": format!("{}/{}.md", catalogue::DOCS, r.code),
        })
    }

    fn json_edit(&self, e: &Edit, r: &Replacement) -> Value {
        let lines = Lines::new(self.source(r.file));
        let mut out = Map::new();
        out.insert("file".into(), json!(self.path_of(r.file)));
        match e {
            Edit::Set { at, value } => {
                out.insert("op".into(), json!("set"));
                out.insert("at".into(), json!(at));
                out.insert("value".into(), value.clone());
            }
            Edit::Remove { at } => {
                out.insert("op".into(), json!("remove"));
                out.insert("at".into(), json!(at));
            }
            Edit::Rename { at, to } => {
                out.insert("op".into(), json!("rename"));
                out.insert("at".into(), json!(at));
                out.insert("to".into(), json!(to));
            }
            Edit::Replace { at, key, value } => {
                out.insert("op".into(), json!("replace"));
                out.insert("at".into(), json!(at));
                out.insert("key".into(), json!(key));
                out.insert("value".into(), value.clone());
            }
            Edit::Dot(_) | Edit::SettingsText(_) => {
                out.insert("op".into(), json!("text"));
            }
        }
        out.insert(
            "range".into(),
            range(lines.position(r.range.start), lines.position(r.range.end)),
        );
        out.insert("text".into(), json!(r.text));
        Value::Object(out)
    }

    // ------------------------------------------------------------------
    // Fixing
    // ------------------------------------------------------------------

    /// The two files (`workflow.dot`, `settings.json`) with one fix applied.
    pub fn with_fix(&self, fix: &ResolvedFix) -> Option<(String, String)> {
        let mut out = Vec::new();
        for file in [File::Dot, File::Settings] {
            let rs: Vec<&Replacement> = fix
                .edits
                .iter()
                .map(|(_, r)| r)
                .filter(|r| r.file == file)
                .collect();
            out.push(edit::apply(self.source(file), &rs)?);
        }
        let settings = out.pop()?;
        Some((out.pop()?, settings))
    }

    /// The files with the first fix of each diagnostic applied, for the fixes of
    /// `applicability`; a fix that overlaps one taken before it is left for later.
    pub fn apply(&self, applicability: Applicability) -> Applied {
        let mut taken: Vec<(String, Vec<Replacement>)> = Vec::new();
        for r in self.resolved() {
            let Some(fix) = r.fixes.first() else { continue };
            if fix.applicability != applicability {
                continue;
            }
            let reps: Vec<Replacement> = fix.edits.iter().map(|(_, r)| r.clone()).collect();
            let overlaps = taken.iter().flat_map(|(_, t)| t).any(|t| {
                reps.iter().any(|r| {
                    r.file == t.file
                        && r.range.start < t.range.end.max(t.range.start + 1)
                        && t.range.start < r.range.end.max(r.range.start + 1)
                })
            });
            if !overlaps {
                taken.push((r.code.clone(), reps));
            }
        }
        let mut applied = Applied::default();
        for file in [File::Dot, File::Settings] {
            let all: Vec<&Replacement> = taken
                .iter()
                .flat_map(|(_, t)| t)
                .filter(|r| r.file == file)
                .collect();
            if all.is_empty() {
                continue;
            }
            let Some(text) = edit::apply(self.source(file), &all) else {
                continue;
            };
            for (code, reps) in &taken {
                let mine: Vec<&Replacement> = reps.iter().filter(|r| r.file == file).collect();
                if let Some(regions) = edit::changed(self.source(file), &mine) {
                    if !mine.is_empty() {
                        applied.changes.push((code.clone(), file, regions));
                    }
                }
            }
            match file {
                File::Dot => applied.dot = Some(text),
                File::Settings => applied.settings = Some(text),
            }
        }
        applied
    }
}

/// The result of [`Report::apply`]: the new texts of the files it changed, and each
/// change as the lines it touched.
#[derive(Debug, Default)]
pub struct Applied {
    pub dot: Option<String>,
    pub settings: Option<String>,
    pub changes: Vec<(String, File, Vec<Region>)>,
}

impl Applied {
    /// Each change: the code it fixed and where, then its lines as a diff.
    pub fn text(&self, report: &Report) -> String {
        let mut out = String::new();
        for (code, file, regions) in &self.changes {
            out.push_str(&format!("Fixed {code} in {}:\n", report.path_of(*file)));
            for region in regions {
                let lines = region.diff();
                let width = gutter(lines.iter().map(|l| l.0));
                for (n, mark, text) in lines {
                    out.push_str(&row(width, n, mark, text));
                }
            }
            out.push('\n');
        }
        out
    }
}

struct Place {
    file: File,
    start: (usize, usize),
    end: (usize, usize),
    mark: Option<(usize, usize)>,
    lines: Vec<usize>,
}

fn file_name(file: File) -> &'static str {
    match file {
        File::Dot => DOT_FILE,
        File::Settings => SETTINGS_FILE,
    }
}

fn quote(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// A pointer as people read it: `"actions" → "upload"`; the whole file is "the top
/// level".
pub fn path(pointer: &str) -> String {
    if pointer.is_empty() {
        return "the top level".to_string();
    }
    steps(pointer)
        .iter()
        .map(|s| quote(s))
        .collect::<Vec<_>>()
        .join(" → ")
}

/// A computed value: a string quoted, a list as its items joined by ` | `.
fn computed(v: &Value) -> String {
    match v {
        Value::Array(items) => items.iter().map(computed).collect::<Vec<_>>().join(" | "),
        other => edit::inline(other),
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn gutter(lines: impl Iterator<Item = usize>) -> usize {
    let digits = lines.max().unwrap_or(1).to_string().len();
    (digits + 1).max(3)
}

fn row(width: usize, n: usize, mark: char, text: &str) -> String {
    format!("{n:>width$} │{mark}{text}\n")
}

fn range(start: (usize, usize), end: (usize, usize)) -> Value {
    json!({"start": {"line": start.0, "column": start.1},
           "end": {"line": end.0, "column": end.1}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(diagnostics: Vec<Diagnostic>) -> Report {
        let dot = "digraph \"x\" {\n  dialect = \"runspore/0.2\";\n}\n".to_string();
        let settings = "{\n  \"actions\": {}\n}\n".to_string();
        let value = serde_json::from_str(&settings).ok();
        let spans = Some(Spans::scan(&settings, 32));
        Report::new(
            "x".into(),
            "x".into(),
            dot,
            settings,
            value,
            spans,
            diagnostics,
        )
    }

    #[test]
    fn a_broken_diagnostic_is_shown_as_broken_never_dropped() {
        let broken = [
            // A pointer that names nothing in the file.
            Diagnostic::new("action.no-kind", At::key("/actions/nope")),
            // A placeholder the code does not declare.
            Diagnostic::new("dot.no-steps", At::dot_line(1)).arg("extra", Arg::computed(1)),
            // An argument of another kind than declared.
            Diagnostic::new("settings.empty", At::key("/actions")).arg("place", Arg::computed(1)),
            // A placeholder left empty.
            Diagnostic::new("settings.empty", At::key("/actions")),
            // A code the catalogue does not have.
            Diagnostic::new("no.such-code", At::dot_line(1)),
            // An edit that does not apply.
            Diagnostic::new("dot.no-steps", At::dot_line(1)).fix(crate::diagnostic::Fix::safe(
                vec![Edit::Remove { at: "/nope".into() }],
            )),
        ];
        for d in broken {
            let code = d.code;
            let r = report(vec![d]);
            let resolved = r.resolved();
            assert_eq!(resolved.len(), 1, "{code}: dropped");
            assert_eq!(resolved[0].code, "spore.broken-message", "{code}");
            assert_eq!(
                resolved[0].message,
                format!("spore made a broken message for {code}, please report it.")
            );
        }
    }

    #[test]
    fn the_text_stops_after_twenty_blocks_and_says_how_to_see_the_rest() {
        let many: Vec<Diagnostic> = (0..25)
            .map(|_| Diagnostic::new("dot.no-steps", At::dot_line(1)))
            .collect();
        let text = report(many).text("spore validate --short x");
        assert_eq!(text.matches("── No steps").count(), MAX_BLOCKS);
        assert!(
            text.ends_with(
                "… and 5 more. spore validate --short x lists them all.\n\n25 problems in x. Nothing was run.\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_missed_check_asks_for_a_report() {
        let mut r = report(Vec::new());
        r.add(Diagnostic::new("spore.missed-check", At::dot_line(1)));
        let text = r.text("spore validate --short x");
        assert!(text.contains("spore.missed-check"), "{text}");
        assert!(
            text.contains("https://github.com/Web-tree/runspore/issues"),
            "{text}"
        );
        assert!(
            text.ends_with("1 problem in x. Nothing was run.\n"),
            "{text}"
        );
    }
}
