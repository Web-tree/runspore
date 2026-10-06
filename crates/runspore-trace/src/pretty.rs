//! Stable, readable JSON text for trace files and mismatch reports.
//!
//! Members of the trace's own records are written in the order the format
//! documents them. Members of free-form JSON (workflow, payloads, snapshots,
//! details) are written in canonical order, by UTF-16 code units, so a file does
//! not depend on how its values were built. A container is written on one line
//! when that line stays within [`LINE_WIDTH`].

use serde_json::Value;

/// Longest line a container is kept on, in characters.
const LINE_WIDTH: usize = 100;

pub(crate) enum Doc {
    /// A scalar, already rendered as JSON text.
    Atom(String),
    Array(Vec<Doc>),
    /// Members in the order they are written; keys are already quoted.
    Object(Vec<(String, Doc)>),
}

impl Doc {
    pub(crate) fn string(text: &str) -> Doc {
        Doc::Atom(quote(text))
    }

    pub(crate) fn null() -> Doc {
        Doc::Atom("null".to_string())
    }

    pub(crate) fn number(number: u32) -> Doc {
        Doc::Atom(number.to_string())
    }

    /// Free-form JSON, with object members in canonical order.
    pub(crate) fn value(value: &Value) -> Doc {
        match value {
            Value::Array(items) => Doc::Array(items.iter().map(Doc::value).collect()),
            Value::Object(map) => {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                Doc::Object(
                    entries
                        .into_iter()
                        .map(|(key, item)| (quote(key), Doc::value(item)))
                        .collect(),
                )
            }
            scalar => Doc::Atom(scalar.to_string()),
        }
    }

    /// A record: members in the given order, absent ones left out.
    pub(crate) fn record<const N: usize>(members: [(&str, Option<Doc>); N]) -> Doc {
        Doc::Object(
            members
                .into_iter()
                .filter_map(|(key, doc)| Some((quote(key), doc?)))
                .collect(),
        )
    }

    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        self.write(0, 0, &mut out);
        out
    }

    /// `column` is where the value starts on its line. One column is kept
    /// free for the comma that may follow it.
    fn write(&self, depth: usize, column: usize, out: &mut String) {
        if self
            .inline_len(LINE_WIDTH.saturating_sub(column + 1))
            .is_some()
        {
            self.write_inline(out);
            return;
        }
        match self {
            Doc::Atom(text) => out.push_str(text),
            Doc::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    line(depth + 1, out);
                    item.write(depth + 1, 2 * (depth + 1), out);
                }
                line(depth, out);
                out.push(']');
            }
            Doc::Object(members) => {
                out.push('{');
                for (index, (key, item)) in members.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    line(depth + 1, out);
                    out.push_str(key);
                    out.push_str(": ");
                    let column = 2 * (depth + 1) + key.chars().count() + 2;
                    item.write(depth + 1, column, out);
                }
                line(depth, out);
                out.push('}');
            }
        }
    }

    fn write_inline(&self, out: &mut String) {
        match self {
            Doc::Atom(text) => out.push_str(text),
            Doc::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    item.write_inline(out);
                }
                out.push(']');
            }
            Doc::Object(members) => {
                out.push('{');
                for (index, (key, item)) in members.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(key);
                    out.push_str(": ");
                    item.write_inline(out);
                }
                out.push('}');
            }
        }
    }

    /// Length of the one-line form, or `None` once it exceeds `limit`.
    fn inline_len(&self, limit: usize) -> Option<usize> {
        let len = match self {
            Doc::Atom(text) => text.chars().count(),
            Doc::Array(items) => {
                let mut len = 2;
                for (index, item) in items.iter().enumerate() {
                    len += if index > 0 { 2 } else { 0 };
                    len += item.inline_len(limit.checked_sub(len)?)?;
                }
                len
            }
            Doc::Object(members) => {
                let mut len = 2;
                for (index, (key, item)) in members.iter().enumerate() {
                    len += if index > 0 { 2 } else { 0 };
                    len += key.chars().count() + 2;
                    len += item.inline_len(limit.checked_sub(len)?)?;
                }
                len
            }
        };
        (len <= limit).then_some(len)
    }
}

fn quote(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

fn line(depth: usize, out: &mut String) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn short_containers_stay_on_one_line() {
        let doc = Doc::value(&json!({"b": [1, 2], "a": {"x": null}}));
        assert_eq!(doc.render(), r#"{"a": {"x": null}, "b": [1, 2]}"#);
    }

    #[test]
    fn long_containers_expand_one_member_per_line() {
        let long = "x".repeat(LINE_WIDTH);
        let doc = Doc::value(&json!({"k": [long.clone(), {"a": 1}], "e": {}}));
        assert_eq!(
            doc.render(),
            format!("{{\n  \"e\": {{}},\n  \"k\": [\n    \"{long}\",\n    {{\"a\": 1}}\n  ]\n}}")
        );
    }

    #[test]
    fn free_form_members_are_in_utf16_order() {
        let doc = Doc::value(&json!({"\u{fb33}": 1, "\u{1f600}": 2, "a": 3}));
        assert_eq!(
            doc.render(),
            "{\"a\": 3, \"\u{1f600}\": 2, \"\u{fb33}\": 1}"
        );
    }

    #[test]
    fn records_keep_their_order_and_skip_absent_members() {
        let doc = Doc::record([
            ("z", Some(Doc::number(1))),
            ("skipped", None),
            ("a", Some(Doc::string("q\"\n"))),
        ]);
        assert_eq!(doc.render(), r#"{"z": 1, "a": "q\"\n"}"#);
    }

    #[test]
    fn output_parses_back_to_the_same_value() {
        let value = json!({
            "nested": {"list": [1, -2, "three", null, true, {"deep": ["x".repeat(120)]}]},
            "text": "tab\t and \u{1} and \u{1f600}",
        });
        let text = Doc::value(&value).render();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), value);
    }
}
