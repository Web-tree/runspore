//! Fixes as text: each [`Edit`] becomes a replacement of a byte range of the file as
//! the author wrote it, keeping their layout, so a fix can be shown on their own
//! lines and written back.

use std::ops::Range;

use serde_json::Value;

pub use crate::diagnostic::File;
use crate::diagnostic::{steps, Edit};
use crate::spans::{Lines, Spans};

/// A replacement of `range` in `file` by `text`, against the file as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub file: File,
    pub range: Range<usize>,
    pub text: String,
}

/// The texts an edit applies to.
pub struct Texts<'a> {
    pub dot: &'a str,
    pub settings: &'a str,
    pub spans: Option<&'a Spans>,
}

/// The replacement an edit makes, or `None` when it does not apply to these texts.
pub fn replacement(edit: &Edit, texts: &Texts<'_>) -> Option<Replacement> {
    let settings = |range: Range<usize>, text: String| {
        Some(Replacement {
            file: File::Settings,
            range,
            text,
        })
    };
    let text_edit = |file: File, text: &str, e: &crate::diagnostic::TextEdit| {
        let lines = Lines::new(text);
        if e.line == 0 || e.line > lines.count() + 1 || (e.end_line, e.end_col) < (e.line, e.col) {
            return None;
        }
        let start = lines.offset(e.line, e.col);
        let end = lines.offset(e.end_line, e.end_col);
        Some(Replacement {
            file,
            range: start..end,
            text: e.text.clone(),
        })
    };
    match edit {
        Edit::Dot(e) => text_edit(File::Dot, texts.dot, e),
        Edit::SettingsText(e) => text_edit(File::Settings, texts.settings, e),
        Edit::Replace { at, key, value } => {
            let span = texts.spans?.get(at)?;
            let start = span.key.as_ref()?.start;
            settings(
                start..span.value.end,
                format!("{}: {}", quote(key), inline(value)),
            )
        }
        Edit::Set { at, value } => {
            let (range, text) = set(texts.settings, texts.spans?, at, value)?;
            settings(range, text)
        }
        Edit::Remove { at } => settings(remove(texts.spans?, at)?, String::new()),
        Edit::Rename { at, to } => {
            let key = texts.spans?.get(at)?.key.clone()?;
            settings(key, serde_json::to_string(to).ok()?)
        }
    }
}

/// `text` with `replacements` applied; `None` when two overlap.
pub fn apply(text: &str, replacements: &[&Replacement]) -> Option<String> {
    let mut sorted: Vec<&&Replacement> = replacements.iter().collect();
    sorted.sort_by_key(|r| (r.range.start, r.range.end));
    for pair in sorted.windows(2) {
        if pair[0].range.end > pair[1].range.start {
            return None;
        }
    }
    let mut out = text.to_string();
    for r in sorted.iter().rev() {
        if r.range.end > out.len() || !out.is_char_boundary(r.range.start) {
            return None;
        }
        out.replace_range(r.range.clone(), &r.text);
    }
    Some(out)
}

/// The lines one replacement changes: before (numbered in the old text) and after
/// (numbered in the new text), lines that stay the same around them left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub before: Vec<(usize, String)>,
    pub after: Vec<(usize, String)>,
}

impl Region {
    /// The lines as they read after the change: a changed line as it is (`' '`), an
    /// added one marked `'+'`, a removed one (as it was) marked `'-'`.
    pub fn result(&self) -> Vec<(usize, char, &str)> {
        let paired = self.before.len().min(self.after.len());
        let mut lines: Vec<(usize, char, &str)> = self
            .after
            .iter()
            .enumerate()
            .map(|(i, (n, text))| (*n, if i < paired { ' ' } else { '+' }, text.as_str()))
            .collect();
        lines.extend(
            self.before[paired..]
                .iter()
                .map(|(n, text)| (*n, '-', text.as_str())),
        );
        lines
    }

    /// The lines as a diff: each old line marked `'-'`, then each new one `'+'`.
    pub fn diff(&self) -> Vec<(usize, char, &str)> {
        let before = self.before.iter().map(|(n, t)| (*n, '-', t.as_str()));
        let after = self.after.iter().map(|(n, t)| (*n, '+', t.as_str()));
        before.chain(after).collect()
    }
}

