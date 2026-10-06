//! A trace resolved for runners that do not implement canonical JSON.
//!
//! [`compile`] canonicalizes everything once. A runner then only turns strings
//! into UTF-8 bytes, calls `transition`, and compares the bytes it gets back
//! with strings. Serialized with serde, a [`CompiledTrace`] is this JSON:
//!
//! ```json
//! {
//!   "format": "runspore.trace.compiled/0.1",
//!   "name": "retry-then-success",
//!   "description": "one line",
//!   "identity": {"packageDigest": "sha256:…", "kernelDigest": "trace:runspore.trace/0.1",
//!                "semanticsVersion": "0.1", "codecVersion": "jcs-int53/1"},
//!   "limits": {"microsteps": 1000, "expressionOperations": 10000,
//!              "maxStateBytes": 262144, "maxCommandCount": 128},
//!   "workflow": "{\"format\":\"runspore.workflow/0.1\",…}",
//!   "steps": [
//!     {"event": {"eventId": "start", "sequence": "1", "acceptedAtMs": "1000",
//!                "kind": "run.started", "payload": "{\"input\":…}"},
//!      "expect": {"snapshot": "{\"activations\":1,…}", "snapshotDigest": "sha256:…",
//!                 "commands": [{"commandId": "cmd_…", "activationId": "build/1",
//!                               "kind": "activity.schedule", "payload": "{…}"}],
//!                 "diagnostics": [{"code": "…", "nodeId": null, "details": "{…}"}],
//!                 "decisionDigest": "sha256:…"}},
//!     {"event": {"eventId": "…", "sequence": "2", "acceptedAtMs": "1500",
//!                "kind": "…", "payload": "…"},
//!      "identity": {"packageDigest": "…", "kernelDigest": "…",
//!                   "semanticsVersion": "9.9", "codecVersion": "…"},
//!      "snapshot": "…",
//!      "expect": {"failure": {"kind": "incompatible-version", "code": "version.semantics"}}}
//!   ]
//! }
//! ```
//!
//! - `workflow`, `payload`, `snapshot`, command `payload`, and `details` are
//!   strings holding the exact bytes, as UTF-8 text. A `payloadRaw` of the
//!   source trace arrives in `payload` verbatim.
//! - `sequence` and `acceptedAtMs` are decimal strings because they are 64-bit
//!   (`BigInt` in JavaScript). The four limits are 32-bit and are JSON numbers.
//! - A step's `identity` and `snapshot` members are present only when the
//!   source step uses those extensions; `identity` is then complete, not a
//!   partial override.
//! - `expect` holds either the five decision members or `failure`.
//!
//! # Replay
//!
//! ```text
//! snapshot = none
//! for step in steps:
//!     result = transition({
//!         identity:     step.identity or trace.identity,
//!         graph:        utf8(trace.workflow),
//!         snapshot:     utf8(step.snapshot) if present, else snapshot,
//!         inputEvent:   {eventId, sequence, acceptedAtMs, kind, payload: utf8(step.event.payload)},
//!         frozenLimits: trace.limits })
//!     if step.expect.failure:
//!         require result is a failure with the same kind and code
//!     else:
//!         require result is a decision whose snapshot, command payloads and
//!             diagnostic details equal the expected strings byte for byte, whose
//!             snapshotDigest, command IDs, activation IDs, kinds, diagnostic
//!             codes and node IDs equal the expected values, in order and in number
//!         snapshot = result.snapshot
//! ```
//!
//! `decisionDigest` is `digest::decision` of the expected decision. It is a
//! function of the members already compared, so a runner without the digest
//! framing may skip it; [`run_compiled`](crate::run::run_compiled) checks it.

use runspore_types::canonical::{self, CanonError};
use runspore_types::model::{u64_str, CODEC_VERSION, SEMANTICS_VERSION};
use runspore_types::reducer::{Identity, Limits};
use runspore_types::{digest, Value};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::format::{
    Event, Expect, ExpectedFailure, IdentityOverride, Payload, Step, Trace, KERNEL_DIGEST,
};

