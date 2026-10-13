//! The DOT dialect of spec/dot.md §2–§3: a lexer, a parser for the accepted grammar,
//! and the checks that turn the parse into a [`Graph`]. A wrong dialect stops
//! everything; a syntax error stops the reading of this file; every other problem is
//! collected and the graph is still returned, so the build can go on checking.

// A diagnostic is the error of the reading functions; it is not on a hot path.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::diagnostic::{Applicability, Arg, At, Diagnostic, Edit, Fix, Found, TextEdit};
use crate::near::closest;
use crate::{is_name, is_node_id, is_outcome, DIALECT};

/// Ignored so the file still renders; spec/dot.md §3 Render attributes.
const RENDER: [&str; 25] = [
    "label",
    "xlabel",
    "shape",
    "style",
    "color",
    "fillcolor",
    "fontcolor",
    "fontname",
    "fontsize",
    "penwidth",
    "peripheries",
    "width",
    "height",
    "fixedsize",
    "arrowhead",
    "arrowtail",
    "dir",
    "weight",
    "constraint",
    "tooltip",
    "rankdir",
    "splines",
    "nodesep",
    "ranksep",
    "bgcolor",
];

pub const KINDS: [&str; 4] = ["activity", "await-signal", "complete", "fail"];

const MAX_NODES: usize = runspore_types::model::MAX_NODES;

/// A place in the text: line, 1-based column and length in characters, on one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spot {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    /// Where the text ends, when it goes on past its first line (a quoted string
    /// continued with `\` or `+`); the end is excluded.
    pub end: Option<(usize, usize)>,
}

impl Spot {
    pub fn at(self) -> At {
        At::dot(self.line, self.col, self.len)
    }

    /// A text edit replacing this spot.
    fn replace(self, text: impl Into<String>) -> Edit {
        let (end_line, end_col) = self.end.unwrap_or((self.line, self.col + self.len));
        Edit::Dot(TextEdit {
            line: self.line,
            col: self.col,
            end_line,
            end_col,
            text: text.into(),
        })
    }
}

/// An edit adding `text` (whole lines) after line `after`: on the next line, or just
/// before the graph's closing brace when that shares the line.
fn insert_after(after: usize, close: Spot, text: String) -> Edit {
    if after >= close.line {
        return Edit::Dot(TextEdit {
            line: close.line,
            col: close.col,
            end_line: close.line,
            end_col: close.col,
            text: format!("\n{text}"),
        });
    }
    insert_lines(after + 1, text)
}

/// An edit inserting `text` (whole lines) before line `line`.
fn insert_lines(line: usize, text: String) -> Edit {
    Edit::Dot(TextEdit {
        line,
        col: 1,
        end_line: line,
        end_col: 1,
        text,
    })
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// An ID; `true` when it was quoted, so it is never a keyword.
    Id(String, bool),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eq,
    Semi,
    Comma,
    Colon,
    Arrow,
    Undirected,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    spot: Spot,
}

/// A token as a message quotes it.
fn describe(tok: &Tok) -> String {
    let text = match tok {
        Tok::Id(s, _) => s.as_str(),
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::Eq => "=",
        Tok::Semi => ";",
        Tok::Comma => ",",
        Tok::Colon => ":",
        Tok::Arrow => "->",
        Tok::Undirected => "--",
    };
    Value::String(text.to_string()).to_string()
}

fn is_id_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_id_char(c: char) -> bool {
    is_id_start(c) || c.is_ascii_digit()
}

/// A DOT problem that stops the reading of the file.
fn unsupported(spot: Spot, feature: &str) -> Diagnostic {
    Diagnostic::new("dot.unsupported", spot.at()).arg("feature", Arg::Code(feature.into()))
}

fn unclosed(spot: Spot, what: &str, closer: &str) -> Diagnostic {
    Diagnostic::new("dot.unclosed", spot.at())
        .arg("what", Arg::Code(what.into()))
        .arg("closer", Arg::Code(closer.into()))
}