/// The regions `replacements` change, in text order.
pub fn changed(old: &str, replacements: &[&Replacement]) -> Option<Vec<Region>> {
    let new = apply(old, replacements)?;
    let (old_lines, new_lines) = (Lines::new(old), Lines::new(&new));
    let mut sorted: Vec<&&Replacement> = replacements.iter().collect();
    sorted.sort_by_key(|r| r.range.start);
    let mut shift: isize = 0;
    let mut regions = Vec::new();
    for r in sorted {
        let lines = |l: &Lines<'_>, a: usize, b: usize| -> Vec<(usize, String)> {
            let (first, last) = (l.position(a).0, l.position(b).0);
            (first..=last).map(|n| (n, l.line(n).to_string())).collect()
        };
        let start = r.range.start.checked_add_signed(shift)?;
        let mut before = lines(&old_lines, r.range.start, r.range.end);
        let mut after = lines(&new_lines, start, start + r.text.len());
        while !before.is_empty() && !after.is_empty() && before[0].1 == after[0].1 {
            before.remove(0);
            after.remove(0);
        }
        while let (Some(b), Some(a)) = (before.last(), after.last()) {
            if b.1 != a.1 {
                break;
            }
            before.pop();
            after.pop();
        }
        regions.push(Region { before, after });
        shift += r.text.len() as isize - r.range.len() as isize;
    }
    Some(regions)
}

/// The lines that differ between two texts, as `-` and `+` rows with their numbers.
pub fn line_diff(before: &str, after: &str) -> String {
    let (old, new): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let removed = (head..old.len() - tail).map(|i| (i + 1, '-', old[i]));
    let added = (head..new.len() - tail).map(|i| (i + 1, '+', new[i]));
    let rows: Vec<(usize, char, &str)> = removed.chain(added).collect();
    let width = (rows
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap_or(1)
        .to_string()
        .len()
        + 1)
    .max(3);
    rows.iter()
        .map(|(n, mark, text)| format!("{n:>width$} │{mark}{text}\n"))
        .collect()
}

/// A value on one line, as an author would type it: `["a", "b"]`, `{"k": 1}`.
pub fn inline(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(inline).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let members: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", quote(k), inline(v)))
                .collect();
            format!("{{{}}}", members.join(", "))
        }
        other => other.to_string(),
    }
}

