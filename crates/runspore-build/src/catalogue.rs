//! Every message the build can give, written once per code in
//! `catalogue/catalogue.toml`, with a wrong/fixed example folder for each.
//!
//! A template is text with `{name}` placeholders and nothing else: no conditions, no
//! plurals. Wording that a template cannot express is another code.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::diagnostic::Severity;

/// The catalogue as it ships inside spore.
const TEXT: &str = include_str!("../catalogue/catalogue.toml");

/// Names spore fills from where a diagnostic points; a check never passes them.
/// `{owner}` is the step or the action the place is in, with its kind: `step "build"`.
pub const HOST_NAMES: [&str; 5] = ["action", "step", "owner", "file", "workflow"];

/// Where the help page of each code is published.
pub const DOCS: &str = "https://github.com/Web-tree/runspore/blob/main/docs/codes";

/// What kind of value a declared placeholder holds (see [`crate::diagnostic::Arg`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArgKind {
    Input,
    Key,
    Place,
    Computed,
    Code,
}

impl ArgKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArgKind::Input => "input",
            ArgKind::Key => "key",
            ArgKind::Place => "place",
            ArgKind::Computed => "computed",
            ArgKind::Code => "code",
        }
    }

    /// Whether the value comes from the author's own file.
    pub fn from_files(self) -> bool {
        matches!(self, ArgKind::Input | ArgKind::Key | ArgKind::Place)
    }
}

/// One code.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Entry {
    pub code: String,
    pub severity: Severity,
    /// Short and general, with no placeholders: the code's name for people.
    pub title: String,
    /// The block's heading when it names a value; `title` otherwise.
    #[serde(default)]
    pub heading: Option<String>,
    /// One plain sentence about what is wrong, quoting the author's values.
    pub message: String,
    /// What to do, as a template; the first fix is its example.
    pub fix: String,
    /// Whether the text output shows the first fix applied to the author's lines.
    #[serde(default)]
    pub show_fix: bool,
    #[serde(default)]
    pub why: Option<String>,
    /// A longer explanation for `spore explain`; when there is one, each block ends
    /// with "More: spore explain <code>".
    #[serde(default)]
    pub explanation: Option<String>,
    /// The placeholders a check passes, by kind. Host names need no entry.
    #[serde(default)]
    pub args: BTreeMap<String, ArgKind>,
    /// The folder under `catalogue/examples/` that produces this code; none for
    /// spore's own codes, which no folder produces.
    #[serde(default)]
    pub example: Option<String>,
    /// The other codes that folder produces.
    #[serde(default)]
    pub also: Vec<String>,
    /// What is wrong is something missing, so the message has nothing of the
    /// author's to quote.
    #[serde(default)]
    pub absent: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    code: Vec<Entry>,
}

pub struct Catalogue {
    pub entries: Vec<Entry>,
    by_code: BTreeMap<String, usize>,
}

impl Catalogue {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut by_code = BTreeMap::new();
        for (i, entry) in file.code.iter().enumerate() {
            if by_code.insert(entry.code.clone(), i).is_some() {
                return Err(format!("code {} is in the catalogue twice", entry.code));
            }
        }
        Ok(Self {
            entries: file.code,
            by_code,
        })
    }

    pub fn get(&self, code: &str) -> Option<&Entry> {
        self.by_code.get(code).map(|&i| &self.entries[i])
    }
}

/// The catalogue spore ships.
pub fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| Catalogue::parse(TEXT).expect("the catalogue parses (cargo test)"))
}

/// A piece of a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(String),
    Hole(&'a str),
}

/// Splits a template into text and `{name}` holes; `{{` and `}}` are braces.
pub fn pieces(template: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("{{") || rest.starts_with("}}") {
            text.push(c);
            rest = &rest[2..];
            continue;
        }
        match c {
            '{' => {
                let end = rest
                    .find('}')
                    .ok_or_else(|| format!("`{{` without `}}` in {template:?}"))?;
                let name = &rest[1..end];
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                    return Err(format!("placeholder {{{name}}} in {template:?}"));
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Hole(name));
                rest = &rest[end + 1..];
            }
            '}' => return Err(format!("`}}` without `{{` in {template:?}")),
            c => {
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

/// The placeholders a template uses.
pub fn holes(template: &str) -> Result<BTreeSet<&str>, String> {
    Ok(pieces(template)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Hole(name) => Some(name),
            Piece::Text(_) => None,
        })
        .collect())
}

/// Fills a template; `None` when a placeholder has no value.
pub fn fill(template: &str, value: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    for piece in pieces(template).ok()? {
        match piece {
            Piece::Text(text) => out.push_str(&text),
            Piece::Hole(name) => out.push_str(&value(name)?),
        }
    }
    Some(out)
}

/// Codes about spore itself rather than the author's files.
pub fn internal(code: &str) -> bool {
    code.starts_with("spore.")
}

/// What is wrong with an entry's templates (the quality checks of the catalogue),
/// empty when nothing is.
pub fn quality(entry: &Entry) -> Vec<String> {
    let mut found = Vec::new();
    let code = &entry.code;
    let mut used: BTreeSet<&str> = BTreeSet::new();
    let templates = [
        ("title", Some(&entry.title)),
        ("heading", entry.heading.as_ref()),
        ("message", Some(&entry.message)),
        ("fix", Some(&entry.fix)),
        ("why", entry.why.as_ref()),
    ];
    for (field, template) in templates {
        let Some(template) = template else { continue };
        match holes(template) {
            Err(e) => found.push(format!("{code}: {field}: {e}")),
            Ok(holes) => {
                if field == "title" && !holes.is_empty() {
                    found.push(format!("{code}: the title has placeholders"));
                }
                for hole in &holes {
                    if !HOST_NAMES.contains(hole) && !entry.args.contains_key(*hole) {
                        found.push(format!(
                            "{code}: {field} uses {{{hole}}}, which is not declared"
                        ));
                    }
                }
                if field == "message" && !internal(code) && !entry.absent {
                    let names_a_value = holes.iter().any(|h| {
                        HOST_NAMES.contains(h) || entry.args.get(*h).is_some_and(|k| k.from_files())
                    });
                    if !names_a_value {
                        found.push(format!(
                            "{code}: the message names nothing from the author's files"
                        ));
                    }
                }
                used.extend(holes);
            }
        }
    }
    for name in entry.args.keys() {
        if HOST_NAMES.contains(&name.as_str()) {
            found.push(format!(
                "{code}: {{{name}}} is a host name; it needs no declaration"
            ));
        } else if !used.contains(name.as_str()) {
            found.push(format!("{code}: {{{name}}} is declared and never used"));
        }
    }
    if entry.title.trim().is_empty()
        || entry.message.trim().is_empty()
        || entry.fix.trim().is_empty()
    {
        found.push(format!("{code}: title, message and fix are required"));
    }
    if entry.example.is_none() && !internal(code) {
        found.push(format!("{code}: no example"));
    }
    if entry.also.contains(&entry.code) {
        found.push(format!("{code}: `also` names the code itself"));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_have_holes_and_braces() {
        assert_eq!(
            pieces("a {b} {{c}}").unwrap(),
            [
                Piece::Text("a ".into()),
                Piece::Hole("b"),
                Piece::Text(" {c}".into())
            ]
        );
        assert!(pieces("a {B}").is_err());
        assert!(pieces("a {b").is_err());
        assert_eq!(
            fill("x {a} y", |n| (n == "a").then(|| "1".into())).as_deref(),
            Some("x 1 y")
        );
        assert_eq!(fill("x {b}", |_| None), None);
    }
}
