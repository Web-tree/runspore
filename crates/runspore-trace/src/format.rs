//! The `runspore.trace/0.1` document (`spec/kernel.md` section 9).
//!
//! ```json
//! {
//!   "format": "runspore.trace/0.1",
//!   "name": "retry-then-success",
//!   "description": "one line",
//!   "workflow": { },
//!   "limits": {"microsteps": 1000},
//!   "steps": [
//!     {"event": {"eventId": "start", "sequence": "1", "acceptedAtMs": "1000",
//!                "kind": "run.started", "payload": { }},
//!      "expect": {"snapshot": { }, "snapshotDigest": "sha256:…",
//!                 "commands": [{"commandId": "…", "activationId": "…", "kind": "…", "payload": { }}],
//!                 "diagnostics": [{"code": "…", "nodeId": null, "details": { }}],
//!                 "decisionDigest": "sha256:…"}},
//!     {"event": { }, "expect": {"failure": {"kind": "resource-limit", "code": "budget.microsteps"}}}
//!   ]
//! }
//! ```
//!
//! `workflow`, payloads, snapshots, and details are plain JSON: key order and
//! whitespace are free, and a runner canonicalizes them before use. Members are
//! written in the order shown above.
//!
//! # What the runner supplies
//!
//! A trace says nothing about the request identity, so every runner uses the
//! same fixed values. A request is a pure function of the trace.
//!
//! | Request field | Value |
//! | --- | --- |
//! | `identity.packageDigest` | `digest::package` of the canonical workflow |
//! | `identity.kernelDigest` | [`KERNEL_DIGEST`], whatever reducer is under test |
//! | `identity.semanticsVersion` | `model::SEMANTICS_VERSION` |
//! | `identity.codecVersion` | `model::CODEC_VERSION` |
//! | `graph` | the canonical workflow |
//! | `snapshot` | absent for the first step, then the snapshot of the last step that produced a decision |
//! | `frozenLimits` | `Limits::default()` with the trace's `limits` fields replaced |
//!
//! Tenant and run ID are not runner defaults: they are whatever the trace's
//! `run.started` payload says.
//!
//! # Extensions to section 9
//!
//! Two optional step members let a trace reach failures that the threaded
//! request cannot produce. Both are absent from ordinary traces.
//!
//! - `identity`: replaces individual identity fields for this step only
//!   (`version.semantics`, `version.codec`).
//! - `snapshotRaw`: bytes sent as the snapshot for this step instead of the
//!   threaded one (`state.invalid`, `version.state-format`). If the step still
//!   produces a decision, that decision's snapshot is threaded on as usual.
//!
//! A step may also omit `expect` while a trace is being written; such a trace
//! cannot be run, only blessed.

use std::path::Path;

use runspore_types::model::u64_str;
use runspore_types::reducer::{FailureKind, Limits};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::pretty::Doc;

pub const TRACE_FORMAT: &str = "runspore.trace/0.1";

/// `Identity.kernel_digest` of every trace request. Fixed, so the same trace
/// produces the same request bytes on every host.
pub const KERNEL_DIGEST: &str = "trace:runspore.trace/0.1";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trace {
    pub format: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The workflow document as plain JSON. Not checked here: a trace may hold an
    /// invalid workflow to pin the failure it causes.
    pub workflow: Value,
    #[serde(default)]
    pub limits: Option<LimitsOverride>,
    pub steps: Vec<Step>,
}

/// Replaces individual fields of `Limits::default()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LimitsOverride {
    #[serde(default)]
    pub microsteps: Option<u32>,
    #[serde(default)]
    pub expression_operations: Option<u32>,
    #[serde(default)]
    pub max_state_bytes: Option<u32>,
    #[serde(default)]
    pub max_command_count: Option<u32>,
}

