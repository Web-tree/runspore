//! Where each value of a JSON text is: the byte range of every member's key and
//! value by JSON Pointer, so a diagnostic can show and mark the author's own lines,
//! and an edit can change them in place. Also finds what JSON allows and the
//! canonical domain does not (kernel.md §1): a key given twice, a number that is
//! not an integer or is out of range, nesting past the depth limit.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::diagnostic::pointer;

/// Integers the canonical domain holds: |n| ≤ 2^53 − 1.
const MAX_SAFE: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The key with its quotes, for an object member.
    pub key: Option<Range<usize>>,
    pub value: Range<usize>,
}

/// Something the canonical domain refuses, by the pointer of the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Odd {
    /// A key given again in the same object; the span is the later member's.
    Duplicate(String, Span),
    NotInteger(String),
    OutOfRange(String),
    TooDeep(String),
}

#[derive(Debug, Clone, Default)]
pub struct Spans {
    spans: BTreeMap<String, Span>,
    /// The members or items of each container, in text order.
    children: BTreeMap<String, Vec<String>>,
    pub odd: Vec<Odd>,
}

impl Spans {
    /// Scans `text`, which must be JSON.
    pub fn scan(text: &str, max_depth: usize) -> Self {
        let mut scanner = Scanner {
            text: text.as_bytes(),
            at: 0,
            spans: Spans::default(),
            max_depth,
        };
        scanner.blank();
        let start = scanner.at;
        scanner.value("", 1);
        let end = scanner.at;
        scanner.spans.spans.insert(
            String::new(),
            Span {
                key: None,
                value: start..end,
            },
        );
        scanner.spans
    }

    pub fn get(&self, pointer: &str) -> Option<&Span> {
        self.spans.get(pointer)
    }

    pub fn children(&self, pointer: &str) -> &[String] {
        self.children.get(pointer).map_or(&[], Vec::as_slice)
    }
}

struct Scanner<'a> {
    text: &'a [u8],
    at: usize,
    spans: Spans,
    max_depth: usize,
}

