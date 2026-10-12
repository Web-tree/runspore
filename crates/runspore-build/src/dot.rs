//! The DOT dialect of spec/dot.md §2–§3: a lexer, a parser for the accepted grammar,
//! and the checks that turn the parse into a [`Graph`].

use std::collections::BTreeMap;

use crate::{is_name, is_node_id, is_outcome, DIALECT};

/// A DOT error: code, line, message. The caller adds the file.
pub type Error = (&'static str, usize, String);

fn syntax(line: usize, message: impl Into<String>) -> Error {
    ("dot.syntax", line, message.into())
}

fn attribute(line: usize, message: impl Into<String>) -> Error {
    ("dot.attribute", line, message.into())
}

fn graph_error(line: usize, message: impl Into<String>) -> Error {
    ("dot.graph", line, message.into())
}

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
    line: usize,
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Id(s, true) => format!("\"{s}\""),
        Tok::Id(s, false) => format!("`{s}`"),
        Tok::LBrace => "`{`".into(),
        Tok::RBrace => "`}`".into(),
        Tok::LBracket => "`[`".into(),
        Tok::RBracket => "`]`".into(),
        Tok::Eq => "`=`".into(),
        Tok::Semi => "`;`".into(),
        Tok::Comma => "`,`".into(),
        Tok::Colon => "`:`".into(),
        Tok::Arrow => "`->`".into(),
        Tok::Undirected => "`--`".into(),
    }
}

fn is_id_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_id_char(c: char) -> bool {
    is_id_start(c) || c.is_ascii_digit()
}

fn lex(text: &str) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut at_line_start = true;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            at_line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start_line = line;
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
            tokens.push(Token { tok, line });
            i += 1;
            continue;
        }
        match c {
            '#' if line_start => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                loop {
                    match chars.get(i) {
                        None => return Err(syntax(start_line, "a `/*` comment is never closed")),
                        Some('*') if chars.get(i + 1) == Some(&'/') => {
                            i += 2;
                            break;
                        }
                        Some('\n') => line += 1,
                        Some(_) => {}
                    }
                    i += 1;
                }
            }
            '-' if next == Some('>') => {
                tokens.push(Token {
                    tok: Tok::Arrow,
                    line,
                });
                i += 2;
            }
            '-' if next == Some('-') => {
                tokens.push(Token {
                    tok: Tok::Undirected,
                    line,
                });
                i += 2;
            }
            '<' => return Err(syntax(line, "HTML strings (`<…>`) are not read")),
            '"' => {
                let mut value = String::new();
                loop {
                    i = quoted(&chars, i, &mut line, &mut value)
                        .ok_or_else(|| syntax(start_line, "a quoted string is never closed"))?;
                    // `"a" + "b"` concatenates.
                    let mut j = i;
                    let mut lines = 0;
                    while j < chars.len() && chars[j].is_whitespace() {
                        lines += usize::from(chars[j] == '\n');
                        j += 1;
                    }
                    if chars.get(j) != Some(&'+') {
                        break;
                    }
                    j += 1;
                    while j < chars.len() && chars[j].is_whitespace() {
                        lines += usize::from(chars[j] == '\n');
                        j += 1;
                    }
                    if chars.get(j) != Some(&'"') {
                        return Err(syntax(
                            line + lines,
                            "`+` must be followed by a quoted string",
                        ));
                    }
                    line += lines;
                    i = j;
                }
                tokens.push(Token {
                    tok: Tok::Id(value, true),
                    line: start_line,
                });
            }
            c if c.is_ascii_digit() || c == '.' || c == '-' => {
                let begin = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let numeral: String = chars[begin..i].iter().collect();
                let digits = numeral.chars().filter(char::is_ascii_digit).count();
                let dots = numeral.matches('.').count();
                if digits == 0 || dots > 1 {
                    return Err(syntax(line, format!("`{numeral}` is not an ID; quote it")));
                }
                if chars.get(i).is_some_and(|&c| is_id_char(c) || c == '-') {
                    return Err(syntax(
                        line,
                        format!("a bare ID cannot start with `{numeral}`; quote it"),
                    ));
                }
                tokens.push(Token {
                    tok: Tok::Id(numeral, false),
                    line,
                });
            }
            c if is_id_start(c) => {
                let begin = i;
                while i < chars.len() && is_id_char(chars[i]) {
                    i += 1;
                }
                tokens.push(Token {
                    tok: Tok::Id(chars[begin..i].iter().collect(), false),
                    line,
                });
            }
            c => return Err(syntax(line, format!("unexpected character `{c}`"))),
        }
    }
    Ok(tokens)
}