impl LimitsOverride {
    pub fn resolve(&self) -> Limits {
        let default = Limits::default();
        Limits {
            microsteps: self.microsteps.unwrap_or(default.microsteps),
            expression_operations: self
                .expression_operations
                .unwrap_or(default.expression_operations),
            max_state_bytes: self.max_state_bytes.unwrap_or(default.max_state_bytes),
            max_command_count: self.max_command_count.unwrap_or(default.max_command_count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub event: Event,
    /// Extension: identity fields replaced for this step only.
    #[serde(default)]
    pub identity: Option<IdentityOverride>,
    /// Extension: bytes sent as the snapshot for this step instead of the threaded one.
    #[serde(default)]
    pub snapshot_raw: Option<String>,
    /// Absent only while the trace is being written.
    #[serde(default)]
    pub expect: Option<Expect>,
}

/// Replaces individual fields of the request identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityOverride {
    #[serde(default)]
    pub package_digest: Option<String>,
    #[serde(default)]
    pub kernel_digest: Option<String>,
    #[serde(default)]
    pub semantics_version: Option<String>,
    #[serde(default)]
    pub codec_version: Option<String>,
}

/// One inbox event. `sequence` and `acceptedAtMs` are decimal strings in the file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "EventRepr")]
pub struct Event {
    pub event_id: String,
    pub sequence: u64,
    pub accepted_at_ms: u64,
    pub kind: String,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// `payload`: plain JSON, canonicalized by the runner.
    Json(Value),
    /// `payloadRaw`: the exact bytes to send, canonical or not.
    Raw(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventRepr {
    event_id: String,
    #[serde(with = "u64_str")]
    sequence: u64,
    #[serde(with = "u64_str")]
    accepted_at_ms: u64,
    kind: String,
    #[serde(default, deserialize_with = "present")]
    payload: Option<Value>,
    #[serde(default)]
    payload_raw: Option<String>,
}

/// Keeps an explicit `null` distinct from an absent member.
fn present<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(de).map(Some)
}

impl TryFrom<EventRepr> for Event {
    type Error = String;

    fn try_from(repr: EventRepr) -> Result<Self, Self::Error> {
        let payload = match (repr.payload, repr.payload_raw) {
            (Some(value), None) => Payload::Json(value),
            (None, Some(raw)) => Payload::Raw(raw),
            (Some(_), Some(_)) => {
                return Err("an event has either `payload` or `payloadRaw`, not both".to_string())
            }
            (None, None) => return Err("an event needs `payload` or `payloadRaw`".to_string()),
        };
        Ok(Event {
            event_id: repr.event_id,
            sequence: repr.sequence,
            accepted_at_ms: repr.accepted_at_ms,
            kind: repr.kind,
            payload,
        })
    }
}

/// What a conforming reducer returns for a step.
#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    Decision(ExpectedDecision),
    /// The step commits nothing; the next step sees the same snapshot.
    Failure(ExpectedFailure),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedDecision {
    pub snapshot: Value,
    pub snapshot_digest: String,
    pub commands: Vec<ExpectedCommand>,
    pub diagnostics: Vec<ExpectedDiagnostic>,
    /// `digest::decision` of the whole decision.
    pub decision_digest: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedCommand {
    pub command_id: String,
    pub activation_id: String,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedDiagnostic {
    pub code: String,
    pub node_id: Option<String>,
    pub details: Value,
}

/// `Failure.details` is explanatory text and is not compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedFailure {
    pub kind: FailureKind,
    pub code: String,
}

impl<'de> Deserialize<'de> for Expect {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct FailureRepr {
            failure: ExpectedFailure,
        }

        let members = serde_json::Map::deserialize(de)?;
        let is_failure = members.contains_key("failure");
        let value = Value::Object(members);
        if is_failure {
            serde_json::from_value::<FailureRepr>(value).map(|repr| Expect::Failure(repr.failure))
        } else {
            serde_json::from_value(value).map(Expect::Decision)
        }
        .map_err(serde::de::Error::custom)
    }
}

impl Trace {
    /// A trace with no limits override and no steps.
    pub fn new(name: &str, description: &str, workflow: Value) -> Self {
        Trace {
            format: TRACE_FORMAT.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            workflow,
            limits: None,
            steps: Vec::new(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        let trace: Trace = serde_json::from_str(text).map_err(|e| Error::Syntax {
            path: None,
            message: e.to_string(),
        })?;
        trace.check_format()?;
        Ok(trace)
    }

    /// The file form: pretty-printed, members in documented order, ending in a newline.
    pub fn to_json(&self) -> String {
        let mut text = self.doc().render();
        text.push('\n');
        text
    }

    pub(crate) fn check_format(&self) -> Result<(), Error> {
        if self.format == TRACE_FORMAT {
            Ok(())
        } else {
            Err(Error::Format {
                trace: self.name.clone(),
                found: self.format.clone(),
                expected: TRACE_FORMAT,
            })
        }
    }

    fn doc(&self) -> Doc {
        Doc::record([
            ("format", Some(Doc::string(&self.format))),
            ("name", Some(Doc::string(&self.name))),
            ("description", Some(Doc::string(&self.description))),
            ("workflow", Some(Doc::value(&self.workflow))),
            ("limits", self.limits.as_ref().map(LimitsOverride::doc)),
            (
                "steps",
                Some(Doc::Array(self.steps.iter().map(Step::doc).collect())),
            ),
        ])
    }
}

impl LimitsOverride {
    fn doc(&self) -> Doc {
        Doc::record([
            ("microsteps", self.microsteps.map(Doc::number)),
            (
                "expressionOperations",
                self.expression_operations.map(Doc::number),
            ),
            ("maxStateBytes", self.max_state_bytes.map(Doc::number)),
            ("maxCommandCount", self.max_command_count.map(Doc::number)),
        ])
    }
}

impl Step {
    /// A step with no extensions and no expectation yet.
    pub fn new(event: Event) -> Self {
        Step {
            event,
            identity: None,
            snapshot_raw: None,
            expect: None,
        }
    }

    fn doc(&self) -> Doc {
        Doc::record([
            ("event", Some(self.event.doc())),
            (
                "identity",
                self.identity.as_ref().map(IdentityOverride::doc),
            ),
            ("snapshotRaw", self.snapshot_raw.as_deref().map(Doc::string)),
            ("expect", self.expect.as_ref().map(Expect::doc)),
        ])
    }
}

impl IdentityOverride {
    fn doc(&self) -> Doc {
        Doc::record([
            (
                "packageDigest",
                self.package_digest.as_deref().map(Doc::string),
            ),
            (
                "kernelDigest",
                self.kernel_digest.as_deref().map(Doc::string),
            ),
            (
                "semanticsVersion",
                self.semantics_version.as_deref().map(Doc::string),
            ),
            (
                "codecVersion",
                self.codec_version.as_deref().map(Doc::string),
            ),
        ])
    }
}

impl Event {
    /// An event with a plain-JSON payload.
    pub fn new(
        event_id: &str,
        sequence: u64,
        accepted_at_ms: u64,
        kind: &str,
        payload: Value,
    ) -> Self {
        Event {
            event_id: event_id.to_string(),
            sequence,
            accepted_at_ms,
            kind: kind.to_string(),
            payload: Payload::Json(payload),
        }
    }

    /// An event whose payload bytes are sent exactly as written.
    pub fn raw(
        event_id: &str,
        sequence: u64,
        accepted_at_ms: u64,
        kind: &str,
        payload: &str,
    ) -> Self {
        Event {
            event_id: event_id.to_string(),
            sequence,
            accepted_at_ms,
            kind: kind.to_string(),
            payload: Payload::Raw(payload.to_string()),
        }
    }

    fn doc(&self) -> Doc {
        let (payload, payload_raw) = match &self.payload {
            Payload::Json(value) => (Some(Doc::value(value)), None),
            Payload::Raw(raw) => (None, Some(Doc::string(raw))),
        };
        Doc::record([
            ("eventId", Some(Doc::string(&self.event_id))),
            ("sequence", Some(Doc::string(&self.sequence.to_string()))),
            (
                "acceptedAtMs",
                Some(Doc::string(&self.accepted_at_ms.to_string())),
            ),
            ("kind", Some(Doc::string(&self.kind))),
            ("payload", payload),
            ("payloadRaw", payload_raw),
        ])
    }
}

impl Expect {
    pub(crate) fn doc(&self) -> Doc {
        match self {
            Expect::Decision(decision) => Doc::record([
                ("snapshot", Some(Doc::value(&decision.snapshot))),
                (
                    "snapshotDigest",
                    Some(Doc::string(&decision.snapshot_digest)),
                ),
                (
                    "commands",
                    Some(Doc::Array(
                        decision.commands.iter().map(ExpectedCommand::doc).collect(),
                    )),
                ),
                (
                    "diagnostics",
                    Some(Doc::Array(
                        decision
                            .diagnostics
                            .iter()
                            .map(ExpectedDiagnostic::doc)
                            .collect(),
                    )),
                ),
                (
                    "decisionDigest",
                    Some(Doc::string(&decision.decision_digest)),
                ),
            ]),
            Expect::Failure(failure) => Doc::record([(
                "failure",
                Some(Doc::record([
                    ("kind", Some(Doc::string(failure_kind(failure.kind)))),
                    ("code", Some(Doc::string(&failure.code))),
                ])),
            )]),
        }
    }
}

/// The wire spelling of a failure kind.
fn failure_kind(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::InvalidInput => "invalid-input",
        FailureKind::IncompatibleVersion => "incompatible-version",
        FailureKind::ResourceLimit => "resource-limit",
        FailureKind::InvariantViolation => "invariant-violation",
    }
}

impl ExpectedCommand {
    fn doc(&self) -> Doc {
        Doc::record([
            ("commandId", Some(Doc::string(&self.command_id))),
            ("activationId", Some(Doc::string(&self.activation_id))),
            ("kind", Some(Doc::string(&self.kind))),
            ("payload", Some(Doc::value(&self.payload))),
        ])
    }
}

impl ExpectedDiagnostic {
    fn doc(&self) -> Doc {
        Doc::record([
            ("code", Some(Doc::string(&self.code))),
            (
                "nodeId",
                Some(self.node_id.as_deref().map_or(Doc::null(), Doc::string)),
            ),
            ("details", Some(Doc::value(&self.details))),
        ])
    }
}

/// Reads one trace file.
pub fn load(path: impl AsRef<Path>) -> Result<Trace, Error> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Trace::from_json(&text).map_err(|error| match error {
        Error::Syntax { message, .. } => Error::Syntax {
            path: Some(path.to_path_buf()),
            message,
        },
        other => other,
    })
}

/// Writes one trace file in the form [`Trace::to_json`] gives.
pub fn save(path: impl AsRef<Path>, trace: &Trace) -> Result<(), Error> {
    let path = path.as_ref();
    std::fs::write(path, trace.to_json()).map_err(|e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DOCUMENT: &str = r#"{
      "steps": [
        {"expect": {"decisionDigest": "sha256:dd", "diagnostics": [{"details": {"b": 1, "a": 2}, "nodeId": null, "code": "result.stale"}],
                    "commands": [{"payload": {"attempt": 1}, "kind": "activity.schedule", "activationId": "build/1", "commandId": "cmd_1"}],
                    "snapshotDigest": "sha256:ss", "snapshot": {"status": "running", "activations": 1}},
         "event": {"payload": {"tenant": "default", "runId": "run_1", "input": null}, "kind": "run.started",
                   "acceptedAtMs": "1000", "sequence": "1", "eventId": "start"}},
        {"event": {"eventId": "bad", "sequence": "2", "acceptedAtMs": "1001", "kind": "signal.received",
                   "payloadRaw": "{\"name\" : \"x\"}"},
         "identity": {"semanticsVersion": "9.9"},
         "snapshotRaw": "{}",
         "expect": {"failure": {"code": "budget.microsteps", "kind": "resource-limit"}}},
        {"event": {"eventId": "open", "sequence": "2", "acceptedAtMs": "1002", "kind": "signal.received", "payload": null}}
      ],
      "limits": {"maxStateBytes": 4096, "microsteps": 7},
      "workflow": {"start": "build", "format": "runspore.workflow/0.1"},
      "description": "one line",
      "name": "example",
      "format": "runspore.trace/0.1"
    }"#;

    #[test]
    fn reads_every_member_of_the_format() {
        let trace = Trace::from_json(DOCUMENT).unwrap();
        assert_eq!(trace.name, "example");
        assert_eq!(trace.description, "one line");
        assert_eq!(trace.workflow["start"], "build");
        let limits = trace.limits.unwrap();
        assert_eq!(
            limits,
            LimitsOverride {
                microsteps: Some(7),
                max_state_bytes: Some(4096),
                ..LimitsOverride::default()
            }
        );
        assert_eq!(
            limits.resolve(),
            Limits {
                microsteps: 7,
                max_state_bytes: 4096,
                ..Limits::default()
            }
        );

        let first = &trace.steps[0];
        assert_eq!(
            first.event,
            Event::new(
                "start",
                1,
                1000,
                "run.started",
                json!({"tenant": "default", "runId": "run_1", "input": null})
            )
        );
        let Some(Expect::Decision(decision)) = &first.expect else {
            panic!("expected a decision: {first:?}");
        };
        assert_eq!(
            decision.snapshot,
            json!({"status": "running", "activations": 1})
        );
        assert_eq!(decision.snapshot_digest, "sha256:ss");
        assert_eq!(decision.commands[0].command_id, "cmd_1");
        assert_eq!(decision.commands[0].payload, json!({"attempt": 1}));
        assert_eq!(decision.diagnostics[0].node_id, None);
        assert_eq!(decision.diagnostics[0].details, json!({"a": 2, "b": 1}));
        assert_eq!(decision.decision_digest, "sha256:dd");

        let second = &trace.steps[1];
        assert_eq!(
            second.event.payload,
            Payload::Raw(r#"{"name" : "x"}"#.to_string())
        );
        assert_eq!(
            second
                .identity
                .as_ref()
                .unwrap()
                .semantics_version
                .as_deref(),
            Some("9.9")
        );
        assert_eq!(second.snapshot_raw.as_deref(), Some("{}"));
        assert_eq!(
            second.expect,
            Some(Expect::Failure(ExpectedFailure {
                kind: FailureKind::ResourceLimit,
                code: "budget.microsteps".to_string(),
            }))
        );

        let third = &trace.steps[2];
        assert_eq!(third.event.payload, Payload::Json(Value::Null));
        assert_eq!(third.expect, None);
    }

    #[test]
    fn writes_members_in_documented_order_whatever_order_they_were_read_in() {
        let text = Trace::from_json(DOCUMENT).unwrap().to_json();
        assert_eq!(
            text,
            r#"{
  "format": "runspore.trace/0.1",
  "name": "example",
  "description": "one line",
  "workflow": {"format": "runspore.workflow/0.1", "start": "build"},
  "limits": {"microsteps": 7, "maxStateBytes": 4096},
  "steps": [
    {
      "event": {
        "eventId": "start",
        "sequence": "1",
        "acceptedAtMs": "1000",
        "kind": "run.started",
        "payload": {"input": null, "runId": "run_1", "tenant": "default"}
      },
      "expect": {
        "snapshot": {"activations": 1, "status": "running"},
        "snapshotDigest": "sha256:ss",
        "commands": [
          {
            "commandId": "cmd_1",
            "activationId": "build/1",
            "kind": "activity.schedule",
            "payload": {"attempt": 1}
          }
        ],
        "diagnostics": [{"code": "result.stale", "nodeId": null, "details": {"a": 2, "b": 1}}],
        "decisionDigest": "sha256:dd"
      }
    },
    {
      "event": {
        "eventId": "bad",
        "sequence": "2",
        "acceptedAtMs": "1001",
        "kind": "signal.received",
        "payloadRaw": "{\"name\" : \"x\"}"
      },
      "identity": {"semanticsVersion": "9.9"},
      "snapshotRaw": "{}",
      "expect": {"failure": {"kind": "resource-limit", "code": "budget.microsteps"}}
    },
    {
      "event": {
        "eventId": "open",
        "sequence": "2",
        "acceptedAtMs": "1002",
        "kind": "signal.received",
        "payload": null
      }
    }
  ]
}
"#
        );
    }

    #[test]
    fn written_text_reads_back_to_the_same_trace_and_the_same_text() {
        let trace = Trace::from_json(DOCUMENT).unwrap();
        let text = trace.to_json();
        let again = Trace::from_json(&text).unwrap();
        assert_eq!(again, trace);
        assert_eq!(again.to_json(), text);
    }

    #[test]
    fn rejects_documents_that_are_not_traces() {
        let with = |from: &str, to: &str| {
            assert!(DOCUMENT.contains(from), "{from}");
            Trace::from_json(&DOCUMENT.replace(from, to))
        };
        for (from, to) in [
            (r#""name": "example","#, r#""name": "example", "extra": 1,"#),
            (r#""sequence": "1""#, r#""sequence": 1"#),
            (r#""sequence": "1""#, r#""sequence": "01""#),
            (
                r#""payload": null"#,
                r#""payload": null, "payloadRaw": "x""#,
            ),
            (r#", "payload": null"#, ""),
            (r#""kind": "resource-limit""#, r#""kind": "out-of-luck""#),
            (
                r#""kind": "resource-limit""#,
                r#""kind": "resource-limit", "details": "x""#,
            ),
            (r#""failure": {"#, r#""snapshot": {}, "failure": {"#),
            (r#""snapshotDigest": "sha256:ss","#, ""),
            (r#""nodeId": null"#, r#""nodeId": 7"#),
        ] {
            let error = with(from, to).unwrap_err();
            assert!(
                matches!(error, Error::Syntax { path: None, .. }),
                "{to}: {error}"
            );
        }
        assert_eq!(
            with("runspore.trace/0.1", "runspore.trace/0.2"),
            Err(Error::Format {
                trace: "example".to_string(),
                found: "runspore.trace/0.2".to_string(),
                expected: TRACE_FORMAT,
            })
        );
    }

    #[test]
    fn load_and_save_report_the_path() {
        let dir =
            std::env::temp_dir().join(format!("runspore-trace-{}-format", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("example.json");
        let trace = Trace::from_json(DOCUMENT).unwrap();
        save(&path, &trace).unwrap();
        assert_eq!(load(&path).unwrap(), trace);

        std::fs::write(&path, "{").unwrap();
        assert!(matches!(load(&path), Err(Error::Syntax { path: Some(at), .. }) if at == path));
        let missing = dir.join("missing.json");
        assert!(matches!(load(&missing), Err(Error::Io { path: at, .. }) if at == missing));
        assert!(matches!(
            save(dir.join("no-such-dir").join("x.json"), &trace),
            Err(Error::Io { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