pub const COMPILED_FORMAT: &str = "runspore.trace.compiled/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledTrace {
    pub format: String,
    pub name: String,
    pub description: String,
    pub identity: CompiledIdentity,
    pub limits: CompiledLimits,
    /// Canonical workflow document.
    pub workflow: String,
    pub steps: Vec<CompiledStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledIdentity {
    pub package_digest: String,
    pub kernel_digest: String,
    pub semantics_version: String,
    pub codec_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledLimits {
    pub microsteps: u32,
    pub expression_operations: u32,
    pub max_state_bytes: u32,
    pub max_command_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledStep {
    pub event: CompiledEvent,
    /// The complete identity of this step's request, when it differs from the trace's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<CompiledIdentity>,
    /// Sent as this step's snapshot instead of the threaded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub expect: CompiledExpect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledEvent {
    pub event_id: String,
    #[serde(with = "u64_str")]
    pub sequence: u64,
    #[serde(with = "u64_str")]
    pub accepted_at_ms: u64,
    pub kind: String,
    /// The exact payload bytes.
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompiledExpect {
    Failure(CompiledFailure),
    Decision(CompiledDecision),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledFailure {
    pub failure: ExpectedFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledDecision {
    /// Canonical state.
    pub snapshot: String,
    pub snapshot_digest: String,
    pub commands: Vec<CompiledCommand>,
    pub diagnostics: Vec<CompiledDiagnostic>,
    pub decision_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledCommand {
    pub command_id: String,
    pub activation_id: String,
    pub kind: String,
    /// Canonical JSON.
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledDiagnostic {
    pub code: String,
    pub node_id: Option<String>,
    /// Canonical JSON.
    pub details: String,
}

impl From<&CompiledIdentity> for Identity {
    fn from(identity: &CompiledIdentity) -> Self {
        Identity {
            package_digest: identity.package_digest.clone(),
            kernel_digest: identity.kernel_digest.clone(),
            semantics_version: identity.semantics_version.clone(),
            codec_version: identity.codec_version.clone(),
        }
    }
}

impl From<CompiledLimits> for Limits {
    fn from(limits: CompiledLimits) -> Self {
        Limits {
            microsteps: limits.microsteps,
            expression_operations: limits.expression_operations,
            max_state_bytes: limits.max_state_bytes,
            max_command_count: limits.max_command_count,
        }
    }
}

/// Resolves a trace into the form documented at the top of this module. Fails
/// if a plain-JSON value is outside the canonical domain or a step has no
/// `expect`.
pub fn compile(trace: &Trace) -> Result<CompiledTrace, Error> {
    let header = Header::of(trace)?;
    let mut steps = Vec::with_capacity(trace.steps.len());
    for (index, step) in trace.steps.iter().enumerate() {
        let request = header.step(index, step)?;
        let expect = step.expect.as_ref().ok_or_else(|| Error::Unblessed {
            trace: trace.name.clone(),
            step: index,
            event_id: step.event.event_id.clone(),
        })?;
        steps.push(CompiledStep {
            event: request.event,
            identity: request.identity,
            snapshot: request.snapshot,
            expect: header.expect(index, expect)?,
        });
    }
    Ok(CompiledTrace {
        format: COMPILED_FORMAT.to_string(),
        name: trace.name.clone(),
        description: trace.description.clone(),
        identity: header.identity,
        limits: header.limits,
        workflow: header.workflow,
        steps,
    })
}

/// The parts of every request of a trace that do not depend on the step.
pub(crate) struct Header {
    name: String,
    pub(crate) identity: CompiledIdentity,
    pub(crate) limits: CompiledLimits,
    pub(crate) workflow: String,
}

/// The parts of one request that come from its step.
pub(crate) struct StepRequest {
    pub(crate) event: CompiledEvent,
    pub(crate) identity: Option<CompiledIdentity>,
    pub(crate) snapshot: Option<String>,
}

impl Header {
    pub(crate) fn of(trace: &Trace) -> Result<Header, Error> {
        trace.check_format()?;
        let workflow = text(&trace.workflow).map_err(|source| Error::Fixture {
            trace: trace.name.clone(),
            location: "workflow".to_string(),
            source,
        })?;
        let limits = trace.limits.unwrap_or_default().resolve();
        Ok(Header {
            name: trace.name.clone(),
            identity: CompiledIdentity {
                package_digest: digest::package(workflow.as_bytes()),
                kernel_digest: KERNEL_DIGEST.to_string(),
                semantics_version: SEMANTICS_VERSION.to_string(),
                codec_version: CODEC_VERSION.to_string(),
            },
            limits: CompiledLimits {
                microsteps: limits.microsteps,
                expression_operations: limits.expression_operations,
                max_state_bytes: limits.max_state_bytes,
                max_command_count: limits.max_command_count,
            },
            workflow,
        })
    }

    pub(crate) fn step(&self, index: usize, step: &Step) -> Result<StepRequest, Error> {
        Ok(StepRequest {
            event: self.event(index, &step.event)?,
            identity: step.identity.as_ref().map(|over| self.identity(over)),
            snapshot: step.snapshot_raw.clone(),
        })
    }

    fn event(&self, index: usize, event: &Event) -> Result<CompiledEvent, Error> {
        let payload = match &event.payload {
            Payload::Json(value) => {
                text(value).map_err(|e| self.fixture(format!("steps[{index}].event.payload"), e))?
            }
            Payload::Raw(raw) => raw.clone(),
        };
        Ok(CompiledEvent {
            event_id: event.event_id.clone(),
            sequence: event.sequence,
            accepted_at_ms: event.accepted_at_ms,
            kind: event.kind.clone(),
            payload,
        })
    }

    fn identity(&self, over: &IdentityOverride) -> CompiledIdentity {
        let base = &self.identity;
        let pick = |field: &Option<String>, default: &String| {
            field.clone().unwrap_or_else(|| default.clone())
        };
        CompiledIdentity {
            package_digest: pick(&over.package_digest, &base.package_digest),
            kernel_digest: pick(&over.kernel_digest, &base.kernel_digest),
            semantics_version: pick(&over.semantics_version, &base.semantics_version),
            codec_version: pick(&over.codec_version, &base.codec_version),
        }
    }

    fn expect(&self, index: usize, expect: &Expect) -> Result<CompiledExpect, Error> {
        let decision = match expect {
            Expect::Failure(failure) => {
                return Ok(CompiledExpect::Failure(CompiledFailure {
                    failure: failure.clone(),
                }))
            }
            Expect::Decision(decision) => decision,
        };
        let at = |member: String, source| {
            self.fixture(format!("steps[{index}].expect.{member}"), source)
        };
        let mut commands = Vec::with_capacity(decision.commands.len());
        for (n, command) in decision.commands.iter().enumerate() {
            commands.push(CompiledCommand {
                command_id: command.command_id.clone(),
                activation_id: command.activation_id.clone(),
                kind: command.kind.clone(),
                payload: text(&command.payload)
                    .map_err(|e| at(format!("commands[{n}].payload"), e))?,
            });
        }
        let mut diagnostics = Vec::with_capacity(decision.diagnostics.len());
        for (n, diagnostic) in decision.diagnostics.iter().enumerate() {
            diagnostics.push(CompiledDiagnostic {
                code: diagnostic.code.clone(),
                node_id: diagnostic.node_id.clone(),
                details: text(&diagnostic.details)
                    .map_err(|e| at(format!("diagnostics[{n}].details"), e))?,
            });
        }
        Ok(CompiledExpect::Decision(CompiledDecision {
            snapshot: text(&decision.snapshot).map_err(|e| at("snapshot".to_string(), e))?,
            snapshot_digest: decision.snapshot_digest.clone(),
            commands,
            diagnostics,
            decision_digest: decision.decision_digest.clone(),
        }))
    }

    fn fixture(&self, location: String, source: CanonError) -> Error {
        Error::Fixture {
            trace: self.name.clone(),
            location,
            source,
        }
    }
}

/// Canonical encoding as text. The encoder only emits UTF-8.
fn text(value: &Value) -> Result<String, CanonError> {
    let bytes = canonical::to_vec(value)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::format::{ExpectedDecision, LimitsOverride};
    use crate::run::{bless, run_compiled};
    use crate::testing::{self, Toy};

    /// A decision with a command, a refused step that uses both extensions,
    /// and a decision with a diagnostic.
    fn trace() -> Trace {
        let mut trace = Trace::new("shape", "pins the compiled form", testing::workflow());
        trace.limits = Some(LimitsOverride {
            microsteps: Some(5),
            ..LimitsOverride::default()
        });
        let mut refused = Step::new(Event::raw("raw", 2, 1_001, "signal.received", "{ }"));
        refused.identity = Some(IdentityOverride {
            codec_version: Some("other/1".to_string()),
            ..IdentityOverride::default()
        });
        refused.snapshot_raw = Some("[]".to_string());
        trace.steps = vec![
            Step::new(Event::new(
                "start",
                1,
                1_000,
                "run.started",
                json!({"tenant": "default", "runId": "run_toy", "input": {"b": 1, "a": [true]}}),
            )),
            refused,
            Step::new(Event::new(
                "stale",
                2,
                1_002,
                "activity.result",
                json!({"invocationId": "inv_x", "attemptId": "inv_x.1", "attempt": 1,
                       "status": "success"}),
            )),
        ];
        bless(&Toy, &trace).unwrap()
    }

    #[test]
    fn compiled_json_has_the_documented_shape() {
        let trace = trace();
        let compiled = compile(&trace).unwrap();
        let workflow = text(&trace.workflow).unwrap();
        assert!(workflow.starts_with(r#"{"actions":{"build":{"effect":"idempotent","#));
        let (Some(Expect::Decision(start)), Some(Expect::Decision(stale))) =
            (&trace.steps[0].expect, &trace.steps[2].expect)
        else {
            panic!("steps 0 and 2 are decisions");
        };
        let package_digest = digest::package(workflow.as_bytes());

        assert_eq!(
            serde_json::to_value(&compiled).unwrap(),
            json!({
                "format": "runspore.trace.compiled/0.1",
                "name": "shape",
                "description": "pins the compiled form",
                "identity": {
                    "packageDigest": package_digest,
                    "kernelDigest": "trace:runspore.trace/0.1",
                    "semanticsVersion": "0.1",
                    "codecVersion": "jcs-int53/1",
                },
                "limits": {
                    "microsteps": 5,
                    "expressionOperations": 10_000,
                    "maxStateBytes": 262_144,
                    "maxCommandCount": 128,
                },
                "workflow": workflow,
                "steps": [
                    {
                        "event": {
                            "eventId": "start",
                            "sequence": "1",
                            "acceptedAtMs": "1000",
                            "kind": "run.started",
                            "payload":
                                r#"{"input":{"a":[true],"b":1},"runId":"run_toy","tenant":"default"}"#,
                        },
                        "expect": {
                            "snapshot": text(&start.snapshot).unwrap(),
                            "snapshotDigest": start.snapshot_digest,
                            "commands": [{
                                "commandId": start.commands[0].command_id,
                                "activationId": "build/1",
                                "kind": "activity.schedule",
                                "payload": text(&start.commands[0].payload).unwrap(),
                            }],
                            "diagnostics": [],
                            "decisionDigest": start.decision_digest,
                        },
                    },
                    {
                        "event": {
                            "eventId": "raw",
                            "sequence": "2",
                            "acceptedAtMs": "1001",
                            "kind": "signal.received",
                            "payload": "{ }",
                        },
                        "identity": {
                            "packageDigest": package_digest,
                            "kernelDigest": "trace:runspore.trace/0.1",
                            "semanticsVersion": "0.1",
                            "codecVersion": "other/1",
                        },
                        "snapshot": "[]",
                        "expect": {"failure": {"kind": "invalid-input", "code": "event.invalid"}},
                    },
                    {
                        "event": {
                            "eventId": "stale",
                            "sequence": "2",
                            "acceptedAtMs": "1002",
                            "kind": "activity.result",
                            "payload":
                                r#"{"attempt":1,"attemptId":"inv_x.1","invocationId":"inv_x","status":"success"}"#,
                        },
                        "expect": {
                            "snapshot": text(&stale.snapshot).unwrap(),
                            "snapshotDigest": stale.snapshot_digest,
                            "commands": [],
                            "diagnostics": [{
                                "code": "result.stale",
                                "nodeId": null,
                                "details": r#"{"attempt":1,"invocationId":"inv_x"}"#,
                            }],
                            "decisionDigest": stale.decision_digest,
                        },
                    },
                ],
            })
        );
        assert!(text(&start.snapshot)
            .unwrap()
            .starts_with(r#"{"activations":1,"format":"runspore.state/0.1","#));
    }

    #[test]
    fn compiled_text_reads_back_and_replays() {
        let compiled = compile(&trace()).unwrap();
        let written = serde_json::to_string_pretty(&compiled).unwrap();
        assert!(
            written.starts_with("{\n  \"format\": \"runspore.trace.compiled/0.1\",\n  \"name\"")
        );
        let read: CompiledTrace = serde_json::from_str(&written).unwrap();
        assert_eq!(read, compiled);
        run_compiled(&Toy, &read).unwrap();
        assert!(matches!(read.steps[1].expect, CompiledExpect::Failure(_)));
        assert!(matches!(read.steps[2].expect, CompiledExpect::Decision(_)));
    }

    #[test]
    fn values_outside_the_canonical_domain_are_located() {
        fn location(alter: fn(&mut Trace)) -> String {
            let mut trace = trace();
            alter(&mut trace);
            match compile(&trace) {
                Err(Error::Fixture {
                    trace, location, ..
                }) => {
                    assert_eq!(trace, "shape");
                    location
                }
                other => panic!("expected a fixture error: {other:?}"),
            }
        }
        fn decision(trace: &mut Trace, step: usize) -> &mut ExpectedDecision {
            match &mut trace.steps[step].expect {
                Some(Expect::Decision(decision)) => decision,
                other => panic!("step {step} is not a decision: {other:?}"),
            }
        }
        assert_eq!(location(|t| t.workflow = json!({"n": 0.5})), "workflow");
        assert_eq!(
            location(|t| t.steps[2].event.payload = Payload::Json(json!(1e300))),
            "steps[2].event.payload"
        );
        assert_eq!(
            location(|t| decision(t, 0).snapshot = json!(9_007_199_254_740_992u64)),
            "steps[0].expect.snapshot"
        );
        assert_eq!(
            location(|t| decision(t, 0).commands[0].payload = json!([2.5])),
            "steps[0].expect.commands[0].payload"
        );
        assert_eq!(
            location(|t| decision(t, 2).diagnostics[0].details = json!(-0.1)),
            "steps[2].expect.diagnostics[0].details"
        );
    }
}