fn lex(text: &str) -> Result<Vec<Token>, Diagnostic> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let (mut line, mut col) = (1, 1);
    let mut at_line_start = true;
    let mut i = 0;
    // Advances past `n` characters, keeping line and column.
    let step = |i: &mut usize, line: &mut usize, col: &mut usize, n: usize| {
        for _ in 0..n {
            if chars.get(*i) == Some(&'\n') {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
            *i += 1;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            at_line_start = true;
            step(&mut i, &mut line, &mut col, 1);
            continue;
        }
        if c.is_whitespace() {
            step(&mut i, &mut line, &mut col, 1);
            continue;
        }
        let start = Spot {
            line,
            col,
            len: 1,
            end: None,
        };
        let line_start = std::mem::replace(&mut at_line_start, false);
        let next = chars.get(i + 1).copied();
        let single = match c {
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            '=' => Some(Tok::Eq),
            ';' => Some(Tok::Semi),
            ',' => Some(Tok::Comma),
            ':' => Some(Tok::Colon),
            _ => None,
        };
        if let Some(tok) = single {
            tokens.push(Token { tok, spot: start });
            step(&mut i, &mut line, &mut col, 1);
            continue;
        }
        match c {
            '#' if line_start => {
                while i < chars.len() && chars[i] != '\n' {
                    step(&mut i, &mut line, &mut col, 1);
                }
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    step(&mut i, &mut line, &mut col, 1);
                }
            }
            '/' if next == Some('*') => {
                step(&mut i, &mut line, &mut col, 2);
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(unclosed(Spot { len: 2, ..start }, "a /* comment", "*/"))
                        }
                        Some('*') if chars.get(i + 1) == Some(&'/') => {
                            step(&mut i, &mut line, &mut col, 2);
                            break;
                        }
                        Some(_) => step(&mut i, &mut line, &mut col, 1),
                    }
                }
            }
            '-' if next == Some('>') => {
                tokens.push(Token {
                    tok: Tok::Arrow,
                    spot: Spot { len: 2, ..start },
                });
                step(&mut i, &mut line, &mut col, 2);
            }
            '-' if next == Some('-') => {
                tokens.push(Token {
                    tok: Tok::Undirected,
                    spot: Spot { len: 2, ..start },
                });
                step(&mut i, &mut line, &mut col, 2);
            }
            '<' => return Err(unsupported(start, "HTML strings (<…>)")),
            '"' => {
                let mut value = String::new();
                loop {
                    let Some(n) = quoted(&chars, i, &mut value) else {
                        return Err(unclosed(start, "a quoted string", "\""));
                    };
                    step(&mut i, &mut line, &mut col, n);
                    // `"a" + "b"` concatenates.
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if chars.get(j) != Some(&'+') {
                        break;
                    }
                    let mut k = j + 1;
                    while k < chars.len() && chars[k].is_whitespace() {
                        k += 1;
                    }
                    if chars.get(k) != Some(&'"') {
                        let n = j - i;
                        step(&mut i, &mut line, &mut col, n);
                        return Err(Diagnostic::new(
                            "dot.syntax",
                            Spot {
                                line,
                                col,
                                len: 1,
                                end: None,
                            }
                            .at(),
                        )
                        .arg("found", Arg::Code("\"+\"".into()))
                        .arg("expected", Arg::Code("a quoted string after \"+\"".into())));
                    }
                    let n = k - i;
                    step(&mut i, &mut line, &mut col, n);
                }
                let spot = if line == start.line {
                    Spot {
                        len: col - start.col,
                        ..start
                    }
                } else {
                    Spot {
                        len: 1,
                        end: Some((line, col)),
                        ..start
                    }
                };
                tokens.push(Token {
                    tok: Tok::Id(value, true),
                    spot,
                });
            }
            c if c.is_ascii_digit() || c == '.' || c == '-' => {
                let begin = i;
                let mut end = i + 1;
                while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
                    end += 1;
                }
                let mut word_end = end;
                while word_end < chars.len()
                    && (is_id_char(chars[word_end]) || chars[word_end] == '-')
                {
                    word_end += 1;
                }
                let numeral: String = chars[begin..end].iter().collect();
                let digits = numeral.chars().filter(char::is_ascii_digit).count();
                let dots = numeral.matches('.').count();
                if digits == 0 || dots > 1 || word_end > end {
                    let word: String = chars[begin..word_end].iter().collect();
                    let spot = Spot {
                        len: word_end - begin,
                        ..start
                    };
                    return Err(Diagnostic::new("dot.number-id", spot.at())
                        .arg("text", Arg::computed(word.clone()))
                        .fix(
                            Fix::safe(vec![spot.replace(Value::String(word.clone()).to_string())])
                                .arg("quoted", Arg::Code(Value::String(word).to_string())),
                        ));
                }
                tokens.push(Token {
                    tok: Tok::Id(numeral, false),
                    spot: Spot {
                        len: end - begin,
                        ..start
                    },
                });
                step(&mut i, &mut line, &mut col, end - begin);
            }
            c if is_id_start(c) => {
                let begin = i;
                let mut end = i;
                while end < chars.len() && is_id_char(chars[end]) {
                    end += 1;
                }
                tokens.push(Token {
                    tok: Tok::Id(chars[begin..end].iter().collect(), false),
                    spot: Spot {
                        len: end - begin,
                        ..start
                    },
                });
                step(&mut i, &mut line, &mut col, end - begin);
            }
            c => {
                return Err(Diagnostic::new("dot.syntax", start.at())
                    .arg("found", Arg::Code(Value::String(c.to_string()).to_string()))
                    .arg("expected", Arg::Code("a statement".into())))
            }
        }
    }
    Ok(tokens)
}

