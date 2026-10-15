//! The build's checks beyond the schema: what joins `workflow.dot` and
//! `settings.json`, every rule of the kernel's validation (kernel.md §2) worded by
//! the build itself, file references, mappings, and likely run-time mistakes. Then,
//! when nothing is wrong, the package.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use runspore_types::artifact::{self, EntryKind, Owner};
use runspore_types::model::{OUTCOME_EXHAUSTED, OUTCOME_FAILED, OUTCOME_OK};
use serde_json::{json, Map, Value};

use crate::diagnostic::{
    pointer, Applicability, Arg, At, Diagnostic, Edit, Fix, Found, Phase, Severity,
};
use crate::dot::{arrow_text, Graph};
use crate::near::closest;
use crate::settings::{self, Action, Broken, Kind, Output};
use crate::spans::{Odd, Spans};
use crate::{
    is_outcome, File, Machine, Programs, DOT_FILE, FORMAT, ON_INTERVENTION, SETTINGS_FILE,
};

/// The build-only marker a node's input or params uses for a copy of its schemas.
const MARKER: &str = "$outputSchemas";
/// The build-only marker for the schema of a result-mode answer built from them.
const RESULT_MARKER: &str = "$resultSchema";
const MARKERS: [&str; 2] = [MARKER, RESULT_MARKER];
const GET: &str = "$get";
const LITERAL: &str = "$literal";
const DEFAULT: &str = "default";
/// An operator's name and the operands it takes: how many (`None` for any one value),
/// exactly (`true`) or at least, and their shape.
type Operator = (&'static str, Option<(usize, bool)>, &'static str);
/// The operators of kernel.md §4 beyond `$get` and `$literal`.
const OPERATORS: [Operator; 4] = [
    ("$if", Some((3, true)), "[cond, then, else]"),
    ("$uuid", Some((1, false)), "[part, ...]"),
    ("$json", None, "value"),
    ("$join", Some((2, true)), "[list, separator]"),
];
const ROOTS: [&str; 4] = ["input", "nodes", "run", "from"];
/// What a command's `result` block reads (spec/host.md §3, `command`).
const RESULT_ROOTS: [&str; 5] = ["stdout", "input", "params", "run", "step"];
/// What an action's own mappings read: its `argv`, and its probe's, watch's and
/// stop's (spec/host.md §3).
const ACTION_ROOTS: [&str; 4] = ["input", "params", "run", "step"];
/// What `onIntervention.params` reads: the run's context, without `from` (spec/host.md
/// §2, Telling a person).
const ON_INTERVENTION_ROOTS: [&str; 3] = ["input", "nodes", "run"];
/// An external action's `async.timeoutMs` when it gives none (spec/host.md §2).
const DEFAULT_TIMEOUT_MS: u64 = 3_600_000;
/// Characters a shell would treat specially: a command line holding one is not
/// split into words safely.
const SHELL: &[char] = &[
    '\'', '"', '$', '*', '?', '[', ']', '\\', '|', '&', ';', '<', '>', '(', ')', '`', '~', '{',
    '}', '#', '!',
];

pub(crate) struct Checks<'a> {
    pub folder: &'a Path,
    pub dot: &'a str,
    pub graph: Option<&'a Graph>,
    pub value: &'a Value,
    pub spans: &'a Spans,
    pub settings_text: &'a str,
    pub found: &'a mut Found,
    pub machine: &'a Machine,
    pub broken: Broken,
    /// The files the settings name, by manifest path and owner.
    pub files: RefCell<BTreeMap<(String, Option<Owner>), File>>,
    /// Each step's schemas in package form, once checked.
    schemas: BTreeMap<String, Map<String, Value>>,
    /// The workflow's input schema in package form, once checked.
    input_schema: Option<Value>,
    /// `nodes` keys that name no step, left out of every other check.
    strays: BTreeSet<String>,
    /// Steps whose settings are under a misspelt key, reported as such.
    misspelt: BTreeSet<String>,
}