/// Reads the quoted string opening at `chars[i]`; returns the index after its closing
/// quote. Only `\"` is an escape and `\` before a newline continues the line, as in
/// Graphviz; any other backslash is kept.
fn quoted(chars: &[char], mut i: usize, line: &mut usize, out: &mut String) -> Option<usize> {
    i += 1;
    loop {
        match *chars.get(i)? {
            '"' => return Some(i + 1),
            '\\' if chars.get(i + 1) == Some(&'"') => {
                out.push('"');
                i += 2;
            }
            '\\' if chars.get(i + 1) == Some(&'\n') => {
                *line += 1;
                i += 2;
            }
            c => {
                *line += usize::from(c == '\n');
                out.push(c);
                i += 1;
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
/// unknown dialect is the first error (spec/dot.md §3). Returns its line.
fn dialect(tokens: &[Token]) -> Result<usize, Error> {
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
                if let (
                    Some(Tok::Eq),
                    Some(Token {
                        tok: Tok::Id(v, _), ..
                    }),
                ) = (tokens.get(i + 1).map(|t| &t.tok), tokens.get(i + 2))
                {
                    found = Some((v.clone(), t.line));
                    break;
                }
            }
            _ => {}
        }
    }
    let head = tokens.first().map_or(1, |t| t.line);
    match found {
        None => Err((
            "dot.dialect",
            head,
            format!("the graph has no `dialect`; add `dialect = \"{DIALECT}\";` inside it"),
        )),
        Some((v, line)) if v == "runspore/0.1" => Err((
            "dot.dialect",
            line,
            format!(
                "dialect `runspore/0.1` is not read by this spore: change the dialect line to \
                 `{DIALECT}`, and `format` in settings.json to `runspore.workflow/0.2`"
            ),
        )),
        Some((v, line)) if v != DIALECT => Err((
            "dot.dialect",
            line,
            format!("unknown dialect `{v}`; this spore reads `{DIALECT}`"),
        )),
        Some((_, line)) => Ok(line),
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Attr {
    name: String,
    value: String,
    line: usize,
}

enum Stmt {
    GraphAttr(Attr),
    Defaults(&'static str, Vec<Attr>),
    Node(String, usize, Vec<Attr>),
    /// The IDs of a chain with the line of each `->`, and the shared attributes.
    Edges(Vec<(String, usize)>, Vec<usize>, Vec<Attr>),
}

type Parsed = (Option<(String, usize)>, usize, Vec<Stmt>);

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    last_line: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at).map(|t| &t.tok)
    }

    fn line(&self) -> usize {
        self.tokens.get(self.at).map_or(self.last_line, |t| t.line)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn unexpected(&self, wanted: &str) -> Error {
        match self.peek() {
            Some(tok) => syntax(
                self.line(),
                format!("expected {wanted}, found {}", describe(tok)),
            ),
            None => syntax(self.last_line, format!("expected {wanted}, found the end")),
        }
    }

    fn id(&mut self, wanted: &str) -> Result<(String, usize), Error> {
        match self.peek() {
            Some(Tok::Id(..)) => match self.next() {
                Some(Token {
                    tok: Tok::Id(s, _),
                    line,
                }) => Ok((s, line)),
                _ => unreachable!("peeked an ID"),
            },
            _ => Err(self.unexpected(wanted)),
        }
    }

    /// The graph id with its line, the line of `digraph`, and the statements.
    fn graph(&mut self) -> Result<Parsed, Error> {
        let head = self.line();
        match self.peek() {
            Some(t) if keyword(t, "strict") => {
                return Err(syntax(head, "`strict` graphs are not read"))
            }
            Some(t) if keyword(t, "graph") => {
                return Err(syntax(
                    head,
                    "undirected graphs are not read; use `digraph`",
                ))
            }
            Some(t) if keyword(t, "digraph") => {
                self.at += 1;
            }
            _ => return Err(self.unexpected("`digraph`")),
        }
        let name = match self.peek() {
            Some(Tok::Id(..)) => Some(self.id("the graph id")?),
            _ => None,
        };
        if self.peek() != Some(&Tok::LBrace) {
            return Err(self.unexpected("`{`"));
        }
        self.at += 1;
        let mut stmts = Vec::new();
        loop {
            match self.peek() {
                None => return Err(syntax(self.last_line, "the graph is never closed with `}`")),
                Some(Tok::RBrace) => {
                    self.at += 1;
                    break;
                }
                Some(Tok::Semi) => self.at += 1,
                Some(_) => {
                    stmts.push(self.stmt()?);
                    if self.peek() == Some(&Tok::Semi) {
                        self.at += 1;
                    }
                }
            }
        }
        if self.peek().is_some() {
            return Err(syntax(
                self.line(),
                "nothing may follow the graph's closing `}`",
            ));
        }
        Ok((name, head, stmts))
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        let line = self.line();
        let tok = self
            .peek()
            .cloned()
            .ok_or_else(|| self.unexpected("a statement"))?;
        if tok == Tok::LBrace || keyword(&tok, "subgraph") {
            return Err(syntax(line, "subgraphs are not read"));
        }
        for (word, what) in [
            ("graph", None),
            ("node", Some("node")),
            ("edge", Some("edge")),
        ] {
            if keyword(&tok, word) {
                self.at += 1;
                let attrs = self.attr_lists(true)?;
                return Ok(match what {
                    None => Stmt::Defaults("graph", attrs),
                    Some(w) => Stmt::Defaults(w, attrs),
                });
            }
        }
        if keyword(&tok, "digraph") || keyword(&tok, "strict") {
            return Err(syntax(line, format!("unexpected {}", describe(&tok))));
        }
        let first = self.id("a statement")?;
        if self.peek() == Some(&Tok::Eq) {
            self.at += 1;
            let (value, _) = self.id("a value")?;
            return Ok(Stmt::GraphAttr(Attr {
                name: first.0,
                value,
                line,
            }));
        }
        self.no_port()?;
        let mut ids = vec![first];
        let mut arrows = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Arrow) => {
                    arrows.push(self.line());
                    self.at += 1;
                    if matches!(self.peek(), Some(Tok::LBrace))
                        || self.peek().is_some_and(|t| keyword(t, "subgraph"))
                    {
                        return Err(syntax(self.line(), "subgraphs are not read"));
                    }
                    ids.push(self.id("a node ID")?);
                    self.no_port()?;
                }
                Some(Tok::Undirected) => {
                    return Err(syntax(
                        self.line(),
                        "undirected edges (`--`) are not read; use `->`",
                    ))
                }
                _ => break,
            }
        }
        let attrs = if self.peek() == Some(&Tok::LBracket) {
            self.attr_lists(false)?
        } else {
            Vec::new()
        };
        if arrows.is_empty() {
            let (id, line) = ids.remove(0);
            Ok(Stmt::Node(id, line, attrs))
        } else {
            Ok(Stmt::Edges(ids, arrows, attrs))
        }
    }

    fn no_port(&self) -> Result<(), Error> {
        if self.peek() == Some(&Tok::Colon) {
            return Err(syntax(self.line(), "ports (`node:port`) are not read"));
        }
        Ok(())
    }

    /// One or more `[a=b, …]` lists. `required` when the statement needs one.
    fn attr_lists(&mut self, required: bool) -> Result<Vec<Attr>, Error> {
        if required && self.peek() != Some(&Tok::LBracket) {
            return Err(self.unexpected("`[`"));
        }
        let mut attrs: Vec<Attr> = Vec::new();
        while self.peek() == Some(&Tok::LBracket) {
            self.at += 1;
            let mut list: Vec<Attr> = Vec::new();
            loop {
                match self.peek() {
                    Some(Tok::RBracket) => {
                        self.at += 1;
                        break;
                    }
                    Some(Tok::Comma | Tok::Semi) if !list.is_empty() => self.at += 1,
                    _ => {
                        let (name, line) = self.id("an attribute name or `]`")?;
                        if self.peek() != Some(&Tok::Eq) {
                            return Err(syntax(line, format!("attribute `{name}` has no value")));
                        }
                        self.at += 1;
                        let (value, _) = self.id("a value")?;
                        if list.iter().any(|a| a.name == name) {
                            return Err(attribute(
                                line,
                                format!("attribute `{name}` is given twice in one list"),
                            ));
                        }
                        list.push(Attr { name, value, line });
                    }
                }
            }
            attrs.extend(list);
        }
        Ok(attrs)
    }
}