fn quote(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

fn parent(at: &str) -> Option<(&str, &str)> {
    let i = at.rfind('/')?;
    Some((&at[..i], &at[i + 1..]))
}

fn set(text: &str, spans: &Spans, at: &str, value: &Value) -> Option<(Range<usize>, String)> {
    if let Some(span) = spans.get(at) {
        return Some((span.value.clone(), inline(value)));
    }
    // Wrap the value in each missing parent until one exists.
    let mut value = value.clone();
    let mut here = at.to_string();
    loop {
        let (up, _) = parent(&here)?;
        if spans.get(up).is_some() {
            break;
        }
        let key = steps(&here).pop()?;
        let mut object = serde_json::Map::new();
        object.insert(key, value);
        value = Value::Object(object);
        here = up.to_string();
    }
    let (up, _) = parent(&here)?;
    let key = steps(&here).pop()?;
    let container = spans.get(up)?.value.clone();
    let lines = Lines::new(text);
    let children = spans.children(up);
    let member = match text.as_bytes().get(container.start)? {
        b'{' => format!("{}: {}", quote(&key), inline(&value)),
        b'[' if key == "-" || key.parse::<usize>().ok() == Some(children.len()) => inline(&value),
        _ => return None,
    };
    let Some(last) = children.last() else {
        let open = text.as_bytes()[container.start] as char;
        let close = if open == '{' { '}' } else { ']' };
        return Some((container, format!("{open}{member}{close}")));
    };
    let last = spans.get(last)?;
    let last_start = last.key.as_ref().map_or(last.value.start, |k| k.start);
    let (open_line, _) = lines.position(container.start);
    let (last_line, _) = lines.position(last_start);
    let at = last.value.end;
    if last_line == open_line {
        return Some((at..at, format!(", {member}")));
    }
    let line = lines.line(last_line);
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    Some((at..at, format!(",\n{indent}{member}")))
}

fn remove(spans: &Spans, at: &str) -> Option<Range<usize>> {
    let (up, _) = parent(at)?;
    let siblings = spans.children(up);
    let i = siblings.iter().position(|s| s == at)?;
    let start = |p: &str| {
        spans
            .get(p)
            .map(|s| s.key.as_ref().map_or(s.value.start, |k| k.start))
    };
    let this = spans.get(at)?;
    if siblings.len() == 1 {
        let container = spans.get(up)?.value.clone();
        return Some(container.start + 1..container.end - 1);
    }
    if i > 0 {
        let previous = spans.get(&siblings[i - 1])?;
        return Some(previous.value.end..this.value.end);
    }
    Some(start(at)?..start(&siblings[1])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::TextEdit;
    use serde_json::json;

    fn run(text: &str, edit: Edit) -> String {
        let spans = Spans::scan(text, 32);
        let texts = Texts {
            dot: "",
            settings: text,
            spans: Some(&spans),
        };
        let r = replacement(&edit, &texts).expect("applies");
        apply(text, &[&r]).unwrap()
    }

    #[test]
    fn sets_adds_removes_and_renames_in_place() {
        let text = "{\n  \"a\": {\n    \"x\": 1\n  },\n  \"b\": [\"one two\"]\n}";
        let added = run(
            text,
            Edit::Set {
                at: "/a/y".into(),
                value: json!("z"),
            },
        );
        assert_eq!(
            added,
            "{\n  \"a\": {\n    \"x\": 1,\n    \"y\": \"z\"\n  },\n  \"b\": [\"one two\"]\n}"
        );
        let set = run(
            text,
            Edit::Set {
                at: "/b".into(),
                value: json!(["one", "two"]),
            },
        );
        assert!(set.contains("\"b\": [\"one\", \"two\"]"), "{set}");
        let removed = run(text, Edit::Remove { at: "/b".into() });
        assert_eq!(removed, "{\n  \"a\": {\n    \"x\": 1\n  }\n}");
        let first = run(text, Edit::Remove { at: "/a".into() });
        assert_eq!(first, "{\n  \"b\": [\"one two\"]\n}");
        let renamed = run(
            text,
            Edit::Rename {
                at: "/a/x".into(),
                to: "w".into(),
            },
        );
        assert!(renamed.contains("\"w\": 1"));
        let nested = run(
            "{\"a\": {}}",
            Edit::Set {
                at: "/a/b/c".into(),
                value: json!(1),
            },
        );
        assert_eq!(nested, "{\"a\": {\"b\": {\"c\": 1}}}");
        let inline = run(
            "{ \"a\": 1 }",
            Edit::Set {
                at: "/b".into(),
                value: json!(2),
            },
        );
        assert_eq!(inline, "{ \"a\": 1, \"b\": 2 }");
    }

    #[test]
    fn changed_lines_mark_what_was_added() {
        let text = "{\n  \"a\": 1\n}\n";
        let spans = Spans::scan(text, 32);
        let texts = Texts {
            dot: "",
            settings: text,
            spans: Some(&spans),
        };
        let edit = Edit::Set {
            at: "/b".into(),
            value: json!(2),
        };
        let r = replacement(&edit, &texts).unwrap();
        let regions = changed(text, &[&r]).unwrap();
        assert_eq!(
            regions[0].result(),
            [(2, ' ', "  \"a\": 1,"), (3, '+', "  \"b\": 2")]
        );
        let dot = "digraph g {\n  a;\n}\n";
        let texts = Texts {
            dot,
            settings: "",
            spans: None,
        };
        let insert = Edit::Dot(TextEdit {
            line: 3,
            col: 1,
            end_line: 3,
            end_col: 1,
            text: "  a -> b;\n".into(),
        });
        let r = replacement(&insert, &texts).unwrap();
        assert_eq!(
            apply(dot, &[&r]).unwrap(),
            "digraph g {\n  a;\n  a -> b;\n}\n"
        );
        assert_eq!(
            changed(dot, &[&r]).unwrap()[0].result(),
            [(3, '+', "  a -> b;")]
        );
    }
}