/// Reads the quoted string opening at `chars[i]`; returns how many characters it
/// takes, closing quote included. Only `\"` is an escape and `\` before a newline
/// continues the line, as in Graphviz; any other backslash is kept.
fn quoted(chars: &[char], i: usize, out: &mut String) -> Option<usize> {
    let mut j = i + 1;
    loop {
        match *chars.get(j)? {
            '"' => return Some(j + 1 - i),
            '\\' if chars.get(j + 1) == Some(&'"') => {
                out.push('"');
                j += 2;
            }
            '\\' if chars.get(j + 1) == Some(&'\n') => j += 2,
            c => {
                out.push(c);
                j += 1;
            }
        }
    }
}

fn keyword(tok: &Tok, word: &str) -> bool {
    matches!(tok, Tok::Id(s, false) if s.eq_ignore_ascii_case(word))
}

// ---------------------------------------------------------------------------
// The dialect, checked before anything else
// ---------------------------------------------------------------------------

/// The `dialect` graph attribute, found before the full parse so that a missing or
/// unknown dialect is the only problem reported (spec/dot.md §3).
fn dialect(tokens: &[Token]) -> Result<(), Diagnostic> {
    let mut depth = 0usize;
    let mut graph_list = false;
    let mut bracket = 0usize;
    let mut found = None;
    for (i, t) in tokens.iter().enumerate() {
        match &t.tok {
            Tok::LBrace => depth += 1,
            Tok::RBrace => depth = depth.saturating_sub(1),
            Tok::LBracket => {
                bracket += 1;
                graph_list = i > 0 && keyword(&tokens[i - 1].tok, "graph");
            }
            Tok::RBracket => bracket = bracket.saturating_sub(1),
            Tok::Id(name, _) if name == "dialect" && depth == 1 && (bracket == 0 || graph_list) => {
                if let (Some(Tok::Eq), Some(value)) =
                    (tokens.get(i + 1).map(|t| &t.tok), tokens.get(i + 2))
                {
                    if let Tok::Id(v, _) = &value.tok {
                        found = Some((v.clone(), value.spot));
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let quoted_dialect = Value::String(DIALECT.to_string()).to_string();
    match found {
        None => {
            let head = tokens.first().map_or(1, |t| t.spot.line);
            let line = format!("dialect = {quoted_dialect};");
            let mut d = Diagnostic::new("dot.no-dialect", At::dot_line(head))
                .arg("line", Arg::Code(line.clone()));
            if let Some(open) = tokens.iter().find(|t| t.tok == Tok::LBrace) {
                let after = (open.spot.line, open.spot.col + 1);
                let edit = Edit::Dot(TextEdit {
                    line: after.0,
                    col: after.1,
                    end_line: after.0,
                    end_col: after.1,
                    text: format!("\n  {line}"),
                });
                d = d.fix(Fix::safe(vec![edit]));
            }
            Err(d)
        }
        Some((v, spot)) if v == "runspore/0.1" => {
            Err(Diagnostic::new("dot.old-dialect", spot.at())
                .arg("dialect", Arg::computed(v))
                .fix(Fix::review(vec![spot.replace(quoted_dialect)])))
        }
        Some((v, spot)) if v != DIALECT => Err(Diagnostic::new("dot.unknown-dialect", spot.at())
            .arg("dialect", Arg::computed(v))
            .fix(Fix::review(vec![spot.replace(quoted_dialect)]))),
        Some(_) => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Attr {
    name: String,
    value: String,
    spot: Spot,
    value_spot: Spot,
}

enum Stmt {
    GraphAttr(Attr),
    Defaults(&'static str, Vec<Attr>),
    Node(String, Spot, Vec<Attr>),
    /// The IDs of a chain with their spots, and the shared attributes.
    Edges(Vec<(String, Spot)>, Vec<Attr>),
}

struct Parsed {
    name: Option<(String, Spot)>,
    head: Spot,
    open: Spot,
    close: Spot,
    /// Each statement with the line it ends on.
    stmts: Vec<(Stmt, usize)>,
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    end: Spot,
    /// Attributes given again in the same `[...]` list, reported once the parse is done.
    repeated: Vec<Attr>,
}

fn syntax(found: Option<&Token>, end: Spot, expected: &str) -> Diagnostic {
    let (spot, found) = match found {
        Some(t) => (t.spot, Arg::Code(describe(&t.tok))),
        None => (end, Arg::Code("the end of the file".into())),
    };
    Diagnostic::new("dot.syntax", spot.at())
        .arg("found", found)
        .arg("expected", Arg::Code(expected.into()))
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at).map(|t| &t.tok)
    }

    fn spot(&self) -> Spot {
        self.tokens.get(self.at).map_or(self.end, |t| t.spot)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn unexpected(&self, wanted: &str) -> Diagnostic {
        syntax(self.tokens.get(self.at), self.end, wanted)
    }

    fn id(&mut self, wanted: &str) -> Result<(String, Spot), Diagnostic> {
        match self.peek() {
            Some(Tok::Id(..)) => match self.next() {
                Some(Token {
                    tok: Tok::Id(s, _),
                    spot,
                }) => Ok((s, spot)),
                _ => unreachable!("peeked an ID"),
            },
            _ => Err(self.unexpected(wanted)),
        }
    }

    fn graph(&mut self) -> Result<Parsed, Diagnostic> {
        let head = self.spot();
        match self.peek() {
            Some(t) if keyword(t, "strict") => return Err(unsupported(head, "strict graphs")),
            Some(t) if keyword(t, "graph") => {
                return Err(unsupported(head, "undirected graphs (graph)")
                    .fix(Fix::review(vec![head.replace("digraph")])))
            }
            Some(t) if keyword(t, "digraph") => {
                self.at += 1;
            }
            _ => return Err(self.unexpected("\"digraph\"")),
        }
        let name = match self.peek() {
            Some(Tok::Id(..)) => Some(self.id("the graph's name")?),
            _ => None,
        };
        if self.peek() != Some(&Tok::LBrace) {
            return Err(self.unexpected("\"{\""));
        }
        let open = self.spot();
        self.at += 1;
        let mut stmts = Vec::new();
        let close;
        loop {
            match self.peek() {
                None => {
                    let fix = Fix::review(vec![insert_lines(self.end.line + 1, "}\n".into())]);
                    return Err(unclosed(open, "the graph", "}").fix(fix));
                }
                Some(Tok::RBrace) => {
                    close = self.spot();
                    self.at += 1;
                    break;
                }
                Some(Tok::Semi) => self.at += 1,
                Some(_) => {
                    let stmt = self.stmt()?;
                    let mut last = self.tokens[self.at - 1].spot.line;
                    if self.peek() == Some(&Tok::Semi) {
                        last = self.spot().line;
                        self.at += 1;
                    }
                    stmts.push((stmt, last));
                }
            }
        }
        if self.peek().is_some() {
            let spot = self.spot();
            return Err(Diagnostic::new("dot.after-graph", spot.at()));
        }
        Ok(Parsed {
            name,
            head,
            open,
            close,
            stmts,
        })
    }

    fn stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let spot = self.spot();
        let tok = self
            .peek()
            .cloned()
            .ok_or_else(|| self.unexpected("a statement"))?;
        if tok == Tok::LBrace || keyword(&tok, "subgraph") {
            return Err(unsupported(spot, "subgraphs"));
        }
        for (word, what) in [("graph", "graph"), ("node", "node"), ("edge", "edge")] {
            if keyword(&tok, word) {
                self.at += 1;
                let attrs = self.attr_lists(true)?;
                return Ok(Stmt::Defaults(what, attrs));
            }
        }
        if keyword(&tok, "digraph") || keyword(&tok, "strict") {
            return Err(self.unexpected("a statement"));
        }
        let first = self.id("a statement")?;
        if self.peek() == Some(&Tok::Eq) {
            self.at += 1;
            let value_spot = self.spot();
            let (value, _) = self.id("a value")?;
            return Ok(Stmt::GraphAttr(Attr {
                name: first.0,
                value,
                spot: first.1,
                value_spot,
            }));
        }
        self.no_port()?;
        let mut ids = vec![first];
        loop {
            match self.peek() {
                Some(Tok::Arrow) => {
                    self.at += 1;
                    if matches!(self.peek(), Some(Tok::LBrace))
                        || self.peek().is_some_and(|t| keyword(t, "subgraph"))
                    {
                        return Err(unsupported(self.spot(), "subgraphs"));
                    }
                    ids.push(self.id("a node ID")?);
                    self.no_port()?;
                }
                Some(Tok::Undirected) => {
                    let spot = self.spot();
                    return Err(unsupported(spot, "undirected arrows (--)")
                        .fix(Fix::review(vec![spot.replace("->")])));
                }
                _ => break,
            }
        }
        let attrs = if self.peek() == Some(&Tok::LBracket) {
            self.attr_lists(false)?
        } else {
            Vec::new()
        };
        if ids.len() == 1 {
            let (id, spot) = ids.remove(0);
            Ok(Stmt::Node(id, spot, attrs))
        } else {
            Ok(Stmt::Edges(ids, attrs))
        }
    }

    fn no_port(&self) -> Result<(), Diagnostic> {
        if self.peek() == Some(&Tok::Colon) {
            return Err(unsupported(self.spot(), "ports (node:port)"));
        }
        Ok(())
    }

    /// One or more `[a=b, …]` lists. `required` when the statement needs one.
    fn attr_lists(&mut self, required: bool) -> Result<Vec<Attr>, Diagnostic> {
        if required && self.peek() != Some(&Tok::LBracket) {
            return Err(self.unexpected("\"[\""));
        }
        let mut attrs: Vec<Attr> = Vec::new();
        while self.peek() == Some(&Tok::LBracket) {
            self.at += 1;
            let list = attrs.len();
            let mut first = true;
            loop {
                match self.peek() {
                    Some(Tok::RBracket) => {
                        self.at += 1;
                        break;
                    }
                    Some(Tok::Comma | Tok::Semi) if !first => self.at += 1,
                    _ => {
                        let (name, spot) = self.id("an attribute name or \"]\"")?;
                        if self.peek() != Some(&Tok::Eq) {
                            return Err(Diagnostic::new("dot.no-value", spot.at())
                                .arg("attribute", Arg::computed(name.clone()))
                                .arg("example", Arg::Code(format!("{name}=\"…\""))));
                        }
                        self.at += 1;
                        let value_spot = self.spot();
                        let (value, _) = self.id("a value")?;
                        let attr = Attr {
                            name,
                            value,
                            spot,
                            value_spot,
                        };
                        if attrs[list..].iter().any(|a| a.name == attr.name) {
                            self.repeated.push(attr);
                        } else {
                            attrs.push(attr);
                        }
                        first = false;
                    }
                }
            }
        }
        Ok(attrs)
    }
}

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: &'static str,
    /// The node's first mention.
    pub spot: Spot,
    pub mentions: Vec<Spot>,
}

impl Node {
    /// The node's first mention, as a diagnostic's place.
    pub fn spot_at(&self) -> At {
        self.spot.at()
    }
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub on: String,
    /// From the source's ID to the end of the target's, when on one line.
    pub spot: Spot,
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub name: String,
    pub start: Option<(String, Spot)>,
    pub nodes: BTreeMap<String, Node>,
    /// Node IDs in the order of their first mention.
    pub order: Vec<String>,
    pub edges: Vec<Edge>,
    /// The line of the last statement, and of the last arrow statement.
    pub last_line: usize,
    pub last_edge_line: Option<usize>,
    /// The graph's closing brace.
    pub close: Spot,
    /// Steps whose kind or arrows have a problem already reported: the checks that
    /// depend on them skip them.
    pub unsure: BTreeSet<String>,
}

/// What reading `workflow.dot` gives.
pub enum Read {
    /// The dialect is missing or unknown: nothing else can be read.
    Stop,
    /// Outside the grammar; the problem is in `found`.
    Unreadable,
    Graph(Box<Graph>),
}

/// A node ID as written in DOT: bare when it can be, else quoted.
pub fn id_text(id: &str) -> String {
    let bare = id.chars().next().is_some_and(is_id_start) && id.chars().all(is_id_char);
    if bare
        && !["graph", "digraph", "node", "edge", "strict", "subgraph"]
            .iter()
            .any(|k| id.eq_ignore_ascii_case(k))
    {
        id.to_string()
    } else {
        Value::String(id.to_string()).to_string()
    }
}

/// The line drawing an arrow, as the build suggests it.
pub fn arrow_text(from: &str, to: &str, on: &str) -> String {
    let attrs = if on == runspore_types::model::OUTCOME_OK {
        String::new()
    } else {
        format!(" [on={}]", Value::String(on.to_string()))
    };
    format!("{} -> {}{attrs};", id_text(from), id_text(to))
}

/// The indentation of `line` in `text`.
pub fn indent(text: &str, line: usize) -> String {
    text.lines()
        .nth(line.saturating_sub(1))
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect()
}

impl Graph {
    /// An edit adding `lines` (statements) after the last arrow, or after the last
    /// statement when there is none.
    pub fn add_lines(&self, text: &str, lines: &[String]) -> Edit {
        let after = self.last_edge_line.unwrap_or(self.last_line);
        let indent = match indent(text, after) {
            s if s.is_empty() => "  ".to_string(),
            s => s,
        };
        let added: String = lines.iter().map(|l| format!("{indent}{l}\n")).collect();
        insert_after(after, self.close, added)
    }

    /// The steps an arrow out of `from` can lead to: the ends, `complete` ones first,
    /// each in the order written; with no end, a new `complete` step and the line that
    /// declares it.
    pub fn arrow_targets(&self, from: &str) -> Vec<(String, Option<String>)> {
        let of = |kind: &str| {
            self.order
                .iter()
                .filter(|id| *id != from && self.nodes[*id].kind == kind)
                .map(|id| (id.clone(), None))
                .collect::<Vec<_>>()
        };
        let mut ends = of("complete");
        ends.extend(of("fail"));
        if ends.is_empty() {
            let mut name = "done".to_string();
            let mut n = 2;
            while self.nodes.contains_key(&name) {
                name = format!("done-{n}");
                n += 1;
            }
            let declare = format!("{} [kind=\"complete\"];", id_text(&name));
            ends.push((name, Some(declare)));
        }
        ends
    }
}

/// A name sanitized to `allowed` characters, for a rename fix.
fn sanitized(s: &str, allowed: impl Fn(char) -> bool, max: usize) -> String {
    let out: String = s
        .chars()
        .map(|c| if allowed(c) { c } else { '-' })
        .take(max)
        .collect();
    if out.is_empty() {
        "workflow".to_string()
    } else {
        out
    }
}

/// Parses `workflow.dot` and checks spec/dot.md §2, §3 and step 2 of §5. `folder` is
/// the folder's name, offered as the graph's name when it has none.
pub fn read(text: &str, folder: &str, found: &mut Found) -> Read {
    let tokens = match lex(text) {
        Ok(tokens) => tokens,
        Err(d) => {
            found.push(d);
            return Read::Unreadable;
        }
    };
    if let Err(d) = dialect(&tokens) {
        found.push(d);
        return Read::Stop;
    }
    let end = tokens.last().map_or(Spot::default(), |t| t.spot);
    let mut parser = Parser {
        tokens,
        at: 0,
        end: Spot { len: 1, ..end },
        repeated: Vec::new(),
    };
    let parsed = match parser.graph() {
        Ok(parsed) => parsed,
        Err(d) => {
            found.push(d);
            return Read::Unreadable;
        }
    };
    let mut checker = Checker {
        text,
        found,
        nodes: BTreeMap::new(),
        order: Vec::new(),
        kinds: BTreeMap::new(),
        start: None,
        dialect: None,
        unsure: BTreeSet::new(),
    };
    let name = checker.name(parsed.name, parsed.head, parsed.open, folder);
    for a in &parser.repeated {
        checker.repeated(a);
    }
    let mut edges = Vec::new();
    let mut last_line = parsed.open.line;
    let mut last_edge_line = None;
    for (stmt, line) in parsed.stmts {
        last_line = line;
        match stmt {
            Stmt::GraphAttr(a) => checker.graph_attr(a),
            Stmt::Defaults("graph", attrs) => attrs.into_iter().for_each(|a| checker.graph_attr(a)),
            Stmt::Defaults(what, attrs) => {
                for a in attrs {
                    if !RENDER.contains(&a.name.as_str()) {
                        checker.not_render(&a, &format!("{what} [...]"));
                    }
                }
            }
            Stmt::Node(id, spot, attrs) => {
                checker.mention(&id, spot);
                for a in attrs {
                    match a.name.as_str() {
                        "kind" => checker.kind(&id, &a),
                        _ => checker.not_render(&a, "a step"),
                    }
                }
            }
            Stmt::Edges(ids, attrs) => {
                for (id, spot) in &ids {
                    checker.mention(id, *spot);
                }
                let mut on = None;
                for a in attrs {
                    match a.name.as_str() {
                        "on" => {
                            if a.value != "failed" && !is_outcome(&a.value) {
                                if let Some((from, _)) = ids.first() {
                                    checker.unsure.insert(from.clone());
                                }
                                checker.found.push(
                                    Diagnostic::new("dot.bad-outcome", a.value_spot.at())
                                        .arg("outcome", Arg::computed(a.value.clone())),
                                );
                            }
                            on = Some(a.value);
                        }
                        _ => checker.not_render(&a, "an arrow"),
                    }
                }
                let on = on.unwrap_or_else(|| runspore_types::model::OUTCOME_OK.to_string());
                for pair in ids.windows(2) {
                    let (from, to) = (&pair[0], &pair[1]);
                    let spot = if from.1.line == to.1.line {
                        Spot {
                            len: to.1.col + to.1.len - from.1.col,
                            ..from.1
                        }
                    } else {
                        from.1
                    };
                    edges.push(Edge {
                        from: from.0.clone(),
                        to: to.0.clone(),
                        on: on.clone(),
                        spot,
                    });
                }
                last_edge_line = Some(line);
            }
        }
    }
    let mut nodes = std::mem::take(&mut checker.nodes);
    for (id, (kind, _)) in &checker.kinds {
        if let Some(node) = nodes.get_mut(id) {
            node.kind = kind;
        }
    }
    let graph = Graph {
        name,
        start: checker.start.take(),
        nodes,
        order: std::mem::take(&mut checker.order),
        edges,
        last_line,
        last_edge_line,
        close: parsed.close,
        unsure: std::mem::take(&mut checker.unsure),
    };
    checker.start(&graph, parsed.head);
    checker.edges(&graph);
    Read::Graph(Box::new(graph))
}

struct Checker<'a> {
    text: &'a str,
    found: &'a mut Found,
    nodes: BTreeMap<String, Node>,
    order: Vec<String>,
    kinds: BTreeMap<String, (&'static str, usize)>,
    start: Option<(String, Spot)>,
    dialect: Option<Attr>,
    unsure: BTreeSet<String>,
}

impl Checker<'_> {
    fn name(
        &mut self,
        name: Option<(String, Spot)>,
        head: Spot,
        open: Spot,
        folder: &str,
    ) -> String {
        let named = sanitized(
            folder,
            |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'),
            128,
        );
        let Some((name, spot)) = name else {
            let quoted = Value::String(named.clone()).to_string();
            let edit = Edit::Dot(TextEdit {
                line: open.line,
                col: open.col,
                end_line: open.line,
                end_col: open.col,
                text: format!("{quoted} "),
            });
            let line = format!("digraph {quoted} {{");
            self.found.push(
                Diagnostic::new("dot.no-name", At::dot_line(head.line))
                    .arg("line", Arg::Code(line))
                    .fix(Fix::review(vec![edit])),
            );
            return named;
        };
        if !is_name(&name) {
            let fixed = sanitized(
                &name,
                |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'),
                128,
            );
            let quoted = Value::String(fixed.clone()).to_string();
            self.found.push(
                Diagnostic::new("dot.bad-name", spot.at())
                    .arg("name", Arg::computed(name.clone()))
                    .fix(
                        Fix::review(vec![spot.replace(quoted)]).arg("fixed", Arg::computed(fixed)),
                    ),
            );
        }
        name
    }

    fn mention(&mut self, id: &str, spot: Spot) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.mentions.push(spot);
            return;
        }
        if self.nodes.len() == MAX_NODES {
            if !self
                .found
                .list
                .iter()
                .any(|d| d.code == "dot.too-many-steps")
            {
                self.found
                    .push(Diagnostic::new("dot.too-many-steps", spot.at().of_step(id)));
            }
            return;
        }
        self.order.push(id.to_string());
        self.nodes.insert(
            id.to_string(),
            Node {
                kind: "activity",
                spot,
                mentions: vec![spot],
            },
        );
    }

    fn graph_attr(&mut self, a: Attr) {
        match a.name.as_str() {
            "dialect" => {
                if self.dialect.is_some() {
                    self.repeated(&a);
                } else {
                    self.dialect = Some(a);
                }
            }
            "start" => {
                if self.start.is_some() {
                    self.repeated(&a);
                } else {
                    self.start = Some((a.value.clone(), a.value_spot));
                }
            }
            _ => self.not_render(&a, "the graph"),
        }
    }

    fn repeated(&mut self, a: &Attr) {
        self.found.push(
            Diagnostic::new("dot.repeated-attribute", a.spot.at())
                .arg("attribute", Arg::computed(a.name.clone())),
        );
    }

    /// Accepts a render attribute; anything else is misplaced or unknown.
    fn not_render(&mut self, a: &Attr, place: &str) {
        let name = a.name.as_str();
        if RENDER.contains(&name) {
            return;
        }
        let home = match name {
            "dialect" | "start" => Some("the graph"),
            "kind" => Some("a step"),
            "on" => Some("an arrow"),
            _ => None,
        };
        let attribute = Arg::computed(name.to_string());
        let d = match home {
            Some(home) => Diagnostic::new("dot.misplaced-attribute", a.spot.at())
                .arg("attribute", attribute)
                .arg("home", Arg::Code(home.into()))
                .arg("place", Arg::Code(place.into())),
            None => {
                let known = RENDER
                    .iter()
                    .copied()
                    .chain(["kind", "on", "start", "dialect"]);
                match closest(name, known) {
                    Some(near) => Diagnostic::new("dot.attribute-typo", a.spot.at())
                        .arg("attribute", attribute)
                        .fix(
                            Fix::review(vec![a.spot.replace(near)])
                                .arg("closest", Arg::Code(near.into())),
                        ),
                    None => Diagnostic::new("dot.unknown-attribute", a.spot.at())
                        .arg("attribute", attribute),
                }
            }
        };
        self.found.push(d);
    }

    fn kind(&mut self, id: &str, a: &Attr) {
        let Some(kind) = KINDS.iter().find(|k| **k == a.value) else {
            self.unsure.insert(id.to_string());
            let near = closest(&a.value, KINDS);
            let mut ordered: Vec<&str> = near.into_iter().collect();
            ordered.extend(KINDS.iter().filter(|k| Some(**k) != near));
            let fixes = ordered.iter().map(|k| {
                Fix::choice(vec![a
                    .value_spot
                    .replace(Value::String(k.to_string()).to_string())])
            });
            let mut d = Diagnostic::new("dot.unknown-kind", a.value_spot.at().of_step(id))
                .arg("kind", Arg::computed(a.value.clone()))
                .fixes(fixes);
            if let Some(first) = d.fixes.first_mut() {
                first.args.push(("choices", Arg::computed(KINDS.to_vec())));
            }
            self.found.push(d);
            return;
        };
        if let Some((prev, prev_line)) = self.kinds.get(id) {
            if prev != kind {
                self.unsure.insert(id.to_string());
                self.found.push(
                    Diagnostic::new("dot.kind-conflict", a.value_spot.at().of_step(id))
                        .arg("kind", Arg::computed(kind.to_string()))
                        .arg("other", Arg::computed(prev.to_string()))
                        .arg("line", Arg::computed(*prev_line)),
                );
                return;
            }
        }
        self.kinds.insert(id.to_string(), (kind, a.spot.line));
    }

    /// The start: present, and a node of the graph; node IDs are well formed.
    fn start(&mut self, graph: &Graph, head: Spot) {
        for id in &graph.order {
            let node = &graph.nodes[id];
            if !is_node_id(id) {
                let fixed = sanitized(
                    id,
                    |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'),
                    64,
                );
                let edits = node
                    .mentions
                    .iter()
                    .map(|s| s.replace(id_text(&fixed)))
                    .collect();
                self.found.push(
                    Diagnostic::new("dot.bad-step-name", node.spot.at().of_step(id))
                        .fix(Fix::review(edits).arg("fixed", Arg::computed(fixed))),
                );
            }
        }
        if graph.nodes.is_empty() {
            self.found
                .push(Diagnostic::new("dot.no-steps", At::dot_line(head.line)));
            return;
        }
        match &graph.start {
            None => {
                let guess = graph
                    .order
                    .iter()
                    .find(|id| {
                        matches!(graph.nodes[*id].kind, "activity" | "await-signal")
                            && !graph.edges.iter().any(|e| &e.to == *id)
                    })
                    .or_else(|| graph.order.first());
                let mut d = Diagnostic::new("dot.no-start", At::dot_line(head.line));
                if let (Some(guess), Some(dialect)) = (guess, &self.dialect) {
                    d = d.fix(
                        Fix::review(vec![self.start_line(dialect, guess, graph.close)])
                            .arg("start", Arg::computed(guess.clone())),
                    );
                }
                self.found.push(d);
            }
            Some((start, spot)) if !graph.nodes.contains_key(start) => {
                let near = closest(start, graph.order.iter().map(String::as_str));
                let mut choices: Vec<&str> = near.into_iter().collect();
                choices.extend(
                    graph
                        .order
                        .iter()
                        .map(String::as_str)
                        .filter(|id| Some(*id) != near)
                        .take(4),
                );
                let fixes: Vec<Fix> = choices
                    .iter()
                    .map(|id| {
                        let applicability = if Some(*id) == near {
                            Applicability::NeedsReview
                        } else {
                            Applicability::NeedsChoice
                        };
                        Fix::new(
                            applicability,
                            vec![spot.replace(Value::String(id.to_string()).to_string())],
                        )
                        .arg("closest", Arg::computed(id.to_string()))
                    })
                    .collect();
                self.found.push(
                    Diagnostic::new("dot.unknown-start", spot.at())
                        .arg("start", Arg::computed(start.clone()))
                        .fixes(fixes),
                );
            }
            Some(_) => {}
        }
    }

    /// The line `start = "<id>";`, lined up with the dialect line, inserted after it.
    fn start_line(&self, dialect: &Attr, id: &str, close: Spot) -> Edit {
        let line = self.text.lines().nth(dialect.spot.line - 1).unwrap_or("");
        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let rest = line.trim_start();
        let gap = rest
            .strip_prefix("dialect")
            .map(|r| r.len() - r.trim_start().len())
            .filter(|_| rest.starts_with("dialect"))
            .unwrap_or(1);
        let pad = " ".repeat("dialect".len() + gap - "start".len());
        let indent = if indent.is_empty() {
            "  ".to_string()
        } else {
            indent
        };
        insert_after(
            dialect.spot.line,
            close,
            format!("{indent}start{pad}= {};\n", Value::String(id.to_string())),
        )
    }

    /// No arrow leaves an end; no two arrows from one node share an outcome.
    fn edges(&mut self, graph: &Graph) {
        let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for e in &graph.edges {
            let Some(from) = graph.nodes.get(&e.from) else {
                continue;
            };
            if matches!(from.kind, "complete" | "fail") {
                self.found.push(
                    Diagnostic::new("dot.arrow-from-end", e.spot.at().of_step(&e.from))
                        .arg("kind", Arg::computed(from.kind.to_string()))
                        .arg("arrow", Arg::Code(arrow_text(&e.from, &e.to, &e.on))),
                );
                continue;
            }
            if seen.insert((&e.from, &e.on), e.spot.line).is_some() {
                self.found.push(
                    Diagnostic::new("dot.repeated-arrow", e.spot.at().of_step(&e.from))
                        .arg("outcome", Arg::computed(e.on.clone())),
                );
            }
        }
    }
}