// ---------------------------------------------------------------------------
// The graph
// ---------------------------------------------------------------------------

pub struct Node {
    pub kind: &'static str,
    /// The line of the node's first mention.
    pub line: usize,
}

pub struct Edge {
    pub from: String,
    pub to: String,
    pub on: String,
    pub line: usize,
}

pub struct Graph {
    pub name: String,
    pub start: String,
    pub dialect_line: usize,
    pub nodes: BTreeMap<String, Node>,
    pub edges: Vec<Edge>,
}

const MAX_NODES: usize = runspore_types::model::MAX_NODES;

/// Parses `workflow.dot` and checks spec/dot.md §2, §3 and step 2 of §5.
pub fn read(text: &str) -> Result<Graph, Error> {
    let tokens = lex(text)?;
    let dialect_line = dialect(&tokens)?;
    let last_line = tokens.last().map_or(1, |t| t.line);
    let mut parser = Parser {
        tokens,
        at: 0,
        last_line,
    };
    let (name, head, stmts) = parser.graph()?;

    let (name, name_line) =
        name.ok_or_else(|| graph_error(head, "the graph has no id; its id is the workflow name"))?;
    if !is_name(&name) {
        return Err(graph_error(
            name_line,
            format!("graph id `{name}` must match [A-Za-z0-9_.-]{{1,128}}"),
        ));
    }

    let mut start: Option<(String, usize)> = None;
    let mut dialects = 0;
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut kinds: BTreeMap<String, (&'static str, usize)> = BTreeMap::new();
    let mut edges = Vec::new();

    fn mention(nodes: &mut BTreeMap<String, Node>, id: &str, line: usize) -> Result<(), Error> {
        if nodes.contains_key(id) {
            return Ok(());
        }
        if !is_node_id(id) {
            return Err(graph_error(
                line,
                format!("node ID `{id}` must match [A-Za-z0-9_-]{{1,64}}"),
            ));
        }
        if nodes.len() == MAX_NODES {
            return Err(graph_error(
                line,
                format!("node `{id}` is one more than the {MAX_NODES} a graph may have"),
            ));
        }
        nodes.insert(
            id.to_string(),
            Node {
                kind: "activity",
                line,
            },
        );
        Ok(())
    }

    for stmt in stmts {
        match stmt {
            Stmt::GraphAttr(a) => graph_attr(a, &mut start, &mut dialects)?,
            Stmt::Defaults("graph", attrs) => {
                for a in attrs {
                    graph_attr(a, &mut start, &mut dialects)?;
                }
            }
            Stmt::Defaults(what, attrs) => {
                for a in attrs {
                    if !RENDER.contains(&a.name.as_str()) {
                        return Err(attribute(
                            a.line,
                            format!(
                                "`{}` is not allowed in `{what} [...]`, which takes render \
                                 attributes only",
                                a.name
                            ),
                        ));
                    }
                }
            }
            Stmt::Node(id, line, attrs) => {
                mention(&mut nodes, &id, line)?;
                for a in attrs {
                    match a.name.as_str() {
                        "kind" => {
                            let Some(kind) = KINDS.iter().find(|k| **k == a.value) else {
                                return Err(attribute(
                                    a.line,
                                    format!(
                                        "`kind` of `{id}` is `{}`; it must be one of {}",
                                        a.value,
                                        KINDS.join(", ")
                                    ),
                                ));
                            };
                            if let Some((prev, prev_line)) = kinds.get(&id) {
                                if prev != kind {
                                    return Err(attribute(
                                        a.line,
                                        format!(
                                            "node `{id}` is `{kind}` here and `{prev}` on line \
                                             {prev_line}"
                                        ),
                                    ));
                                }
                            }
                            kinds.insert(id.clone(), (kind, a.line));
                        }
                        name => not_meaningful(name, a.line, "a node")?,
                    }
                }
            }
            Stmt::Edges(ids, arrows, attrs) => {
                for (id, line) in &ids {
                    mention(&mut nodes, id, *line)?;
                }
                let mut on = None;
                for a in attrs {
                    match a.name.as_str() {
                        "on" => {
                            if a.value != "failed" && !is_outcome(&a.value) {
                                return Err(attribute(
                                    a.line,
                                    format!(
                                        "`on=\"{}\"` must be one outcome matching \
                                         [a-z0-9][a-z0-9-]{{0,63}}; draw one edge per outcome",
                                        a.value
                                    ),
                                ));
                            }
                            on = Some(a.value);
                        }
                        name => not_meaningful(name, a.line, "an edge")?,
                    }
                }
                let on = on.unwrap_or_else(|| "ok".to_string());
                for (pair, line) in ids.windows(2).zip(arrows) {
                    edges.push(Edge {
                        from: pair[0].0.clone(),
                        to: pair[1].0.clone(),
                        on: on.clone(),
                        line,
                    });
                }
            }
        }
    }

    for (id, (kind, _)) in kinds {
        if let Some(node) = nodes.get_mut(&id) {
            node.kind = kind;
        }
    }
    let (start, start_line) = start.ok_or_else(|| {
        graph_error(
            head,
            "the graph has no `start`; add `start = \"<node>\";` inside it",
        )
    })?;
    if !nodes.contains_key(&start) {
        return Err(graph_error(
            start_line,
            format!("`start` names `{start}`, which is not a node of the graph"),
        ));
    }
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for e in &edges {
        let kind = nodes[&e.from].kind;
        if matches!(kind, "complete" | "fail") {
            return Err(graph_error(
                e.line,
                format!(
                    "`{}` is a `{kind}` node; no edge may leave it (`{} -> {}`)",
                    e.from, e.from, e.to
                ),
            ));
        }
        if let Some(first) = seen.insert((&e.from, &e.on), e.line) {
            return Err(graph_error(
                e.line,
                format!(
                    "`{}` already has an edge for `{}` on line {first}; one outcome routes to one node",
                    e.from, e.on
                ),
            ));
        }
    }
    Ok(Graph {
        name,
        start,
        dialect_line,
        nodes,
        edges,
    })
}

fn graph_attr(
    a: Attr,
    start: &mut Option<(String, usize)>,
    dialects: &mut usize,
) -> Result<(), Error> {
    match a.name.as_str() {
        "dialect" => {
            *dialects += 1;
            if *dialects > 1 {
                return Err(attribute(a.line, "`dialect` is given twice"));
            }
        }
        "start" => {
            if start.is_some() {
                return Err(attribute(a.line, "`start` is given twice"));
            }
            *start = Some((a.value, a.line));
        }
        name => not_meaningful(name, a.line, "the graph")?,
    }
    Ok(())
}

/// Accepts a render attribute; anything else is misplaced or unknown.
fn not_meaningful(name: &str, line: usize, on: &str) -> Result<(), Error> {
    if RENDER.contains(&name) {
        return Ok(());
    }
    let home = match name {
        "dialect" | "start" => Some("a graph"),
        "kind" => Some("a node"),
        "on" => Some("an edge"),
        _ => None,
    };
    Err(attribute(
        line,
        match home {
            Some(home) => format!("`{name}` is an attribute of {home}, not of {on}"),
            None => format!("unknown attribute `{name}`; see the render attributes in spec/dot.md"),
        },
    ))
}
