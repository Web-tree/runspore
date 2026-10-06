//! Replaying traces against a reducer: checking them, or recording what the
//! reducer does (blessing).
//!
//! The snapshot is threaded from step to step. A step that fails leaves it
//! unchanged, as a host that commits nothing would.

use std::convert::Infallible;
use std::fmt;
use std::path::{Path, PathBuf};

use runspore_types::canonical;
use runspore_types::reducer::{
    Decision, Envelope, Failure, Identity, Limits, Reducer, TransitionRequest,
};
use runspore_types::{digest, Value};

use crate::compile::{
    compile, CompiledEvent, CompiledExpect, CompiledIdentity, CompiledLimits, CompiledStep,
    CompiledTrace, Header, COMPILED_FORMAT,
};
use crate::diff;
use crate::error::Error;
use crate::format::{
    self, Expect, ExpectedCommand, ExpectedDecision, ExpectedDiagnostic, ExpectedFailure, Trace,
};

/// Environment variable that turns [`run_file`] and [`run_dir`] into blessing.
pub const BLESS_ENV: &str = "RUNSPORE_BLESS";

/// The first step a reducer answered differently from the trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub trace: String,
    /// Zero-based index into `steps`.
    pub step: usize,
    pub event_id: String,
    /// The parts that differ: `snapshot`, `snapshotDigest`, `commands`,
    /// `diagnostics`, `decisionDigest`, `failure` (kind or code), or `outcome`
    /// (a decision where a failure was expected, or the reverse).
    pub differing: Vec<String>,
    /// The `expect` block of the trace, pretty-printed.
    pub expected: String,
    /// What the reducer returned, in the same form.
    pub actual: String,
    /// Line diff from `expected` to `actual`.
    pub diff: String,
    /// What the diff cannot show, such as the text of an unexpected failure.
    pub notes: Vec<String>,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "trace `{}`, step {} (event `{}`): {} differ{}",
            self.trace,
            self.step,
            self.event_id,
            self.differing.join(", "),
            if self.differing.len() == 1 { "s" } else { "" }
        )?;
        writeln!(f, "--- expected\n+++ actual")?;
        f.write_str(&self.diff)?;
        for note in &self.notes {
            writeln!(f, "note: {note}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Mismatch {}

/// Replays `trace` and returns the first mismatch.
///
/// Compared per step: snapshot bytes, `snapshotDigest` (also against
/// `digest::state` of the returned snapshot), every command and diagnostic in
/// order, and `decisionDigest` against `digest::decision` of the returned
/// decision; or, for a failure step, the failure kind and code.
pub fn run(reducer: &dyn Reducer, trace: &Trace) -> Result<(), Error> {
    run_compiled(reducer, &compile(trace)?)
}

/// [`run`] for a trace that is already compiled. This is the reference for the
/// replay loop documented in [`crate::compile`](mod@crate::compile).
pub fn run_compiled(reducer: &dyn Reducer, trace: &CompiledTrace) -> Result<(), Error> {
    if trace.format != COMPILED_FORMAT {
        return Err(Error::Format {
            trace: trace.name.clone(),
            found: trace.format.clone(),
            expected: COMPILED_FORMAT,
        });
    }
    let mut session = Session::new(reducer, &trace.identity, trace.limits, &trace.workflow);
    for (index, step) in trace.steps.iter().enumerate() {
        let outcome = session.apply(
            &step.event,
            step.identity.as_ref(),
            step.snapshot.as_deref(),
        );
        let differing = compare(&step.expect, &outcome);
        if !differing.is_empty() {
            return Err(mismatch(&trace.name, index, step, &differing, &outcome).into());
        }
    }
    Ok(())
}

/// Replays `trace` and returns it with every `expect` block replaced by what
/// the reducer returned. Nothing else changes.
pub fn bless(reducer: &dyn Reducer, trace: &Trace) -> Result<Trace, Error> {
    let header = Header::of(trace)?;
    let mut session = Session::new(reducer, &header.identity, header.limits, &header.workflow);
    let mut blessed = trace.clone();
    for (index, step) in blessed.steps.iter_mut().enumerate() {
        let request = header.step(index, step)?;
        let outcome = session.apply(
            &request.event,
            request.identity.as_ref(),
            request.snapshot.as_deref(),
        );
        step.expect = Some(record(&trace.name, index, &step.event.event_id, &outcome)?);
    }
    Ok(blessed)
}

/// Blesses a trace file in place and returns whether it was rewritten. A file
/// whose expectations already hold is left untouched, whatever its layout; a
/// rewritten file is in the layout of [`Trace::to_json`].
pub fn bless_file(reducer: &dyn Reducer, path: impl AsRef<Path>) -> Result<bool, Error> {
    let path = path.as_ref();
    let trace = format::load(path)?;
    let blessed = bless(reducer, &trace)?;
    if blessed == trace {
        return Ok(false);
    }
    format::save(path, &blessed)?;
    Ok(true)
}

/// Whether `RUNSPORE_BLESS=1` is set.
pub fn bless_requested() -> bool {
    std::env::var_os(BLESS_ENV).is_some_and(|value| value == "1")
}

/// Checks one trace file, or blesses it when [`bless_requested`].
pub fn run_file(reducer: &dyn Reducer, path: impl AsRef<Path>) -> Result<(), Error> {
    if bless_requested() {
        bless_file(reducer, path).map(|_| ())
    } else {
        run(reducer, &format::load(path)?)
    }
}

/// Every trace file of a directory that failed, in file name order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirError {
    pub failures: Vec<(PathBuf, Error)>,
}

impl fmt::Display for DirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (path, error) in &self.failures {
            writeln!(f, "{}: {error}", path.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for DirError {}

/// Applies [`run_file`] to every `*.json` file directly inside `dir`, in file
/// name order, without stopping at the first failure. Returns how many files
/// there were. An empty directory is `Ok(0)`: assert on the count.
pub fn run_dir(reducer: &dyn Reducer, dir: impl AsRef<Path>) -> Result<usize, DirError> {
    each_trace_file(dir.as_ref(), |path| run_file(reducer, path))
}

fn each_trace_file(
    dir: &Path,
    visit: impl Fn(&Path) -> Result<(), Error>,
) -> Result<usize, DirError> {
    let io = |e: std::io::Error| DirError {
        failures: vec![(
            dir.to_path_buf(),
            Error::Io {
                path: dir.to_path_buf(),
                message: e.to_string(),
            },
        )],
    };
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.extension().is_some_and(|ext| ext == "json") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    let failures: Vec<(PathBuf, Error)> = paths
        .iter()
        .filter_map(|path| visit(path).err().map(|error| (path.clone(), error)))
        .collect();
    if failures.is_empty() {
        Ok(paths.len())
    } else {
        Err(DirError { failures })
    }
}

/// One replay: builds each request and threads the snapshot.
pub(crate) struct Session<'a> {
    reducer: &'a dyn Reducer,
    identity: Identity,
    graph: Vec<u8>,
    limits: Limits,
    snapshot: Option<Vec<u8>>,
}

impl<'a> Session<'a> {
    pub(crate) fn new(
        reducer: &'a dyn Reducer,
        identity: &CompiledIdentity,
        limits: CompiledLimits,
        workflow: &str,
    ) -> Self {
        Session {
            reducer,
            identity: identity.into(),
            graph: workflow.as_bytes().to_vec(),
            limits: limits.into(),
            snapshot: None,
        }
    }

    /// Sends one event. `identity` and `snapshot` replace the session's own for
    /// this request only.
    pub(crate) fn apply(
        &mut self,
        event: &CompiledEvent,
        identity: Option<&CompiledIdentity>,
        snapshot: Option<&str>,
    ) -> Result<Decision, Failure> {
        let request = TransitionRequest {
            identity: identity.map_or_else(|| self.identity.clone(), Identity::from),
            graph: self.graph.clone(),
            snapshot: match snapshot {
                Some(raw) => Some(raw.as_bytes().to_vec()),
                None => self.snapshot.clone(),
            },
            input_event: Envelope {
                event_id: event.event_id.clone(),
                sequence: event.sequence,
                accepted_at_ms: event.accepted_at_ms,
                kind: event.kind.clone(),
                payload: event.payload.as_bytes().to_vec(),
            },
            frozen_limits: self.limits,
        };
        let outcome = self.reducer.transition(&request);
        if let Ok(decision) = &outcome {
            self.snapshot = Some(decision.snapshot.clone());
        }
        outcome
    }
}

/// The `expect` block that records `outcome`. Fails if the reducer returned
/// bytes that are not canonical JSON.
pub(crate) fn record(
    trace: &str,
    step: usize,
    event_id: &str,
    outcome: &Result<Decision, Failure>,
) -> Result<Expect, Error> {
    observe(outcome, |location, bytes| {
        canonical::parse_canonical(bytes).map_err(|source| Error::Output {
            trace: trace.to_string(),
            step,
            event_id: event_id.to_string(),
            location,
            source,
        })
    })
}

fn observe<E>(
    outcome: &Result<Decision, Failure>,
    parse: impl Fn(String, &[u8]) -> Result<Value, E>,
) -> Result<Expect, E> {
    let decision = match outcome {
        Ok(decision) => decision,
        Err(failure) => {
            return Ok(Expect::Failure(ExpectedFailure {
                kind: failure.kind,
                code: failure.code.clone(),
            }))
        }
    };
    let mut commands = Vec::with_capacity(decision.commands.len());
    for (n, command) in decision.commands.iter().enumerate() {
        commands.push(ExpectedCommand {
            command_id: command.command_id.clone(),
            activation_id: command.activation_id.clone(),
            kind: command.kind.clone(),
            payload: parse(format!("commands[{n}].payload"), &command.payload)?,
        });
    }
    let mut diagnostics = Vec::with_capacity(decision.diagnostics.len());
    for (n, diagnostic) in decision.diagnostics.iter().enumerate() {
        diagnostics.push(ExpectedDiagnostic {
            code: diagnostic.code.clone(),
            node_id: diagnostic.node_id.clone(),
            details: parse(format!("diagnostics[{n}].details"), &diagnostic.details)?,
        });
    }
    Ok(Expect::Decision(ExpectedDecision {
        snapshot: parse("snapshot".to_string(), &decision.snapshot)?,
        snapshot_digest: decision.snapshot_digest.clone(),
        commands,
        diagnostics,
        decision_digest: digest::decision(decision),
    }))
}

fn compare(expect: &CompiledExpect, outcome: &Result<Decision, Failure>) -> Vec<&'static str> {
    let mut differing = Vec::new();
    match (expect, outcome) {
        (CompiledExpect::Failure(expected), Err(actual)) => {
            if expected.failure.kind != actual.kind || expected.failure.code != actual.code {
                differing.push("failure");
            }
        }
        (CompiledExpect::Decision(expected), Ok(actual)) => {
            if expected.snapshot.as_bytes() != actual.snapshot {
                differing.push("snapshot");
            }
            if expected.snapshot_digest != actual.snapshot_digest
                || actual.snapshot_digest != digest::state(&actual.snapshot)
            {
                differing.push("snapshotDigest");
            }
            let commands = expected.commands.len() == actual.commands.len()
                && expected
                    .commands
                    .iter()
                    .zip(&actual.commands)
                    .all(|(e, a)| {
                        e.command_id == a.command_id
                            && e.activation_id == a.activation_id
                            && e.kind == a.kind
                            && e.payload.as_bytes() == a.payload
                    });
            if !commands {
                differing.push("commands");
            }
            let diagnostics = expected.diagnostics.len() == actual.diagnostics.len()
                && expected
                    .diagnostics
                    .iter()
                    .zip(&actual.diagnostics)
                    .all(|(e, a)| {
                        e.code == a.code
                            && e.node_id == a.node_id
                            && e.details.as_bytes() == a.details
                    });
            if !diagnostics {
                differing.push("diagnostics");
            }
            if expected.decision_digest != digest::decision(actual) {
                differing.push("decisionDigest");
            }
        }
        _ => differing.push("outcome"),
    }
    differing
}

fn mismatch(
    trace: &str,
    index: usize,
    step: &CompiledStep,
    differing: &[&str],
    outcome: &Result<Decision, Failure>,
) -> Mismatch {
    let expected = expected_view(&step.expect).doc().render();
    let actual = match observe(outcome, |_, bytes| Ok::<_, Infallible>(lenient(bytes))) {
        Ok(view) => view.doc().render(),
        Err(never) => match never {},
    };
    let diff = diff::lines(&expected, &actual);
    let mut notes = Vec::new();
    match outcome {
        Err(failure) => notes.push(format!("failure details: {}", failure.details)),
        Ok(decision) if decision.snapshot_digest != digest::state(&decision.snapshot) => notes
            .push(
                "the returned snapshotDigest is not digest::state of the returned snapshot"
                    .to_string(),
            ),
        Ok(_) => {}
    }
    if diff.is_empty() {
        notes.push(
            "expected and actual print the same, so the returned bytes differ only in \
             encoding: they are not canonical JSON"
                .to_string(),
        );
    }
    Mismatch {
        trace: trace.to_string(),
        step: index,
        event_id: step.event.event_id.clone(),
        differing: differing.iter().map(|part| part.to_string()).collect(),
        expected,
        actual,
        diff,
        notes,
    }
}

/// The expectation of a compiled step, as plain JSON again for printing.
fn expected_view(expect: &CompiledExpect) -> Expect {
    match expect {
        CompiledExpect::Failure(expected) => Expect::Failure(expected.failure.clone()),
        CompiledExpect::Decision(expected) => Expect::Decision(ExpectedDecision {
            snapshot: lenient(expected.snapshot.as_bytes()),
            snapshot_digest: expected.snapshot_digest.clone(),
            commands: expected
                .commands
                .iter()
                .map(|command| ExpectedCommand {
                    command_id: command.command_id.clone(),
                    activation_id: command.activation_id.clone(),
                    kind: command.kind.clone(),
                    payload: lenient(command.payload.as_bytes()),
                })
                .collect(),
            diagnostics: expected
                .diagnostics
                .iter()
                .map(|diagnostic| ExpectedDiagnostic {
                    code: diagnostic.code.clone(),
                    node_id: diagnostic.node_id.clone(),
                    details: lenient(diagnostic.details.as_bytes()),
                })
                .collect(),
            decision_digest: expected.decision_digest.clone(),
        }),
    }
}

/// Bytes as JSON for printing; bytes that are not JSON are shown as text.
fn lenient(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(format!("<not JSON> {}", String::from_utf8_lossy(bytes))))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use runspore_types::model::{code, CODEC_VERSION, SEMANTICS_VERSION};
    use runspore_types::reducer::{Diagnostic, FailureKind};
    use serde_json::json;

    use super::*;
    use crate::format::{Event, IdentityOverride, LimitsOverride, Payload, Step, KERNEL_DIGEST};
    use crate::testing::{self, Altered, Toy};

    fn blessed() -> Trace {
        bless(&Toy, &testing::unblessed()).unwrap()
    }

    fn decision(trace: &Trace, step: usize) -> &ExpectedDecision {
        match &trace.steps[step].expect {
            Some(Expect::Decision(decision)) => decision,
            other => panic!("step {step} is not a decision: {other:?}"),
        }
    }

    fn failure(trace: &Trace, step: usize) -> (FailureKind, &str) {
        match &trace.steps[step].expect {
            Some(Expect::Failure(failure)) => (failure.kind, &failure.code),
            other => panic!("step {step} is not a failure: {other:?}"),
        }
    }

    /// Every request a replay of `trace` sends.
    fn requests(trace: &Trace) -> Vec<TransitionRequest> {
        let seen = Mutex::new(Vec::new());
        let recorder = Altered(
            |request: &TransitionRequest, _: &mut Result<Decision, Failure>| {
                seen.lock().unwrap().push(request.clone());
            },
        );
        bless(&recorder, trace).unwrap();
        seen.into_inner().unwrap()
    }

    fn mismatch(result: Result<(), Error>) -> Mismatch {
        match result {
            Err(Error::Mismatch(mismatch)) => *mismatch,
            other => panic!("expected a mismatch: {other:?}"),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("runspore-trace-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bless_records_decisions_and_failures_and_run_accepts_them() {
        let trace = blessed();
        run(&Toy, &trace).unwrap();

        let statuses: Vec<&str> = [0, 1, 2, 4, 5, 6]
            .iter()
            .map(|&step| decision(&trace, step).snapshot["status"].as_str().unwrap())
            .collect();
        assert_eq!(
            statuses,
            [
                "running",
                "waiting",
                "running",
                "needs-intervention",
                "completed",
                "completed"
            ]
        );
        assert_eq!(
            failure(&trace, 3),
            (FailureKind::InvalidInput, code::EVENT_UNKNOWN_KIND)
        );

        let start = decision(&trace, 0);
        assert_eq!(start.commands.len(), 1);
        assert_eq!(start.commands[0].activation_id, "build/1");
        assert_eq!(
            start.commands[0].payload["invocationId"],
            json!(testing::invocation("build/1"))
        );
        assert!(start.diagnostics.is_empty());
        assert_eq!(
            decision(&trace, 4).diagnostics[0].code,
            code::INVOCATION_UNKNOWN
        );
        assert_eq!(
            decision(&trace, 6).diagnostics[0].details,
            json!({"kind": "signal.received"})
        );
        assert!(start.snapshot_digest.starts_with("sha256:"));
        assert_ne!(start.decision_digest, start.snapshot_digest);
    }

    #[test]
    fn bless_changes_nothing_but_expectations() {
        let unblessed = testing::unblessed();
        let mut stripped = blessed();
        for step in &mut stripped.steps {
            step.expect = None;
        }
        assert_eq!(stripped, unblessed);
        assert_eq!(bless(&Toy, &blessed()).unwrap(), blessed());
    }

    #[test]
    fn requests_carry_the_fixed_identity_and_the_threaded_snapshot() {
        let trace = blessed();
        let requests = requests(&trace);
        let graph = canonical::to_vec(&trace.workflow).unwrap();
        assert_eq!(requests.len(), trace.steps.len());
        for request in &requests {
            assert_eq!(request.graph, graph);
            assert_eq!(
                request.identity,
                Identity {
                    package_digest: digest::package(&graph),
                    kernel_digest: KERNEL_DIGEST.to_string(),
                    semantics_version: SEMANTICS_VERSION.to_string(),
                    codec_version: CODEC_VERSION.to_string(),
                }
            );
            assert_eq!(request.frozen_limits, Limits::default());
        }
        assert_eq!(
            requests[2].input_event,
            Envelope {
                event_id: "approved".to_string(),
                sequence: 3,
                accepted_at_ms: 2_000,
                kind: "signal.received".to_string(),
                payload: br#"{"name":"approval","outcome":"approved"}"#.to_vec(),
            }
        );

        let snapshot_after =
            |step: usize| Some(canonical::to_vec(&decision(&trace, step).snapshot).unwrap());
        assert_eq!(requests[0].snapshot, None);
        assert_eq!(requests[1].snapshot, snapshot_after(0));
        assert_eq!(requests[3].snapshot, snapshot_after(2));
        // Step 3 was refused, so step 4 starts from the same snapshot.
        assert_eq!(requests[4].snapshot, snapshot_after(2));
        assert_eq!(requests[5].snapshot, snapshot_after(4));
    }

    #[test]
    fn limits_identity_and_raw_members_reach_the_request() {
        let mut trace = Trace::new("extensions", "", testing::workflow());
        trace.limits = Some(LimitsOverride {
            max_state_bytes: Some(100),
            ..LimitsOverride::default()
        });
        let start = json!({"tenant": "default", "runId": "run_toy", "input": null});
        let mut versioned = Step::new(Event::new("v", 1, 1, "run.started", start.clone()));
        versioned.identity = Some(IdentityOverride {
            semantics_version: Some("9.9".to_string()),
            kernel_digest: Some("other".to_string()),
            ..IdentityOverride::default()
        });
        let mut corrupt = Step::new(Event::raw(
            "c",
            2,
            2,
            "signal.received",
            r#"{"name":"approval"}"#,
        ));
        corrupt.snapshot_raw = Some("{}".to_string());
        trace.steps = vec![
            versioned,
            corrupt,
            Step::new(Event::raw(
                "spaced",
                1,
                3,
                "run.started",
                r#"{ "tenant": 1 }"#,
            )),
            Step::new(Event::new("big", 1, 4, "run.started", start)),
        ];

        let trace = bless(&Toy, &trace).unwrap();
        let codes: Vec<&str> = (0..4).map(|step| failure(&trace, step).1).collect();
        assert_eq!(
            codes,
            [
                code::VERSION_SEMANTICS,
                code::STATE_INVALID,
                code::EVENT_INVALID,
                code::BUDGET_STATE_BYTES
            ]
        );
        run(&Toy, &trace).unwrap();

        let requests = requests(&trace);
        assert_eq!(requests[0].identity.semantics_version, "9.9");
        assert_eq!(requests[0].identity.kernel_digest, "other");
        assert_eq!(
            requests[0].identity.package_digest,
            requests[1].identity.package_digest
        );
        assert_eq!(requests[1].identity.semantics_version, SEMANTICS_VERSION);
        assert_eq!(requests[1].snapshot.as_deref(), Some(&b"{}"[..]));
        assert_eq!(requests[2].snapshot, None);
        assert_eq!(requests[2].input_event.payload, br#"{ "tenant": 1 }"#);
        assert_eq!(requests[3].frozen_limits.max_state_bytes, 100);
        assert_eq!(
            requests[3].frozen_limits.microsteps,
            Limits::default().microsteps
        );
    }

    #[test]
    fn mismatch_names_the_step_and_shows_a_line_diff() {
        let trace = blessed();
        let skewed = Altered(
            |request: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if request.input_event.event_id == "approved" {
                    outcome.as_mut().unwrap().commands[0].activation_id = "ship/9".to_string();
                }
            },
        );
        let mismatch = mismatch(run(&skewed, &trace));
        assert_eq!(mismatch.trace, "toy-run");
        assert_eq!(mismatch.step, 2);
        assert_eq!(mismatch.event_id, "approved");
        assert_eq!(mismatch.differing, ["commands", "decisionDigest"]);
        assert_eq!(mismatch.notes, Vec::<String>::new());
        assert_eq!(mismatch.expected, decision_text(&trace, 2));

        let removed: Vec<&str> = mismatch
            .diff
            .lines()
            .filter(|line| line.starts_with("- "))
            .collect();
        let added: Vec<&str> = mismatch
            .diff
            .lines()
            .filter(|line| line.starts_with("+ "))
            .collect();
        assert_eq!(removed.len(), 2, "{}", mismatch.diff);
        assert_eq!(
            removed[0].trim_start_matches("- ").trim(),
            r#""activationId": "ship/1","#
        );
        assert_eq!(
            added[0].trim_start_matches("+ ").trim(),
            r#""activationId": "ship/9","#
        );
        assert!(removed[1].contains("decisionDigest") && added[1].contains("decisionDigest"));
        assert!(mismatch.diff.contains("  ...\n"), "{}", mismatch.diff);

        let report = mismatch.to_string();
        assert!(
            report.starts_with(
                "trace `toy-run`, step 2 (event `approved`): commands, decisionDigest differ\n\
                 --- expected\n+++ actual\n"
            ),
            "{report}"
        );
        assert!(report.ends_with(&mismatch.diff));
    }

    fn decision_text(trace: &Trace, step: usize) -> String {
        Expect::Decision(decision(trace, step).clone())
            .doc()
            .render()
    }

    #[test]
    fn every_compared_part_is_reported() {
        let trace = blessed();
        let differing = |alter: fn(&mut Decision)| {
            let skewed = Altered(
                move |request: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                    if request.input_event.sequence == 1 {
                        alter(outcome.as_mut().unwrap());
                    }
                },
            );
            let mismatch = mismatch(run(&skewed, &trace));
            assert_eq!(mismatch.step, 0);
            (mismatch.differing.join(","), mismatch.notes.len())
        };
        assert_eq!(
            differing(|decision| {
                decision.snapshot = br#"{"status":"running"}"#.to_vec();
                decision.snapshot_digest = digest::state(&decision.snapshot);
            }),
            ("snapshot,snapshotDigest,decisionDigest".to_string(), 0)
        );
        assert_eq!(
            differing(|decision| decision.snapshot_digest = "sha256:00".to_string()),
            ("snapshotDigest,decisionDigest".to_string(), 1)
        );
        assert_eq!(
            differing(|decision| decision.commands[0].payload = b"{}".to_vec()),
            ("commands,decisionDigest".to_string(), 0)
        );
        assert_eq!(
            differing(|decision| decision.commands.clear()),
            ("commands,decisionDigest".to_string(), 0)
        );
        assert_eq!(
            differing(|decision| decision.diagnostics.push(Diagnostic {
                code: "extra".to_string(),
                node_id: Some("build".to_string()),
                details: b"null".to_vec(),
            })),
            ("diagnostics,decisionDigest".to_string(), 0)
        );
    }

    #[test]
    fn bytes_that_only_print_the_same_are_still_a_mismatch() {
        let trace = blessed();
        let spaced = Altered(
            |request: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if request.input_event.sequence == 1 {
                    let decision = outcome.as_mut().unwrap();
                    let value: Value = serde_json::from_slice(&decision.snapshot).unwrap();
                    decision.snapshot = serde_json::to_vec_pretty(&value).unwrap();
                }
            },
        );
        let mismatch = mismatch(run(&spaced, &trace));
        assert_eq!(mismatch.differing, ["snapshot", "snapshotDigest"]);
        assert_eq!(mismatch.diff, "");
        assert_eq!(mismatch.expected, mismatch.actual);
        assert_eq!(mismatch.notes.len(), 2, "{:?}", mismatch.notes);
        assert!(mismatch.notes[1].contains("not canonical JSON"));

        let garbage = Altered(
            |_: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if let Ok(decision) = outcome {
                    decision.snapshot = b"\xff not json".to_vec();
                }
            },
        );
        let mismatch = self::mismatch(run(&garbage, &trace));
        assert!(
            mismatch.actual.contains("<not JSON>"),
            "{}",
            mismatch.actual
        );
        assert!(matches!(
            bless(&garbage, &trace),
            Err(Error::Output { step: 0, location, .. }) if location == "snapshot"
        ));
    }

    #[test]
    fn failures_are_compared_by_kind_and_code_only() {
        let trace = blessed();
        let reworded = Altered(
            |_: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if let Err(failure) = outcome {
                    failure.details = "different words".to_string();
                }
            },
        );
        run(&reworded, &trace).unwrap();

        let recoded = Altered(
            |_: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if let Err(failure) = outcome {
                    failure.code = code::EVENT_INVALID.to_string();
                    failure.details = "why it failed".to_string();
                }
            },
        );
        let mismatch = mismatch(run(&recoded, &trace));
        assert_eq!((mismatch.step, mismatch.event_id.as_str()), (3, "timer"));
        assert_eq!(mismatch.differing, ["failure"]);
        assert_eq!(
            mismatch.diff,
            "- {\"failure\": {\"kind\": \"invalid-input\", \"code\": \"event.unknown-kind\"}}\n\
             + {\"failure\": {\"kind\": \"invalid-input\", \"code\": \"event.invalid\"}}\n"
        );
        assert_eq!(mismatch.notes, ["failure details: why it failed"]);
    }

    #[test]
    fn a_decision_in_place_of_a_failure_and_the_reverse_are_mismatches() {
        let trace = blessed();
        let refusing = Altered(
            |request: &TransitionRequest, outcome: &mut Result<Decision, Failure>| {
                if request.input_event.sequence == 2 {
                    *outcome = Err(Failure {
                        kind: FailureKind::InvariantViolation,
                        code: "host.trap".to_string(),
                        details: "boom".to_string(),
                    });
                }
            },
        );
        let mismatch = mismatch(run(&refusing, &trace));
        assert_eq!(
            (mismatch.step, mismatch.differing.join(",")),
            (1, "outcome".to_string())
        );
        assert!(mismatch
            .diff
            .contains("+ {\"failure\": {\"kind\": \"invariant-violation\""));
        assert_eq!(mismatch.notes, ["failure details: boom"]);

        let mut wrong = trace.clone();
        wrong.steps[3].expect = wrong.steps[2].expect.clone();
        let mismatch = self::mismatch(run(&Toy, &wrong));
        assert_eq!(
            (mismatch.step, mismatch.differing.join(",")),
            (3, "outcome".to_string())
        );
    }

    #[test]
    fn traces_that_cannot_be_replayed_are_errors_not_mismatches() {
        assert!(matches!(
            run(&Toy, &testing::unblessed()),
            Err(Error::Unblessed { step: 0, event_id, .. }) if event_id == "start"
        ));

        let mut fractional = blessed();
        fractional.steps[1].event.payload = Payload::Json(json!({"ratio": 1.5}));
        for result in [run(&Toy, &fractional), bless(&Toy, &fractional).map(|_| ())] {
            assert!(matches!(
                result,
                Err(Error::Fixture { location, .. }) if location == "steps[1].event.payload"
            ));
        }

        let mut renamed = blessed();
        renamed.format = "runspore.trace/9".to_string();
        assert!(matches!(run(&Toy, &renamed), Err(Error::Format { .. })));
        let mut compiled = compile(&blessed()).unwrap();
        compiled.format = "other".to_string();
        assert!(matches!(
            run_compiled(&Toy, &compiled),
            Err(Error::Format { .. })
        ));
    }

    #[test]
    fn bless_file_rewrites_only_when_expectations_change() {
        let dir = scratch("bless");
        let path = dir.join("toy.json");
        format::save(&path, &testing::unblessed()).unwrap();
        assert!(bless_file(&Toy, &path).unwrap());
        assert_eq!(format::load(&path).unwrap(), blessed());

        let hand_written = serde_json::to_string(
            &serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap(),
        )
        .unwrap();
        std::fs::write(&path, &hand_written).unwrap();
        assert!(!bless_file(&Toy, &path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), hand_written);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directories_are_run_in_name_order_and_report_every_failing_file() {
        let dir = scratch("dir");
        let good = blessed();
        let mut bad = good.clone();
        bad.steps[1].expect = bad.steps[0].expect.clone();
        format::save(dir.join("b-good.json"), &good).unwrap();
        format::save(dir.join("c-bad.json"), &bad).unwrap();
        format::save(dir.join("a-bad.json"), &bad).unwrap();
        std::fs::write(dir.join("d-broken.json"), "{").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a trace").unwrap();
        std::fs::create_dir(dir.join("nested.json")).unwrap();

        let check = |path: &Path| run(&Toy, &format::load(path)?);
        let error = each_trace_file(&dir, check).unwrap_err();
        let names: Vec<&str> = error
            .failures
            .iter()
            .map(|(path, _)| path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["a-bad.json", "c-bad.json", "d-broken.json"]);
        assert!(matches!(&error.failures[0].1, Error::Mismatch(m) if m.step == 1));
        assert!(matches!(error.failures[2].1, Error::Syntax { .. }));
        assert_eq!(
            error
                .to_string()
                .lines()
                .filter(|l| l.contains("-bad.json: trace"))
                .count(),
            2
        );

        for name in ["a-bad.json", "c-bad.json", "d-broken.json"] {
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        assert_eq!(each_trace_file(&dir, check), Ok(1));
        assert_eq!(run_dir(&Toy, &dir), Ok(1));
        run_file(&Toy, dir.join("b-good.json")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(
            &each_trace_file(&dir, check).unwrap_err().failures[..],
            [(_, Error::Io { .. })]
        ));
    }
}