impl Scanner<'_> {
    fn blank(&mut self) {
        while self.at < self.text.len() && self.text[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.text.get(self.at).copied().unwrap_or(0)
    }

    /// Scans the value at `self.at`, at `depth` (1 for the top level).
    fn value(&mut self, here: &str, depth: usize) {
        match self.peek() {
            b'{' => {
                self.deep(here, depth);
                self.at += 1;
                let mut keys: Vec<String> = Vec::new();
                let mut children = Vec::new();
                loop {
                    self.blank();
                    match self.peek() {
                        b'}' => {
                            self.at += 1;
                            break;
                        }
                        b',' => {
                            self.at += 1;
                            continue;
                        }
                        b'"' => {}
                        _ => break,
                    }
                    let key_start = self.at;
                    self.string();
                    let key_end = self.at;
                    let key: String =
                        serde_json::from_slice(&self.text[key_start..key_end]).unwrap_or_default();
                    self.blank();
                    if self.peek() == b':' {
                        self.at += 1;
                    }
                    self.blank();
                    let child = pointer(here, &key);
                    let value_start = self.at;
                    self.value(&child, depth + 1);
                    let span = Span {
                        key: Some(key_start..key_end),
                        value: value_start..self.at,
                    };
                    if keys.contains(&key) {
                        self.spans.odd.push(Odd::Duplicate(child, span));
                    } else {
                        keys.push(key);
                        self.spans.spans.insert(child.clone(), span);
                        children.push(child);
                    }
                }
                self.spans.children.insert(here.to_string(), children);
            }
            b'[' => {
                self.deep(here, depth);
                self.at += 1;
                let mut children = Vec::new();
                loop {
                    self.blank();
                    match self.peek() {
                        b']' => {
                            self.at += 1;
                            break;
                        }
                        b',' => {
                            self.at += 1;
                            continue;
                        }
                        0 => break,
                        _ => {}
                    }
                    let child = pointer(here, &children.len().to_string());
                    let start = self.at;
                    self.value(&child, depth + 1);
                    self.spans.spans.insert(
                        child.clone(),
                        Span {
                            key: None,
                            value: start..self.at,
                        },
                    );
                    children.push(child);
                }
                self.spans.children.insert(here.to_string(), children);
            }
            b'"' => self.string(),
            b'-' | b'0'..=b'9' => self.number(here),
            _ => {
                // true, false, null
                while self.peek().is_ascii_alphabetic() {
                    self.at += 1;
                }
            }
        }
    }

    fn deep(&mut self, here: &str, depth: usize) {
        if depth > self.max_depth && !self.spans.odd.iter().any(|o| matches!(o, Odd::TooDeep(_))) {
            self.spans.odd.push(Odd::TooDeep(here.to_string()));
        }
    }

    fn string(&mut self) {
        self.at += 1;
        while self.at < self.text.len() {
            match self.text[self.at] {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    fn number(&mut self, here: &str) {
        let start = self.at;
        while matches!(self.peek(), b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.text[start..self.at]).unwrap_or_default();
        if text.contains(['.', 'e', 'E']) {
            // A zero in any spelling is the integer 0, as `canonical::parse` reads it.
            let n = text.parse::<f64>().unwrap_or(f64::NAN);
            if n.fract() == 0.0 && n.abs() > MAX_SAFE as f64 {
                self.spans.odd.push(Odd::OutOfRange(here.to_string()));
            } else if n != 0.0 {
                self.spans.odd.push(Odd::NotInteger(here.to_string()));
            }
        } else if text
            .trim_start_matches('-')
            .parse::<u64>()
            .map_or(true, |n| n > MAX_SAFE)
        {
            self.spans.odd.push(Odd::OutOfRange(here.to_string()));
        }
    }
}

/// Line and column arithmetic over a text.
#[derive(Debug, Clone)]
pub struct Lines<'a> {
    pub text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { text, starts }
    }

    pub fn count(&self) -> usize {
        if self.text.ends_with('\n') {
            self.starts.len() - 1
        } else {
            self.starts.len()
        }
    }

    /// The 1-based line and character column of a byte offset.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&s| s <= offset);
        let start = self.starts[line - 1];
        let col = self.text[start..offset.min(self.text.len())]
            .chars()
            .count()
            + 1;
        (line, col)
    }

    /// The byte offset of a 1-based line and character column, clamped to the line.
    pub fn offset(&self, line: usize, col: usize) -> usize {
        let Some(&start) = self.starts.get(line.saturating_sub(1)) else {
            return self.text.len();
        };
        let text = self.line(line);
        start
            + text
                .char_indices()
                .nth(col.saturating_sub(1))
                .map_or(text.len(), |(i, _)| i)
    }

    /// Line `n` without its newline; empty past the end.
    pub fn line(&self, n: usize) -> &'a str {
        let Some(&start) = self.starts.get(n.saturating_sub(1)) else {
            return "";
        };
        let end = self.starts.get(n).map_or(self.text.len(), |&e| e - 1);
        self.text.get(start..end.max(start)).unwrap_or("")
    }

    /// The byte offset where line `n` starts.
    pub fn start(&self, n: usize) -> usize {
        self.starts
            .get(n.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keys_values_and_odd_values() {
        let text =
            "{\n  \"a\": {\"x\": [1, {\"b\": 2.5}]},\n  \"a\": 3,\n  \"n\": 9007199254740992\n}";
        let spans = Spans::scan(text, 32);
        let a = spans.get("/a").unwrap();
        assert_eq!(&text[a.key.clone().unwrap()], "\"a\"");
        assert_eq!(&text[spans.get("/a/x/1/b").unwrap().value.clone()], "2.5");
        assert_eq!(&text[spans.get("/a/x/0").unwrap().value.clone()], "1");
        assert_eq!(spans.children("/a/x"), ["/a/x/0", "/a/x/1"]);
        assert!(matches!(&spans.odd[0], Odd::NotInteger(p) if p == "/a/x/1/b"));
        assert!(matches!(&spans.odd[1], Odd::Duplicate(p, _) if p == "/a"));
        assert!(matches!(&spans.odd[2], Odd::OutOfRange(p) if p == "/n"));
        let lines = Lines::new(text);
        assert_eq!(lines.position(a.key.clone().unwrap().start), (2, 3));
        assert_eq!(lines.line(3), "  \"a\": 3,");
        assert_eq!(lines.offset(2, 3), a.key.clone().unwrap().start);
    }

    #[test]
    fn finds_nesting_past_the_limit() {
        let spans = Spans::scan("{\"a\": [[1]]}", 2);
        assert_eq!(spans.odd, [Odd::TooDeep("/a/0".into())]);
    }
}