impl<'a> Checks<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        folder: &'a Path,
        dot: &'a str,
        graph: Option<&'a Graph>,
        value: &'a Value,
        spans: &'a Spans,
        settings_text: &'a str,
        found: &'a mut Found,
        machine: &'a Machine,
    ) -> Self {
        Self {
            folder,
            dot,
            graph,
            value,
            spans,
            settings_text,
            found,
            machine,
            broken: Broken::default(),
            files: RefCell::new(BTreeMap::new()),
            schemas: BTreeMap::new(),
            input_schema: None,
            strays: BTreeSet::new(),
            misspelt: BTreeSet::new(),
        }
    }

    fn push(&mut self, d: Diagnostic) {
        self.found.push(d);
    }

    /// Every check, in order.
    pub fn run(&mut self) {
        self.found.phase(Phase::Join);
        if self.graph.is_some() {
            self.stray_steps();
        }
        self.found.phase(Phase::Settings);
        self.canonical();
        let mut pruned = self.value.clone();
        if let Some(nodes) = pruned.get_mut("nodes").and_then(Value::as_object_mut) {
            for stray in &self.strays {
                nodes.remove(stray);
            }
        }
        settings::shape(&pruned, self.found, &mut self.broken);
        if !self.value.is_object() {
            return;
        }
        self.input_schema();
        for id in self.ids("actions") {
            self.action(&id);
        }
        self.on_intervention();
        self.found.phase(Phase::Join);
        if let Some(graph) = self.graph {
            for id in &graph.order {
                self.step(graph, id);
            }
            self.signal_schemas(graph);
        }
        self.found.phase(Phase::Graph);
        if let Some(graph) = self.graph {
            let routes_broken = self.found.list.iter().any(|d| {
                matches!(d.phase, Phase::Dot | Phase::Join) && severity(d) == Severity::Problem
            });
            if !routes_broken {
                self.reachability(graph);
            }
        }
    }

    fn ids(&self, key: &str) -> Vec<String> {
        self.value
            .get(key)
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------------
    // Values JSON allows and the package does not
    // ------------------------------------------------------------------

    fn canonical(&mut self) {
        for odd in self.spans.odd.clone() {
            match odd {
                Odd::Duplicate(at, span) => {
                    let key = span.key.clone().unwrap_or(span.value.clone());
                    let lines = crate::spans::Lines::new(self.settings_text);
                    let (line, col) = lines.position(key.start);
                    let len = self.settings_text[key.clone()].chars().count();
                    self.broken.add(&at);
                    let mut d =
                        Diagnostic::new("settings.repeated-key", At::settings_text(line, col, len))
                            .arg("key", Arg::Key(at.clone()))
                            .arg("place", Arg::Place(parent(&at)));
                    // JSON keeps the last copy: removing the first changes nothing else.
                    if let Some(first) = self.spans.get(&at) {
                        let start = first.key.as_ref().map_or(first.value.start, |k| k.start);
                        let next = self
                            .spans
                            .children(&parent(&at))
                            .iter()
                            .filter_map(|c| self.spans.get(c))
                            .map(|c| c.key.as_ref().map_or(c.value.start, |k| k.start))
                            .chain([key.start])
                            .filter(|s| *s > first.value.end)
                            .min();
                        if let Some(next) = next {
                            let (line, col) = lines.position(start);
                            let (end_line, end_col) = lines.position(next);
                            d = d.fix(Fix::safe(vec![Edit::SettingsText(
                                crate::diagnostic::TextEdit {
                                    line,
                                    col,
                                    end_line,
                                    end_col,
                                    text: String::new(),
                                },
                            )]));
                        }
                    }
                    self.push(d);
                }
                Odd::NotInteger(at) => {
                    self.broken.add(&at);
                    let mut d = Diagnostic::new("settings.not-integer", At::value(&at))
                        .arg("value", Arg::Input(at.clone()))
                        .arg("place", Arg::Place(at.clone()));
                    if let Some(n) = self.value.pointer(&at).and_then(Value::as_f64) {
                        let rounded = json!(n.round() as i64);
                        d = d.fix(
                            Fix::review(vec![Edit::Set {
                                at: at.clone(),
                                value: rounded.clone(),
                            }])
                            .arg("rounded", Arg::Computed(rounded)),
                        );
                    }
                    self.push(d);
                }
                Odd::OutOfRange(at) => {
                    self.broken.add(&at);
                    self.push(
                        Diagnostic::new("settings.number-too-large", At::value(&at))
                            .arg("value", Arg::Input(at.clone()))
                            .arg("place", Arg::Place(at)),
                    );
                }
                Odd::TooDeep(at) => {
                    self.broken.add(&at);
                    // The member it is in, three levels down at most: the full path is
                    // as deep as the problem.
                    let steps = crate::diagnostic::steps(&at);
                    let place = steps
                        .iter()
                        .take(3)
                        .fold(String::new(), |p, s| pointer(&p, s));
                    self.push(
                        Diagnostic::new("settings.too-deep", At::value(&at))
                            .arg("place", Arg::Place(place)),
                    );
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // `nodes` against the DOT
    // ------------------------------------------------------------------

    /// A key under `nodes` that names no step: a field that belongs elsewhere, a
    /// step's name misspelt, or a step `workflow.dot` lacks.
    fn stray_steps(&mut self) {
        let Some(graph) = self.graph else { return };
        let Some(nodes) = self.value.get("nodes").and_then(Value::as_object) else {
            return;
        };
        let action_fields = settings::fields("Action");
        let step_fields = settings::fields("Step");
        let keys: Vec<&String> = nodes.keys().collect();
        let text_order = self.spans.children("/nodes").to_vec();
        for key in keys {
            if graph.nodes.contains_key(key) {
                continue;
            }
            let at = pointer("/nodes", key);
            self.strays.insert(key.clone());
            let previous_step = text_order
                .iter()
                .position(|p| *p == at)
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| text_order.get(i))
                .map(|p| crate::diagnostic::steps(p).pop().unwrap_or_default());
            let moved = nodes[key].clone();
            let d = if action_fields.contains(key) && !step_fields.contains(key) {
                // Wording differs by grammar: "outcomes belong", "effect is a key".
                let code = if key.ends_with('s') {
                    "settings.misplaced-key"
                } else {
                    "settings.misplaced-action-key"
                };
                // Into the action of the step written just above, else every action.
                let previous_action = previous_step
                    .as_ref()
                    .and_then(|s| nodes.get(s))
                    .and_then(|s| s.get("action"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let mut targets: Vec<String> = previous_action.into_iter().collect();
                for id in self.ids("actions") {
                    if !targets.contains(&id) {
                        targets.push(id);
                    }
                }
                if targets.is_empty() {
                    targets.extend(
                        previous_step
                            .clone()
                            .or_else(|| graph.order.first().cloned()),
                    );
                }
                let fixes = targets.iter().map(|a| {
                    let target = pointer(&pointer("/actions", a), key);
                    Fix::choice(vec![
                        Edit::Remove { at: at.clone() },
                        Edit::Set {
                            at: target,
                            value: moved.clone(),
                        },
                    ])
                    .arg("target", Arg::Place(pointer("/actions", a)))
                    .arg("field", Arg::Code(key.clone()))
                });
                Diagnostic::new(code, At::key(&at))
                    .arg("key", Arg::Key(at.clone()))
                    .fixes(fixes)
            } else if step_fields.contains(key) {
                let mut targets: Vec<String> = previous_step.into_iter().collect();
                for id in &graph.order {
                    if !targets.contains(id) && nodes.contains_key(id) {
                        targets.push(id.clone());
                    }
                }
                if targets.is_empty() {
                    targets.extend(graph.order.first().cloned());
                }
                let fixes = targets.iter().map(|s| {
                    let target = pointer(&pointer("/nodes", s), key);
                    let edits = if nodes.contains_key(s) {
                        vec![
                            Edit::Remove { at: at.clone() },
                            Edit::Set {
                                at: target,
                                value: moved.clone(),
                            },
                        ]
                    } else {
                        // The step has no entry yet: this line becomes it.
                        let mut entry = Map::new();
                        entry.insert(key.clone(), moved.clone());
                        vec![Edit::Replace {
                            at: at.clone(),
                            key: s.clone(),
                            value: Value::Object(entry),
                        }]
                    };
                    Fix::choice(edits)
                        .arg("target", Arg::Place(pointer("/nodes", s)))
                        .arg("field", Arg::Code(key.clone()))
                });
                Diagnostic::new("settings.misplaced-step-key", At::key(&at))
                    .arg("key", Arg::Key(at.clone()))
                    .fixes(fixes)
            } else {
                let unnamed = graph
                    .order
                    .iter()
                    .map(String::as_str)
                    .filter(|s| !nodes.contains_key(*s));
                let near = closest(key, unnamed);
                if let Some(near) = near {
                    self.misspelt.insert(near.to_string());
                }
                match near {
                    Some(near) => Diagnostic::new("settings.step-typo", At::key(&at))
                        .arg("key", Arg::Key(at.clone()))
                        .fix(
                            Fix::review(vec![Edit::Rename {
                                at: at.clone(),
                                to: near.to_string(),
                            }])
                            .arg("closest", Arg::computed(near.to_string())),
                        ),
                    None => Diagnostic::new("settings.unknown-step", At::key(&at))
                        .arg("key", Arg::Key(at.clone()))
                        .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
                }
            };
            self.push(d);
        }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    /// An action's outcomes, as the routes see them: `None` when they are broken.
    fn outcomes(&self, action: &str) -> Option<Vec<String>> {
        let at = pointer(&pointer("/actions", action), "outcomes");
        if self.broken.touches(&at) || self.broken.within(&pointer("/actions", action)) {
            return None;
        }
        let action = self.value.get("actions")?.get(action)?;
        match action.get("outcomes") {
            None => Some(vec![OUTCOME_OK.to_string()]),
            Some(Value::Array(list)) => list.iter().map(|o| o.as_str().map(String::from)).collect(),
            Some(_) => None,
        }
    }

    fn action(&mut self, id: &str) {
        let here = pointer("/actions", id);
        if self.broken.within(&here) {
            return;
        }
        let Some(raw) = self.value.pointer(&here).cloned() else {
            return;
        };
        if !raw.is_object() {
            return;
        }
        if !crate::is_node_id(id) {
            self.push(Diagnostic::new("action.bad-name", At::key(&here)));
        }
        let outcomes_at = pointer(&here, "outcomes");
        if let (false, Some(Value::Array(list))) =
            (self.broken.touches(&outcomes_at), raw.get("outcomes"))
        {
            // A list the routes cannot use: its arrows are not checked against it.
            let mut unusable = list.is_empty();
            if list.is_empty() {
                self.push(
                    Diagnostic::new("action.no-outcomes", At::key(&outcomes_at)).fix(Fix::review(
                        vec![Edit::Remove {
                            at: outcomes_at.clone(),
                        }],
                    )),
                );
            }
            let mut seen = BTreeSet::new();
            for (i, outcome) in list.iter().enumerate() {
                let at = pointer(&outcomes_at, &i.to_string());
                let Some(outcome) = outcome.as_str() else {
                    continue;
                };
                if outcome == OUTCOME_FAILED || outcome == OUTCOME_EXHAUSTED {
                    unusable = true;
                    self.push(
                        Diagnostic::new("action.reserved-outcome", At::value(&at))
                            .arg("outcome", Arg::Input(at.clone()))
                            .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
                    );
                } else if !is_outcome(outcome) {
                    let fixed: String = outcome
                        .to_lowercase()
                        .chars()
                        .map(|c| {
                            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                                c
                            } else {
                                '-'
                            }
                        })
                        .collect::<String>()
                        .trim_matches('-')
                        .chars()
                        .take(64)
                        .collect();
                    unusable = true;
                    let mut d = Diagnostic::new("action.bad-outcome", At::value(&at))
                        .arg("outcome", Arg::Input(at.clone()));
                    if is_outcome(&fixed) {
                        d = d.fix(Fix::review(vec![Edit::Set {
                            at: at.clone(),
                            value: json!(fixed),
                        }]));
                    }
                    self.push(d);
                } else if !seen.insert(outcome) {
                    self.push(
                        Diagnostic::new("action.repeated-outcome", At::value(&at))
                            .arg("outcome", Arg::Input(at.clone()))
                            .fix(Fix::safe(vec![Edit::Remove { at: at.clone() }])),
                    );
                }
            }
            if unusable {
                self.broken.add(&outcomes_at);
            }
        }
        if let Some(Value::Array(assets)) = raw.get("assets") {
            for i in 0..assets.len() {
                let at = pointer(&pointer(&here, "assets"), &i.to_string());
                if !self.broken.touches(&at) {
                    self.reference(&at, EntryKind::Asset, Some(Owner::Action(id.to_string())));
                }
            }
        }
        self.bundled_programs(id, &raw);
        for (object, field) in [
            ("retry", "backoffMs"),
            ("retry", "maxBackoffMs"),
            ("async", "timeoutMs"),
            ("async", "leaseMs"),
        ] {
            let at = pointer(&pointer(&here, object), field);
            let ms = self.value.pointer(&at).and_then(Value::as_str);
            let too_large = ms
                .and_then(|ms| ms.parse::<u64>().ok())
                .is_some_and(|ms| ms > i64::MAX as u64);
            if too_large && !self.broken.touches(&at) {
                let limit = json!(i64::MAX.to_string());
                self.push(
                    Diagnostic::new("settings.too-large", At::value(&at))
                        .arg("value", Arg::Input(at.clone()))
                        .arg("place", Arg::Place(at.clone()))
                        .fix(
                            Fix::review(vec![Edit::Set {
                                at: at.clone(),
                                value: limit.clone(),
                            }])
                            .arg("limit", Arg::Computed(limit)),
                        ),
                );
            }
        }
        let argv_at = pointer(&here, "argv");
        let command = raw.get("kind").and_then(Value::as_str) == Some("command");
        if let (true, false, Some(Value::Array(argv))) =
            (command, self.broken.touches(&argv_at), raw.get("argv"))
        {
            self.program(id, argv);
            self.argv_mappings(&argv_at, argv, id);
        }
        for program in ["probe", "watch", "stop"] {
            let at = format!("{here}/async/{program}/command/argv");
            if let (false, Some(Value::Array(argv))) =
                (self.broken.touches(&at), self.value.pointer(&at))
            {
                self.argv_mappings(&at, &argv.clone(), id);
            }
        }
        // The rest reads the action as the build's own type, once it has that shape.
        let Ok(action) = serde_json::from_value::<Action>(raw.clone()) else {
            return;
        };
        self.lease(id, &action);
        if action.kind == Kind::External {
            return;
        }
        if action.kind == Kind::Native {
            if let (Some(known), Some(function)) = (&self.machine.functions, &action.function) {
                if !known.contains(function) {
                    self.push(
                        Diagnostic::new(
                            "action.unknown-function",
                            At::value(pointer(&here, "function")),
                        )
                        .arg("function", Arg::Input(pointer(&here, "function"))),
                    );
                }
            }
            return;
        }
        let declared = action
            .outcomes
            .clone()
            .unwrap_or_else(|| vec![OUTCOME_OK.to_string()]);
        // Exit 0 reads the result from stdout in result mode and with a `result` block.
        let reads_result = action.output == Some(Output::Result) || action.result.is_some();
        let exits_at = pointer(&here, "exitOutcomes");
        for (code, outcome) in action.exit_outcomes.iter().flatten() {
            let at = pointer(&exits_at, code);
            if code.parse::<i32>().is_err() {
                self.push(
                    Diagnostic::new("command.bad-exit-code", At::key(&at))
                        .arg("exit", Arg::Key(at.clone())),
                );
            } else if code.parse::<i32>() == Ok(0) && reads_result {
                self.push(
                    Diagnostic::new("command.result-exit-zero", At::key(&at))
                        .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
                );
            } else if !declared.contains(outcome) {
                let mut with = declared.clone();
                with.push(outcome.clone());
                self.push(
                    Diagnostic::new("command.exit-outcome", At::value(&at))
                        .arg("exit", Arg::Key(at.clone()))
                        .arg("outcome", Arg::Input(at.clone()))
                        .fix(Fix::review(vec![Edit::Set {
                            at: pointer(&here, "outcomes"),
                            value: json!(with),
                        }])),
                );
            }
        }
        if let Some(result) = &action.result {
            let at = pointer(&here, "result");
            if action.output != Some(Output::Json) {
                self.push(
                    Diagnostic::new("command.result-not-json", At::key(&at)).fix(Fix::review(
                        vec![Edit::Set {
                            at: pointer(&here, "output"),
                            value: json!("json"),
                        }],
                    )),
                );
            }
            for field in ["outcome", "output"] {
                let field_at = pointer(&at, field);
                if let (Some(mapping), false) = (result.get(field), self.broken.touches(&field_at))
                {
                    self.mapping(&field_at, mapping, None, id, &RESULT_ROOTS);
                }
            }
        }
    }

    /// An async action's holder renews within `leaseMs`, so it must be shorter than the
    /// deadline, `timeoutMs` (spec/host.md §2, Steps outside the worker).
    fn lease(&mut self, id: &str, action: &Action) {
        let Some(spec) = &action.asynchronous else {
            return;
        };
        let at = pointer(&pointer(&pointer("/actions", id), "async"), "leaseMs");
        let millis = |text: &Option<String>| text.as_deref().map(str::parse::<u64>);
        let timeout = match millis(&spec.timeout_ms) {
            None => DEFAULT_TIMEOUT_MS,
            Some(Ok(timeout)) => timeout,
            Some(Err(_)) => return,
        };
        let Some(Ok(lease)) = millis(&spec.lease_ms) else {
            return;
        };
        if lease < timeout || self.broken.touches(&at) {
            return;
        }
        self.push(
            Diagnostic::new("action.lease-too-long", At::value(&at))
                .arg("lease", Arg::Input(at.clone()))
                .arg("timeout", Arg::Computed(json!(timeout.to_string())))
                .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
        );
    }

    /// The mappings among the items of an `argv` at `at`, an action's own: their paths
    /// start with one of `ACTION_ROOTS`. A string item is a word as it is.
    fn argv_mappings(&mut self, at: &str, argv: &[Value], id: &str) {
        for (i, item) in argv.iter().enumerate() {
            if item.is_object() {
                self.mapping(&pointer(at, &i.to_string()), item, None, id, &ACTION_ROOTS);
            }
        }
    }

    /// `argv[0]` holding a whole command line, or a program this machine lacks. A
    /// mapping in `argv[0]` names its program only at dispatch.
    fn program(&mut self, id: &str, argv: &[Value]) {
        let Some(Value::String(program)) = argv.first() else {
            return;
        };
        let at = pointer(&pointer(&pointer("/actions", id), "argv"), "0");
        let exists = |p: &str| Path::new(p).exists() || self.folder.join(p).exists();
        if program.trim().contains(char::is_whitespace) && !exists(program) {
            let mut words: Vec<Value> = program.split_whitespace().map(|w| json!(w)).collect();
            words.extend(argv[1..].iter().cloned());
            let applicability = if program.contains(SHELL) {
                Applicability::NeedsReview
            } else {
                Applicability::Safe
            };
            self.push(
                Diagnostic::new("command.argv-spaces", At::value(&at))
                    .arg("item", Arg::Input(at.clone()))
                    .fix(Fix::new(
                        applicability,
                        vec![Edit::Set {
                            at: parent(&at),
                            value: json!(words),
                        }],
                    )),
            );
            return;
        }
        if program.contains('/') || program.is_empty() {
            return;
        }
        let found = match &self.machine.programs {
            Programs::Unchecked => true,
            Programs::Known(list) => list.iter().any(|p| p == program),
            Programs::Path(path) => std::env::split_paths(path).any(|dir| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(dir.join(program))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }),
        };
        if !found {
            self.push(
                Diagnostic::new("command.program-not-found", At::value(&at))
                    .arg("program", Arg::Input(at.clone())),
            );
        }
    }

    /// An async probe, watch or stop named `./<path>` is a file of the folder, bundled
    /// as the action's asset (spec/dot.md §4). A bare name is looked up when it runs.
    fn bundled_programs(&mut self, id: &str, action: &Value) {
        let Some(Value::Object(spec)) = action.get("async") else {
            return;
        };
        for program in ["probe", "watch", "stop"] {
            let argv0 = spec
                .get(program)
                .and_then(|p| p.get("command"))
                .and_then(|c| c.get("argv"))
                .and_then(|a| a.get(0));
            if let Some(Value::String(argv0)) = argv0 {
                if let Some(path) = argv0.strip_prefix("./") {
                    let at = format!("/actions/{}/async/{program}/command/argv/0", escape(id));
                    self.reference_path(
                        &at,
                        path,
                        EntryKind::Asset,
                        Some(Owner::Action(id.to_string())),
                    );
                }
            }
        }
    }

    /// The workflow's `onIntervention` (spec/host.md §2, Telling a person): an action
    /// `actions` defines and the worker runs itself, and params that map the run.
    fn on_intervention(&mut self) {
        let here = pointer("", ON_INTERVENTION);
        if self.broken.within(&here) {
            return;
        }
        let Some(hook) = self.value.get(ON_INTERVENTION).cloned() else {
            return;
        };
        let action_at = pointer(&here, "action");
        let action = hook
            .get("action")
            .and_then(Value::as_str)
            .filter(|_| !self.broken.touches(&action_at));
        if let Some(action) = action {
            match self.value.pointer(&pointer("/actions", action)) {
                None => {
                    let fixes = self.action_fixes(action, &action_at);
                    self.push(
                        Diagnostic::new("on-intervention.unknown-action", At::value(&action_at))
                            .arg("name", Arg::Input(action_at.clone()))
                            .fixes(fixes),
                    );
                }
                Some(def) if def.get("kind").and_then(Value::as_str) == Some("external") => {
                    self.push(
                        Diagnostic::new("on-intervention.external-action", At::value(&action_at))
                            .arg("name", Arg::Input(action_at.clone())),
                    );
                }
                Some(_) => {}
            }
        }
        let params_at = pointer(&here, "params");
        if let Some(params) = hook.get("params") {
            if !self.broken.touches(&params_at) {
                self.mapping(&params_at, params, None, "", &ON_INTERVENTION_ROOTS);
            }
        }
    }

    /// Fixes naming another action at `at` for an action `actions` lacks: the closest
    /// name first, to review, then every other one, to choose from.
    fn action_fixes(&self, action: &str, at: &str) -> Vec<Fix> {
        let actions = self.ids("actions");
        let near = closest(action, actions.iter().map(String::as_str));
        let mut order: Vec<&str> = near.into_iter().collect();
        order.extend(
            actions
                .iter()
                .map(String::as_str)
                .filter(|a| Some(*a) != near),
        );
        order
            .iter()
            .map(|a| {
                let applicability = if Some(*a) == near {
                    Applicability::NeedsReview
                } else {
                    Applicability::NeedsChoice
                };
                Fix::new(
                    applicability,
                    vec![Edit::Set {
                        at: at.to_string(),
                        value: json!(a),
                    }],
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // Steps
    // ------------------------------------------------------------------

    fn step(&mut self, graph: &Graph, id: &str) {
        if graph.unsure.contains(id) {
            return;
        }
        let node = &graph.nodes[id];
        let here = pointer("/nodes", id);
        let kind = node.kind;
        let (allowed, required): (&[&str], Option<&str>) = match kind {
            "activity" => (
                &[
                    "action",
                    "input",
                    "params",
                    "instructions",
                    "outputSchemas",
                    "loopLimits",
                ],
                Some("action"),
            ),
            "await-signal" => (
                &["signal", "instructions", "outputSchemas", "loopLimits"],
                Some("signal"),
            ),
            "complete" => (&["output"], None),
            _ => (&["error"], Some("error")),
        };
        let entry = self.value.pointer(&here).cloned();
        let Some(entry) = entry else {
            if let (Some(field), false) = (required, self.misspelt.contains(id)) {
                let d = Diagnostic::new("node.missing-field", node.spot_at().of_step(id))
                    .arg("kind", Arg::computed(kind.to_string()))
                    .arg("field", Arg::computed(field.to_string()))
                    .fixes(self.guess_field(id, field));
                self.push(d);
            }
            self.routes(graph, id, None);
            return;
        };
        if self.broken.within(&here) || !entry.is_object() {
            return;
        }
        for key in entry.as_object().into_iter().flat_map(|m| m.keys()) {
            let at = pointer(&here, key);
            if self.broken.within(&at) || allowed.contains(&key.as_str()) {
                continue;
            }
            self.broken.add(&at);
            self.push(
                Diagnostic::new("node.field-not-allowed", At::key(&at))
                    .arg("key", Arg::Key(at.clone()))
                    .arg("kind", Arg::computed(kind.to_string()))
                    .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
            );
        }
        if let Some(field) = required {
            let step_fields = settings::fields("Step");
            let misspelt = entry
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys())
                .any(|k| {
                    !step_fields.contains(k)
                        && closest(k, step_fields.iter().map(String::as_str)) == Some(field)
                });
            if entry.get(field).is_none()
                && !misspelt
                && !self.broken.touches(&pointer(&here, field))
            {
                self.push(
                    Diagnostic::new("node.missing-field", At::key(&here))
                        .arg("kind", Arg::computed(kind.to_string()))
                        .arg("field", Arg::computed(field.to_string()))
                        .fixes(self.guess_field(id, field)),
                );
            }
        }
        if let Some(instructions) = entry.get("instructions") {
            let at = pointer(&here, "instructions");
            if allowed.contains(&"instructions")
                && !self.broken.touches(&at)
                && instructions.is_string()
            {
                self.reference(
                    &at,
                    EntryKind::Instructions,
                    Some(Owner::Node(id.to_string())),
                );
            }
        }
        self.routes(graph, id, Some(&entry));
        if allowed.contains(&"outputSchemas") {
            self.output_schemas(graph, id, &entry);
        }
        for field in ["input", "params"] {
            if let Some(value) = entry.get(field) {
                let at = pointer(&here, field);
                if allowed.contains(&field) && !self.broken.touches(&at) {
                    self.mapping(&at, value, Some(field), id, &ROOTS);
                }
            }
        }
        if let Some(output) = entry.get("output") {
            let at = pointer(&here, "output");
            if allowed.contains(&"output") && !self.broken.touches(&at) {
                self.mapping(&at, output, None, id, &ROOTS);
            }
        }
    }

    /// Fixes giving a step the field its kind needs.
    fn guess_field(&self, id: &str, field: &str) -> Vec<Fix> {
        let here = pointer("/nodes", id);
        let set = |value: Value| Edit::Set {
            at: pointer(&here, field),
            value,
        };
        match field {
            "action" => {
                let actions = self.ids("actions");
                let mut order: Vec<&str> = actions
                    .iter()
                    .map(String::as_str)
                    .filter(|a| *a == id)
                    .collect();
                order.extend(actions.iter().map(String::as_str).filter(|a| *a != id));
                order
                    .into_iter()
                    .map(|a| Fix::choice(vec![set(json!(a))]))
                    .collect()
            }
            "signal" => vec![Fix::review(vec![set(json!(id))])],
            _ => {
                let code: String = id
                    .to_lowercase()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '.' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect();
                let code = if code.starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    code
                } else {
                    format!("x{code}")
                };
                vec![Fix::review(vec![set(json!({"code": code, "message": id}))])]
            }
        }
    }

    /// Arrows against outcomes (kernel.md W08, W09, W12): every outcome a step can end
    /// with has an arrow, every arrow is for an outcome it can end with.
    fn routes(&mut self, graph: &Graph, id: &str, entry: Option<&Value>) {
        let node = &graph.nodes[id];
        let here = pointer("/nodes", id);
        let arrows: Vec<&crate::dot::Edge> = graph.edges.iter().filter(|e| e.from == id).collect();
        let limits_at = pointer(&here, "loopLimits");
        let has_limits = entry.is_some_and(|e| e.get("loopLimits").is_some());
        let limits_ok = has_limits && !self.broken.touches(&limits_at);
        match node.kind {
            "activity" => {
                let action = entry.and_then(|e| e.get("action")).and_then(Value::as_str);
                let action_at = pointer(&here, "action");
                let Some(action) = action.filter(|_| !self.broken.touches(&action_at)) else {
                    return;
                };
                if self.value.pointer(&pointer("/actions", action)).is_none() {
                    let fixes = self.action_fixes(action, &action_at);
                    self.push(
                        Diagnostic::new("node.unknown-action", At::value(&action_at))
                            .arg("name", Arg::Input(action_at.clone()))
                            .fixes(fixes),
                    );
                    return;
                }
                let Some(declared) = self.outcomes(action) else {
                    return;
                };
                let explicit = self
                    .value
                    .pointer(&pointer(&pointer("/actions", action), "outcomes"))
                    .is_some();
                for outcome in &declared {
                    if arrows.iter().any(|e| &e.on == outcome) {
                        continue;
                    }
                    let fixes = self.arrow_fixes(graph, id, outcome);
                    let code = if explicit {
                        "dot.missing-outcome-arrow"
                    } else {
                        "dot.missing-arrow"
                    };
                    let mut d = Diagnostic::new(code, node.spot_at().of_step(id))
                        .arg("outcome", Arg::computed(outcome.clone()))
                        .arg("action", Arg::Input(action_at.clone()))
                        .fixes(fixes);
                    if explicit {
                        d = d.arg(
                            "outcomes",
                            Arg::Input(pointer(&pointer("/actions", action), "outcomes")),
                        );
                    }
                    self.push(d);
                }
                for e in &arrows {
                    if e.on == OUTCOME_EXHAUSTED {
                        if !has_limits {
                            self.push(Diagnostic::new(
                                "dot.exhausted-arrow",
                                e.spot.at().of_step(id),
                            ));
                        }
                    } else if e.on != OUTCOME_FAILED && !declared.contains(&e.on) {
                        let mut with = declared.clone();
                        with.push(e.on.clone());
                        self.push(
                            Diagnostic::new("dot.undeclared-outcome", e.spot.at().of_step(id))
                                .arg("outcome", Arg::computed(e.on.clone()))
                                .arg("action", Arg::Input(action_at.clone()))
                                .arg("declared", Arg::computed(declared.clone()))
                                .fix(Fix::review(vec![Edit::Set {
                                    at: pointer(&pointer("/actions", action), "outcomes"),
                                    value: json!(with),
                                }])),
                        );
                    }
                }
            }
            "await-signal" => {
                if arrows.is_empty() {
                    let fixes = self.arrow_fixes(graph, id, "received");
                    self.push(
                        Diagnostic::new("dot.no-arrows", node.spot_at().of_step(id)).fixes(fixes),
                    );
                }
                for e in &arrows {
                    if e.on == OUTCOME_EXHAUSTED && !has_limits {
                        self.push(Diagnostic::new(
                            "dot.exhausted-arrow",
                            e.spot.at().of_step(id),
                        ));
                    }
                }
            }
            _ => return,
        }
        if !limits_ok {
            return;
        }
        let Some(limits) = entry
            .and_then(|e| e.get("loopLimits"))
            .and_then(Value::as_object)
        else {
            return;
        };
        let routed: BTreeSet<&str> = arrows.iter().map(|e| e.on.as_str()).collect();
        let max_visits = self
            .value
            .pointer("/limits/maxVisitsPerNode")
            .and_then(Value::as_u64)
            .unwrap_or(16);
        for (outcome, limit) in limits {
            let at = pointer(&limits_at, outcome);
            if outcome == OUTCOME_EXHAUSTED {
                self.push(
                    Diagnostic::new("node.limit-on-exhausted", At::key(&at))
                        .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
                );
            } else if !routed.contains(outcome.as_str()) {
                let candidates = routed
                    .iter()
                    .copied()
                    .filter(|o| *o != OUTCOME_EXHAUSTED && !limits.contains_key(*o));
                let near = closest(outcome, candidates.clone());
                let mut order: Vec<&str> = near.into_iter().collect();
                order.extend(candidates.filter(|o| Some(*o) != near));
                let fixes = order.iter().map(|o| {
                    Fix::choice(vec![Edit::Rename {
                        at: at.clone(),
                        to: o.to_string(),
                    }])
                });
                self.push(
                    Diagnostic::new("node.limit-unknown-outcome", At::key(&at))
                        .arg("outcome", Arg::Key(at.clone()))
                        .fixes(fixes),
                );
            } else if limit.as_u64().is_some_and(|l| l + 1 > max_visits) {
                let highest = max_visits.saturating_sub(1).max(1);
                self.push(
                    Diagnostic::new("node.limit-too-high", At::value(&at))
                        .arg("limit", Arg::Input(at.clone()))
                        .arg("outcome", Arg::Key(at.clone()))
                        .arg("max", Arg::computed(max_visits))
                        .fix(
                            Fix::review(vec![Edit::Set {
                                at: at.clone(),
                                value: json!(highest),
                            }])
                            .arg("highest", Arg::computed(highest)),
                        ),
                );
            }
        }
        if !routed.contains(OUTCOME_EXHAUSTED) {
            let node = &graph.nodes[id];
            let fixes = self.arrow_fixes(graph, id, OUTCOME_EXHAUSTED);
            self.push(
                Diagnostic::new("dot.no-exhausted-arrow", node.spot_at().of_step(id)).fixes(fixes),
            );
        }
    }

    /// Fixes drawing the arrow for `outcome` from `id` to each end (complete first).
    fn arrow_fixes(&self, graph: &Graph, id: &str, outcome: &str) -> Vec<Fix> {
        graph
            .arrow_targets(id)
            .into_iter()
            .map(|(to, declare)| {
                let line = arrow_text(id, &to, outcome);
                let mut lines: Vec<String> = declare.into_iter().collect();
                lines.push(line.clone());
                Fix::choice(vec![graph.add_lines(self.dot, &lines)]).arg("arrow", Arg::Code(line))
            })
            .collect()
    }

    /// A step's `outputSchemas` (kernel.md W13, schema.md): keyed by outcomes the step
    /// ends with, each a schema in the subset, from the file when it names one.
    fn output_schemas(&mut self, graph: &Graph, id: &str, entry: &Value) {
        let here = pointer("/nodes", id);
        let at = pointer(&here, "outputSchemas");
        let Some(Value::Object(schemas)) = entry.get("outputSchemas") else {
            return;
        };
        if self.broken.within(&at) {
            return;
        }
        let produced: Option<Vec<String>> = match graph.nodes[id].kind {
            "activity" => entry
                .get("action")
                .and_then(Value::as_str)
                .filter(|a| self.value.pointer(&pointer("/actions", a)).is_some())
                .and_then(|a| self.outcomes(a)),
            _ => Some(
                graph
                    .edges
                    .iter()
                    .filter(|e| e.from == id)
                    .map(|e| e.on.clone())
                    .collect(),
            ),
        };
        let mut packaged = Map::new();
        let mut sound = true;
        for (outcome, schema) in schemas {
            let at = pointer(&at, outcome);
            if self.broken.touches(&at) {
                sound = false;
                continue;
            }
            if outcome == OUTCOME_FAILED || outcome == OUTCOME_EXHAUSTED {
                self.push(
                    Diagnostic::new("node.schema-reserved-outcome", At::key(&at))
                        .arg("outcome", Arg::Key(at.clone()))
                        .fix(Fix::review(vec![Edit::Remove { at: at.clone() }])),
                );
                sound = false;
                continue;
            }
            if let Some(produced) = &produced {
                if !produced.contains(outcome) {
                    let candidates = produced.iter().map(String::as_str).filter(|o| {
                        *o != OUTCOME_FAILED && *o != OUTCOME_EXHAUSTED && !schemas.contains_key(*o)
                    });
                    let near = closest(outcome, candidates.clone());
                    let mut order: Vec<&str> = near.into_iter().collect();
                    order.extend(candidates.filter(|o| Some(*o) != near));
                    let fixes = order.iter().map(|o| {
                        Fix::choice(vec![Edit::Rename {
                            at: at.clone(),
                            to: o.to_string(),
                        }])
                    });
                    self.push(
                        Diagnostic::new("node.schema-unknown-outcome", At::key(&at))
                            .arg("outcome", Arg::Key(at.clone()))
                            .fixes(fixes),
                    );
                    sound = false;
                    continue;
                }
            }
            let authored = match schema {
                Value::Object(_) => schema.clone(),
                Value::String(_) => {
                    let Some(target) = self.reference(&at, EntryKind::Source, None) else {
                        sound = false;
                        continue;
                    };
                    let parsed = std::fs::read(&target)
                        .ok()
                        .and_then(|bytes| runspore_types::canonical::parse(&bytes).ok());
                    match parsed {
                        Some(v) => v,
                        None => {
                            self.push(
                                Diagnostic::new("schema.file-not-json", At::value(&at))
                                    .arg("path", Arg::Input(at.clone()))
                                    .arg("outcome", Arg::Key(at.clone())),
                            );
                            sound = false;
                            continue;
                        }
                    }
                }
                _ => continue,
            };
            let checked = runspore_schema::inline(&authored).and_then(|schema| {
                runspore_schema::check(&schema, runspore_schema::ROOT_LEVEL).map(|()| schema)
            });
            match checked {
                Ok(schema) => {
                    packaged.insert(outcome.clone(), schema);
                }
                Err(e) => {
                    let inner = format!("{at}{}", e.path);
                    let spot = if schema.is_object() && self.spans.get(&inner).is_some() {
                        At::key(&inner)
                    } else {
                        At::value(&at)
                    };
                    self.push(
                        Diagnostic::new("schema.outside-subset", spot)
                            .arg("outcome", Arg::Key(at.clone()))
                            .arg(
                                "keyword",
                                Arg::computed(runspore_schema::shown(&e.path).to_string()),
                            )
                            .arg("reason", Arg::Code(e.message.clone())),
                    );
                    sound = false;
                }
            }
        }
        if sound {
            self.schemas.insert(id.to_string(), packaged);
        }
    }

    /// The workflow's `inputSchema` (schema.md §4): a schema in the subset at its place
    /// in the package, from the file when it names one, with a root of
    /// `"type": "object"`.
    fn input_schema(&mut self) {
        let at = "/inputSchema";
        let Some(schema) = self.value.get("inputSchema") else {
            return;
        };
        if self.broken.touches(at) {
            return;
        }
        let authored = match schema {
            Value::Object(_) => schema.clone(),
            Value::String(_) => {
                let Some(target) = self.reference(at, EntryKind::Source, None) else {
                    return;
                };
                let parsed = std::fs::read(&target)
                    .ok()
                    .and_then(|bytes| runspore_types::canonical::parse(&bytes).ok());
                match parsed {
                    Some(v) => v,
                    None => {
                        self.push(
                            Diagnostic::new("schema.input-file-not-json", At::value(at))
                                .arg("path", Arg::Input(at.to_string())),
                        );
                        return;
                    }
                }
            }
            _ => return,
        };
        let checked = runspore_schema::inline(&authored).and_then(|schema| {
            runspore_schema::check(&schema, runspore_schema::INPUT_LEVEL).map(|()| schema)
        });
        let checked = match checked {
            Ok(schema) => schema,
            Err(e) => {
                let inner = format!("{at}{}", e.path);
                let spot = if schema.is_object() && self.spans.get(&inner).is_some() {
                    At::key(&inner)
                } else {
                    At::value(at)
                };
                self.push(
                    Diagnostic::new("schema.input-outside-subset", spot)
                        .arg(
                            "keyword",
                            Arg::computed(runspore_schema::shown(&e.path).to_string()),
                        )
                        .arg("reason", Arg::Code(e.message.clone())),
                );
                return;
            }
        };
        if checked.get("type") != Some(&json!("object")) {
            // Inline, the fix sets the root's type; a schema file is the author's to edit.
            let typed = pointer(at, "type");
            let d = match schema {
                Value::Object(written) => {
                    let spot = match written.contains_key("type") {
                        true => At::value(&typed),
                        false => At::key(at),
                    };
                    Diagnostic::new("schema.input-not-object", spot).fix(Fix::review(vec![
                        Edit::Set {
                            at: typed,
                            value: json!("object"),
                        },
                    ]))
                }
                _ => Diagnostic::new("schema.input-not-object", At::value(at)),
            };
            self.push(d);
            return;
        }
        self.input_schema = Some(checked);
    }

    /// Await-signal steps that wait for one signal declare equal schemas (W13).
    fn signal_schemas(&mut self, graph: &Graph) {
        let mut first: BTreeMap<String, String> = BTreeMap::new();
        for id in graph.nodes.keys() {
            if graph.nodes[id].kind != "await-signal" {
                continue;
            }
            let here = pointer("/nodes", id);
            let Some(signal) = self
                .value
                .pointer(&pointer(&here, "signal"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Some(other) = first.get(signal).cloned() else {
                first.insert(signal.to_string(), id.clone());
                continue;
            };
            // A step with no outputSchemas has none for any outcome; one whose schemas
            // have a problem already reported is not compared.
            let schemas_of = |step: &str| {
                let written = self
                    .value
                    .pointer(&pointer(&pointer("/nodes", step), "outputSchemas"))
                    .is_some();
                match written {
                    false => Some(Map::new()),
                    true => self.schemas.get(step).cloned(),
                }
            };
            let (Some(mine), Some(theirs)) = (schemas_of(id), schemas_of(&other)) else {
                continue;
            };
            let routes = |n: &str| -> BTreeSet<&str> {
                graph
                    .edges
                    .iter()
                    .filter(|e| e.from == n)
                    .map(|e| e.on.as_str())
                    .collect()
            };
            let shared: Vec<String> = routes(id)
                .intersection(&routes(&other))
                .map(|s| s.to_string())
                .collect();
            for outcome in shared {
                if mine.get(&outcome) == theirs.get(&outcome) {
                    continue;
                }
                let at = match mine.get(&outcome) {
                    Some(_) => pointer(&pointer(&here, "outputSchemas"), &outcome),
                    None => pointer(&here, "signal"),
                };
                self.push(
                    Diagnostic::new("node.signal-schemas-differ", At::key(&at))
                        .arg("other", Arg::computed(other.clone()))
                        .arg("signal", Arg::Input(pointer(&here, "signal")))
                        .arg("outcome", Arg::computed(outcome.clone())),
                );
            }
        }
    }

    /// Mappings (kernel.md §4, W10) whose paths start with one of `roots`, and in a
    /// step's input or params (`field`) the `$outputSchemas` and `$resultSchema`
    /// markers.
    fn mapping(&mut self, at: &str, value: &Value, field: Option<&str>, id: &str, roots: &[&str]) {
        let marker = match value {
            Value::Object(map) => MARKERS.into_iter().find(|m| map.contains_key(*m)),
            _ => None,
        };
        match value {
            Value::Object(map) if field.is_some() && marker.is_some() => {
                let (Some(field), Some(marker)) = (field, marker) else {
                    return;
                };
                let key = pointer(at, marker);
                let marker_args = |d: Diagnostic| {
                    d.arg("marker", Arg::Key(key.clone()))
                        .arg("field", Arg::computed(field.to_string()))
                };
                if map.len() != 1 || map[marker] != json!({}) {
                    self.push(
                        marker_args(Diagnostic::new("input.marker-shape", At::key(&key))).fix(
                            Fix::safe(vec![Edit::Set {
                                at: at.to_string(),
                                value: json!({ marker: {} }),
                            }]),
                        ),
                    );
                } else if !self.has_schemas(id) {
                    self.push(marker_args(Diagnostic::new(
                        "input.marker-no-schemas",
                        At::key(&key),
                    )));
                } else if let Some(copy) = self.copy(marker, id) {
                    // The copy takes the marker's place: its level, plus the copy's own.
                    let level = crate::diagnostic::steps(at).len() + 1;
                    let depth = level + runspore_schema::nesting(&copy);
                    if depth > runspore_types::canonical::MAX_DEPTH {
                        self.push(
                            marker_args(Diagnostic::new("input.marker-too-deep", At::key(&key)))
                                .arg("depth", Arg::computed(depth)),
                        );
                    }
                }
            }
            Value::Object(map) if map.contains_key(GET) => {
                for key in map.keys().filter(|k| *k != GET && *k != DEFAULT) {
                    let k = pointer(at, key);
                    self.push(
                        Diagnostic::new("mapping.get-extra-key", At::key(&k))
                            .arg("key", Arg::Key(k.clone()))
                            .fix(Fix::review(vec![Edit::Remove { at: k.clone() }])),
                    );
                }
                let path_at = pointer(at, GET);
                match &map[GET] {
                    Value::Array(path) if !path.is_empty() => {
                        let first = pointer(&path_at, "0");
                        match path[0].as_str() {
                            Some(root) if roots.contains(&root) => {}
                            other => {
                                let near = other.and_then(|r| closest(r, roots.iter().copied()));
                                let mut d = Diagnostic::new("mapping.path-root", At::value(&first))
                                    .arg("root", Arg::Input(first.clone()))
                                    .arg("roots", Arg::computed(roots.to_vec()));
                                if let Some(near) = near {
                                    d = d.fix(Fix::review(vec![Edit::Set {
                                        at: first.clone(),
                                        value: json!(near),
                                    }]));
                                }
                                self.push(d);
                            }
                        }
                        for (i, step) in path.iter().enumerate().skip(1) {
                            let ok = match step {
                                Value::String(_) => true,
                                Value::Number(n) => n.is_u64(),
                                _ => false,
                            };
                            if !ok {
                                let s = pointer(&path_at, &i.to_string());
                                self.push(
                                    Diagnostic::new("mapping.path-step", At::value(&s))
                                        .arg("value", Arg::Input(s.clone())),
                                );
                            }
                        }
                    }
                    _ => self.push(
                        Diagnostic::new("mapping.path-not-list", At::value(&path_at))
                            .arg("path", Arg::Input(path_at.clone()))
                            .arg("roots", Arg::computed(roots.to_vec())),
                    ),
                }
            }
            Value::Object(map) if map.contains_key(LITERAL) => {
                for key in map.keys().filter(|k| *k != LITERAL) {
                    let k = pointer(at, key);
                    self.push(
                        Diagnostic::new("mapping.literal-extra-key", At::key(&k))
                            .arg("key", Arg::Key(k.clone()))
                            .fix(Fix::review(vec![Edit::Remove { at: k.clone() }])),
                    );
                }
            }
            Value::Object(map) if map.keys().any(|k| operator(k).is_some()) => {
                let Some((name, operand)) = map.iter().find(|(k, _)| operator(k).is_some()) else {
                    return;
                };
                let op = pointer(at, name);
                for key in map.keys().filter(|k| *k != name) {
                    let k = pointer(at, key);
                    self.push(
                        Diagnostic::new("mapping.operator-extra-key", At::key(&k))
                            .arg("key", Arg::Key(k.clone()))
                            .arg("operator", Arg::Key(op.clone()))
                            .fix(Fix::review(vec![Edit::Remove { at: k.clone() }])),
                    );
                }
                let Some((_, count, shape)) = operator(name) else {
                    return;
                };
                let Some((count, exact)) = count else {
                    self.mapping(&op, operand, field, id, roots);
                    return;
                };
                match operand {
                    Value::Array(items)
                        if items.len() == count || (!exact && items.len() > count) =>
                    {
                        for (i, item) in items.iter().enumerate() {
                            self.mapping(&pointer(&op, &i.to_string()), item, field, id, roots);
                        }
                    }
                    _ => self.push(
                        Diagnostic::new("mapping.operator-shape", At::value(&op))
                            .arg("operator", Arg::Key(op.clone()))
                            .arg("operands", Arg::Input(op.clone()))
                            .arg("shape", Arg::Code(shape.to_string())),
                    ),
                }
            }
            Value::Object(map) => {
                for (key, member) in map {
                    let k = pointer(at, key);
                    if key.starts_with('$') {
                        let operators = [GET, LITERAL]
                            .into_iter()
                            .chain(OPERATORS.iter().map(|(name, ..)| *name))
                            .chain(MARKERS.into_iter().filter(|_| field.is_some()));
                        let mut d = Diagnostic::new("mapping.unknown-operator", At::key(&k))
                            .arg("key", Arg::Key(k.clone()));
                        if let Some(near) = closest(key, operators) {
                            d = d.fix(Fix::review(vec![Edit::Rename {
                                at: k.clone(),
                                to: near.to_string(),
                            }]));
                        }
                        self.push(d);
                    } else {
                        self.mapping(&k, member, field, id, roots);
                    }
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.mapping(&pointer(at, &i.to_string()), item, field, id, roots);
                }
            }
            _ => {}
        }
    }

    /// What a marker on step `id` is replaced with: a copy of its schemas, or the
    /// schema of a result-mode answer built from them.
    fn copy(&self, marker: &str, id: &str) -> Option<Value> {
        let schemas = self.schemas.get(id)?;
        if marker == MARKER {
            return Some(Value::Object(schemas.clone()));
        }
        let outcomes = self
            .value
            .pointer(&pointer(&pointer("/nodes", id), "action"))
            .and_then(Value::as_str)
            .and_then(|action| self.outcomes(action))
            .unwrap_or_else(|| schemas.keys().cloned().collect());
        Some(result_schema(&outcomes, schemas))
    }

    fn has_schemas(&self, id: &str) -> bool {
        self.value
            .pointer(&pointer(&pointer("/nodes", id), "outputSchemas"))
            .is_some_and(|s| s.as_object().is_some_and(|m| !m.is_empty()))
    }

    // ------------------------------------------------------------------
    // The whole graph
    // ------------------------------------------------------------------

    /// Kernel rules W14 and W15: every step is reached from the first, and every step
    /// reaches an end. A step that is not reached is reported once, as not reached.
    fn reachability(&mut self, graph: &Graph) {
        let Some((start, _)) = &graph.start else {
            return;
        };
        let targets = |id: &str| -> Vec<&str> {
            graph
                .edges
                .iter()
                .filter(|e| e.from == id)
                .map(|e| e.to.as_str())
                .collect()
        };
        let mut reached = BTreeSet::from([start.as_str()]);
        let mut frontier = vec![start.as_str()];
        while let Some(id) = frontier.pop() {
            for t in targets(id) {
                if reached.insert(t) {
                    frontier.push(t);
                }
            }
        }
        for id in &graph.order {
            if !reached.contains(id.as_str()) {
                self.push(
                    Diagnostic::new("dot.unreachable", graph.nodes[id].spot_at().of_step(id))
                        .arg("start", Arg::computed(start.clone())),
                );
            }
        }
        let mut ending: BTreeSet<&str> = graph
            .nodes
            .iter()
            .filter(|(_, n)| matches!(n.kind, "complete" | "fail"))
            .map(|(id, _)| id.as_str())
            .collect();
        loop {
            let before = ending.len();
            for e in &graph.edges {
                if ending.contains(e.to.as_str()) {
                    ending.insert(e.from.as_str());
                }
            }
            if ending.len() == before {
                break;
            }
        }
        for id in &graph.order {
            if reached.contains(id.as_str()) && !ending.contains(id.as_str()) {
                self.push(Diagnostic::new(
                    "dot.no-way-out",
                    graph.nodes[id].spot_at().of_step(id),
                ));
            }
        }
    }

    // ------------------------------------------------------------------
    // File references
    // ------------------------------------------------------------------

    fn reference(&mut self, at: &str, kind: EntryKind, owner: Option<Owner>) -> Option<PathBuf> {
        let reference = self.value.pointer(at)?.as_str()?.to_string();
        self.reference_path(at, &reference, kind, owner)
    }

    /// A file reference: a relative path to a regular file that stays inside the folder
    /// once `..` and symbolic links are resolved (spec/dot.md §4). The file joins the
    /// artifact as an entry of `kind` with `owner`.
    fn reference_path(
        &mut self,
        at: &str,
        reference: &str,
        kind: EntryKind,
        owner: Option<Owner>,
    ) -> Option<PathBuf> {
        let refuse = |code: &'static str| {
            Diagnostic::new(code, At::value(at)).arg("path", Arg::Input(at.to_string()))
        };
        if reference.is_empty() {
            self.push(refuse("file.missing"));
            return None;
        }
        if Path::new(reference).is_absolute() {
            let inside = std::fs::canonicalize(self.folder).ok().and_then(|f| {
                Path::new(reference)
                    .strip_prefix(f)
                    .ok()
                    .map(Path::to_path_buf)
            });
            let mut d = refuse("file.absolute");
            if let Some(relative) = inside.and_then(|p| p.to_str().map(String::from)) {
                d = d.fix(Fix::safe(vec![self.set_reference(at, &relative)]));
            }
            self.push(d);
            return None;
        }
        if reference.contains('\\') {
            let fixed = reference.replace('\\', "/");
            let d = refuse("file.backslash").fix(Fix::safe(vec![self.set_reference(at, &fixed)]));
            self.push(d);
            return None;
        }
        let Some(entry_path) = normalize(reference) else {
            self.push(refuse("file.outside"));
            return None;
        };
        if entry_path == DOT_FILE || entry_path == SETTINGS_FILE {
            self.push(refuse("file.fixed-name"));
            return None;
        }
        let folder = std::fs::canonicalize(self.folder).ok()?;
        let Ok(target) = std::fs::canonicalize(self.folder.join(reference)) else {
            let mut d = refuse("file.missing");
            let present = folder_files(&folder);
            if let Some(near) = closest(&entry_path, present.iter().map(String::as_str)) {
                d = d.fix(Fix::review(vec![self.set_reference(at, near)]));
            }
            self.push(d);
            return None;
        };
        if !target.starts_with(&folder) {
            self.push(refuse("file.outside"));
            return None;
        }
        if !target.is_file() {
            self.push(refuse("file.not-a-file"));
            return None;
        }
        let key = (entry_path, owner);
        let mut files = self.files.borrow_mut();
        if !files.contains_key(&key) {
            let bytes = std::fs::read(&target).ok()?;
            files.insert(
                key.clone(),
                File {
                    path: key.0.clone(),
                    kind,
                    owner: key.1.clone(),
                    executable: crate::executable(&target),
                    bytes,
                },
            );
        }
        Some(target)
    }

    fn set_reference(&self, at: &str, path: &str) -> Edit {
        let value = if at.contains("/async/") && !path.starts_with("./") {
            format!("./{path}")
        } else {
            path.to_string()
        };
        Edit::Set {
            at: at.to_string(),
            value: json!(value),
        }
    }

    // ------------------------------------------------------------------
    // The package
    // ------------------------------------------------------------------

    /// The package of spec/dot.md §5 step 6, for settings with no problems.
    pub fn package(&self, graph: &Graph) -> Value {
        let mut actions = self
            .value
            .get("actions")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for action in actions.values_mut() {
            if let Some(action) = action.as_object_mut() {
                action.remove("assets");
            }
        }
        let nodes = self.value.get("nodes").and_then(Value::as_object);
        let mut package_nodes = Map::new();
        for (id, node) in &graph.nodes {
            let mut fields = nodes
                .and_then(|n| n.get(id))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            fields.remove("instructions");
            // As written when the build could not check them (a draft with problems).
            let authored = fields.remove("outputSchemas");
            let schemas = self.schemas.get(id);
            let copies = Copies {
                output: self.copy(MARKER, id),
                result: self.copy(RESULT_MARKER, id),
            };
            for field in ["input", "params"] {
                if let Some(value) = fields.get_mut(field) {
                    markers(value, &copies);
                }
            }
            match (schemas, authored) {
                (Some(schemas), _) => {
                    fields.insert("outputSchemas".into(), Value::Object(schemas.clone()));
                }
                (None, Some(authored)) => {
                    fields.insert("outputSchemas".into(), authored);
                }
                (None, None) => {}
            }
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
            package_nodes.insert(id.clone(), Value::Object(fields));
        }
        let mut package = Map::new();
        package.insert("format".into(), Value::String(FORMAT.into()));
        package.insert("name".into(), Value::String(graph.name.clone()));
        let start = graph
            .start
            .as_ref()
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        package.insert("start".into(), Value::String(start));
        if let Some(limits) = self.value.get("limits") {
            package.insert("limits".into(), limits.clone());
        }
        package.insert("actions".into(), Value::Object(actions));
        package.insert("nodes".into(), Value::Object(package_nodes));
        // As written when the build could not check it (a draft with problems).
        if let Some(schema) = self
            .input_schema
            .as_ref()
            .or_else(|| self.value.get("inputSchema"))
        {
            package.insert("inputSchema".into(), schema.clone());
        }
        if let Some(hook) = self.value.get(ON_INTERVENTION) {
            package.insert(ON_INTERVENTION.into(), hook.clone());
        }
        Value::Object(package)
    }
}

/// The operator named `key` among [`OPERATORS`].
fn operator(key: &str) -> Option<Operator> {
    OPERATORS.into_iter().find(|(name, ..)| *name == key)
}

/// The schema of a result-mode answer (host.md §2) for a step that ends with
/// `outcomes`: an object `{"outcome", "output"}` whose outcome is one of `outcomes` and
/// whose output fits that outcome's schema, or anything when it has none. With several
/// outcomes the output is the `anyOf` of their schemas. The top level stays an object,
/// since model APIs refuse a top-level `anyOf`; the host checks the output against its
/// own outcome's schema after the answer.
fn result_schema(outcomes: &[String], schemas: &Map<String, Value>) -> Value {
    let mut outputs: Vec<Value> = outcomes
        .iter()
        .map(|outcome| schemas.get(outcome).cloned().unwrap_or_else(|| json!({})))
        .collect();
    let output = if outputs.len() == 1 {
        outputs.remove(0)
    } else {
        json!({ "anyOf": outputs })
    };
    json!({
        "type": "object",
        "properties": {
            "outcome": {"enum": outcomes},
            "output": output,
        },
        "required": ["outcome", "output"],
        "additionalProperties": false,
    })
}

/// What the markers of one step are replaced with, when it has schemas.
struct Copies {
    output: Option<Value>,
    result: Option<Value>,
}

/// Replaces each `{"$outputSchemas": {}}` and `{"$resultSchema": {}}` in an evaluated
/// position of a node's input or params with a `$literal` of its copy.
fn markers(value: &mut Value, copies: &Copies) {
    match value {
        Value::Object(map) if MARKERS.iter().any(|m| map.contains_key(*m)) => {
            let copy = if map.contains_key(MARKER) {
                &copies.output
            } else {
                &copies.result
            };
            if let Some(copy) = copy {
                *value = json!({ "$literal": copy.clone() });
            }
        }
        // The operands of `$if`, `$uuid`, `$json` and `$join` are evaluated.
        Value::Object(map) if map.len() == 1 && map.keys().any(|k| operator(k).is_some()) => {
            for operand in map.values_mut() {
                markers(operand, copies);
            }
        }
        // `$literal`, `$get` and any other operation: their contents are data.
        Value::Object(map) if map.keys().any(|k| k.starts_with('$')) => {}
        Value::Object(map) => {
            for item in map.values_mut() {
                markers(item, copies);
            }
        }
        Value::Array(items) => {
            for item in items {
                markers(item, copies);
            }
        }
        _ => {}
    }
}

fn severity(d: &Diagnostic) -> Severity {
    crate::catalogue::catalogue()
        .get(d.code)
        .map_or(Severity::Problem, |e| e.severity)
}

fn parent(at: &str) -> String {
    at.rfind('/').map_or(String::new(), |i| at[..i].to_string())
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// The regular files under `folder`, relative, for a missing file's closest name.
fn folder_files(folder: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(folder.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let rel = dir.join(entry.file_name());
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(rel),
                Ok(t) if t.is_file() => {
                    if let Some(s) = rel.to_str() {
                        if s != DOT_FILE && s != SETTINGS_FILE {
                            out.push(s.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// A folder-relative reference with empty and `.` segments dropped and `..` resolved
/// by name; `None` when a `..` climbs above the folder.
fn normalize(reference: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in reference.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            s => segments.push(s),
        }
    }
    let path = segments.join("/");
    artifact::check_path(&path).ok().map(|()| path)
}
