//! Workflow document, run state, event payloads, and command payloads.
//!
//! These types are the wire contract. Every one of them is serialized with
//! [`crate::canonical::encode`]; field names and enum spellings are frozen under
//! [`SEMANTICS_VERSION`]. Behavior is specified in `spec/kernel.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WORKFLOW_FORMAT: &str = "runspore.workflow/0.1";
pub const STATE_FORMAT: &str = "runspore.state/0.1";
pub const SEMANTICS_VERSION: &str = "0.1";
pub const CODEC_VERSION: &str = "jcs-int53/1";
pub const ABI_VERSION: &str = "runspore:machine@0.1.0";

/// Outcome every activity node may route without the action declaring it.
pub const OUTCOME_FAILED: &str = "failed";
/// Outcome of a success result or a signal that names none.
pub const OUTCOME_OK: &str = "ok";
pub const OUTCOME_RECEIVED: &str = "received";

pub const MAX_NODES: usize = 256;
pub const MAX_BUFFERED_SIGNALS: usize = 64;

/// `u64` counters and timestamps travel as decimal strings, capped at 2^63 − 1.
pub mod u64_str {
    use serde::{Deserialize, Deserializer, Serializer};

    pub const MAX: u64 = i64::MAX as u64;

    pub fn serialize<S: Serializer>(value: &u64, ser: S) -> Result<S::Ok, S::Error> {
        if *value > MAX {
            return Err(serde::ser::Error::custom("counter exceeds 2^63 - 1"));
        }
        ser.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
        let text = String::deserialize(de)?;
        let canonical = text == "0" || (!text.starts_with('0') && !text.is_empty());
        let value: u64 = text
            .parse()
            .ok()
            .filter(|_| canonical && text.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| {
                serde::de::Error::custom("expected a canonical decimal counter string")
            })?;
        if value > MAX {
            return Err(serde::de::Error::custom("counter exceeds 2^63 - 1"));
        }
        Ok(value)
    }
}

// ---------------------------------------------------------------------------
// Workflow document
// ---------------------------------------------------------------------------

/// A workflow package: one JSON document holding the graph and its action bindings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workflow {
    pub format: String,
    pub name: String,
    pub start: String,
    #[serde(default)]
    pub limits: WorkflowLimits,
    /// Action bindings by ID. The kernel reads [`ActionPolicy`] from each entry and
    /// ignores every other field; hosts read their implementation fields.
    #[serde(default)]
    pub actions: BTreeMap<String, Value>,
    pub nodes: BTreeMap<String, Node>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowLimits {
    /// Entries allowed into any single node. Bounds every loop.
    #[serde(default = "WorkflowLimits::default_max_visits_per_node")]
    pub max_visits_per_node: u32,
    /// Node entries allowed across the whole run.
    #[serde(default = "WorkflowLimits::default_max_activations")]
    pub max_activations: u32,
}

impl WorkflowLimits {
    fn default_max_visits_per_node() -> u32 {
        16
    }
    fn default_max_activations() -> u32 {
        256
    }
}

impl Default for WorkflowLimits {
    fn default() -> Self {
        Self {
            max_visits_per_node: Self::default_max_visits_per_node(),
            max_activations: Self::default_max_activations(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Node {
    /// Runs one action. `outcomes` routes each outcome name to the next node ID.
    Activity {
        action: String,
        /// Mapping evaluated against the run context to build the activity input.
        #[serde(default)]
        input: Value,
        outcomes: BTreeMap<String, String>,
    },
    /// Consumes the earliest buffered signal with this name.
    AwaitSignal {
        signal: String,
        outcomes: BTreeMap<String, String>,
    },
    Complete {
        /// Mapping evaluated against the run context to build the run output.
        #[serde(default)]
        output: Value,
    },
    Fail {
        error: ErrorInfo,
    },
}

/// The fields of an action binding the kernel depends on. Unknown fields are
/// implementation details of the host and are ignored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPolicy {
    /// Outcome names a success result may report. `failed` is reserved.
    #[serde(default = "ActionPolicy::default_outcomes")]
    pub outcomes: Vec<String>,
    pub effect: Effect,
    #[serde(default)]
    pub retry: RetryPolicy,
}

impl ActionPolicy {
    fn default_outcomes() -> Vec<String> {
        vec![OUTCOME_OK.to_string()]
    }
}

/// What may happen when an attempt's outcome is not known to be a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    Pure,
    ReadOnly,
    Idempotent,
    /// Declared in the architecture but not supported by semantics 0.1; rejected at validation.
    Reconcilable,
    /// An unknown or expired attempt pauses the run for an operator.
    Unsafe,
}

impl Effect {
    /// Whether an attempt with an unknown outcome may be issued again.
    pub fn repeatable(self) -> bool {
        matches!(self, Effect::Pure | Effect::ReadOnly | Effect::Idempotent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryPolicy {
    /// Total physical attempts, including the first. 1 to 100.
    #[serde(default = "RetryPolicy::default_max_attempts")]
    pub max_attempts: u32,
    /// Delay before attempt 2; doubles per attempt up to `max_backoff_ms`.
    #[serde(default, with = "u64_str")]
    pub backoff_ms: u64,
    #[serde(default = "RetryPolicy::default_max_backoff_ms", with = "u64_str")]
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    fn default_max_attempts() -> u32 {
        1
    }
    fn default_max_backoff_ms() -> u64 {
        60_000
    }
    /// Delay before the attempt that follows failed attempt number `failed_attempt`.
    pub fn delay_after(&self, failed_attempt: u32) -> u64 {
        let shift = failed_attempt.saturating_sub(1).min(32);
        self.backoff_ms
            .saturating_mul(1u64 << shift)
            .min(self.max_backoff_ms)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: Self::default_max_attempts(),
            backoff_ms: 0,
            max_backoff_ms: Self::default_max_backoff_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub details: Value,
}

// ---------------------------------------------------------------------------
// Run state (the snapshot)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub format: String,
    pub semantics: String,
    pub tenant: String,
    pub run_id: String,
    pub status: RunStatus,
    pub input: Value,
    /// Latest result per node ID; a later visit replaces an earlier one.
    pub nodes: BTreeMap<String, NodeResult>,
    /// Entries per node ID so far.
    pub visits: BTreeMap<String, u32>,
    pub activations: u32,
    /// The node the run is parked at; `None` once terminal.
    pub position: Option<Position>,
    /// The single outstanding logical invocation, if any.
    pub invocation: Option<InvocationState>,
    /// Signals accepted but not yet consumed, in acceptance order.
    pub signals: Vec<BufferedSignal>,
    pub result: Option<RunResult>,
    #[serde(with = "u64_str")]
    pub last_sequence: u64,
    /// Logical time: the maximum accepted time seen so far. Never decreases.
    #[serde(with = "u64_str")]
    pub last_accepted_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    /// An activity invocation is scheduled or in flight.
    Running,
    /// Parked at an await-signal node.
    Waiting,
    /// An unsafe invocation has an unknown outcome; an operator must resolve it.
    NeedsIntervention,
    Completed,
    Failed,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, RunStatus::Completed | RunStatus::Failed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Waiting => "waiting",
            RunStatus::NeedsIntervention => "needs-intervention",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeResult {
    pub visit: u32,
    pub outcome: String,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub node_id: String,
    pub activation_id: String,
    pub visit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationState {
    pub invocation_id: String,
    pub activation_id: String,
    pub node_id: String,
    pub action_id: String,
    /// The attempt the kernel last authorized.
    pub attempt: u32,
    pub state: InvocationPhase,
    pub input_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationPhase {
    /// Attempt `attempt` is authorized; its result has not been accepted.
    Scheduled,
    /// Attempt `attempt` ended with an unknown outcome on an unsafe action.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BufferedSignal {
    pub event_id: String,
    #[serde(with = "u64_str")]
    pub sequence: u64,
    pub name: String,
    pub outcome: Option<String>,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunResult {
    Output(Value),
    Error(ErrorInfo),
}

// ---------------------------------------------------------------------------
// Events (inbox) — `Envelope.kind` plus canonical payload
// ---------------------------------------------------------------------------

pub mod event_kind {
    pub const RUN_STARTED: &str = "run.started";
    pub const ACTIVITY_RESULT: &str = "activity.result";
    pub const SIGNAL_RECEIVED: &str = "signal.received";
    pub const INVOCATION_RESOLVED: &str = "invocation.resolved";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunStarted {
    pub tenant: String,
    pub run_id: String,
    pub input: Value,
}

/// The outcome of one physical attempt, appended by `finish_attempt` or `expire_attempt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityResult {
    pub invocation_id: String,
    pub attempt_id: String,
    pub attempt: u32,
    pub status: AttemptStatus,
    /// Success only; defaults to `ok`.
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub output: Value,
    /// Failure and unknown only.
    #[serde(default)]
    pub error: Option<ErrorInfo>,
    /// Failure only: `false` routes straight to `failed` regardless of the retry budget.
    #[serde(default = "ActivityResult::default_retryable")]
    pub retryable: bool,
}

impl ActivityResult {
    fn default_retryable() -> bool {
        true
    }

    /// The body every store appends when it expires a lease. Byte-identical across stores.
    pub fn expired(invocation_id: &str, attempt: u32) -> Self {
        Self {
            invocation_id: invocation_id.to_string(),
            attempt_id: crate::digest::ids::attempt(invocation_id, attempt),
            attempt,
            status: AttemptStatus::Expired,
            outcome: None,
            output: Value::Null,
            error: None,
            retryable: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptStatus {
    /// The implementation ran and reported an outcome.
    Success,
    /// The implementation ran and reported that it failed.
    Failure,
    /// The host cannot say whether the effect happened (timeout, killed process).
    Unknown,
    /// The lease ran out with no result; the worker may still be acting.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignalReceived {
    pub name: String,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvocationResolved {
    pub invocation_id: String,
    pub resolution: Resolution,
}

/// An operator's answer for an invocation whose outcome is unknown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Resolution {
    /// The effect happened; continue with this outcome and output.
    Complete {
        outcome: String,
        #[serde(default)]
        output: Value,
    },
    /// The effect did not happen, or repeating it is acceptable; issue a new attempt.
    Retry,
    /// Give up on the invocation and take the `failed` route.
    Fail { error: ErrorInfo },
}

// ---------------------------------------------------------------------------
// Commands (outbox) — `Command.kind` plus canonical payload
// ---------------------------------------------------------------------------

pub mod command_kind {
    pub const ACTIVITY_SCHEDULE: &str = "activity.schedule";
    pub const ACTIVITY_RETRY: &str = "activity.retry";
}

/// Creates a logical invocation and authorizes its first attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleActivity {
    pub invocation_id: String,
    pub activation_id: String,
    pub node_id: String,
    pub action_id: String,
    pub effect_key: String,
    /// Always 1.
    pub attempt: u32,
    #[serde(with = "u64_str")]
    pub not_before_ms: u64,
    pub input: Value,
}

/// Authorizes one more attempt of an existing invocation, with its original input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryActivity {
    pub invocation_id: String,
    pub attempt: u32,
    #[serde(with = "u64_str")]
    pub not_before_ms: u64,
}

// ---------------------------------------------------------------------------
// Stable code vocabulary
// ---------------------------------------------------------------------------

/// Codes carried by `Failure`, `Diagnostic`, and `ErrorInfo`. Stable under [`SEMANTICS_VERSION`].
pub mod code {
    // Failure::InvalidInput
    pub const WORKFLOW_INVALID: &str = "workflow.invalid";
    pub const EVENT_INVALID: &str = "event.invalid";
    pub const EVENT_OUT_OF_ORDER: &str = "event.out-of-order";
    pub const EVENT_UNKNOWN_KIND: &str = "event.unknown-kind";
    pub const STATE_INVALID: &str = "state.invalid";
    pub const STATE_MISSING: &str = "state.missing";
    pub const STATE_UNEXPECTED_START: &str = "state.unexpected-start";
    // Failure::IncompatibleVersion
    pub const VERSION_SEMANTICS: &str = "version.semantics";
    pub const VERSION_CODEC: &str = "version.codec";
    pub const VERSION_STATE_FORMAT: &str = "version.state-format";
    pub const WORKFLOW_UNSUPPORTED_EFFECT: &str = "workflow.unsupported-effect";
    // Failure::ResourceLimit
    pub const BUDGET_MICROSTEPS: &str = "budget.microsteps";
    pub const BUDGET_EXPRESSION_OPERATIONS: &str = "budget.expression-operations";
    pub const BUDGET_STATE_BYTES: &str = "budget.state-bytes";
    pub const BUDGET_COMMAND_COUNT: &str = "budget.command-count";
    // Diagnostics (the event is consumed; the decision records why nothing else happened)
    pub const EVENT_IGNORED_TERMINAL: &str = "event.ignored-terminal";
    pub const RESULT_STALE: &str = "result.stale";
    pub const RESOLVE_NOT_APPLICABLE: &str = "resolve.not-applicable";
    pub const SIGNAL_BUFFER_OVERFLOW: &str = "signal.buffer-overflow";
    pub const SIGNAL_OUTCOME_UNROUTED: &str = "signal.outcome-unrouted";
    pub const INVOCATION_UNKNOWN: &str = "invocation.unknown";
    pub const ACTIVITY_RETRY_SCHEDULED: &str = "activity.retry-scheduled";
    // ErrorInfo codes produced by the kernel (business-visible failures)
    pub const LIMIT_VISITS_EXCEEDED: &str = "limit.visits-exceeded";
    pub const LIMIT_ACTIVATIONS_EXCEEDED: &str = "limit.activations-exceeded";
    pub const MAPPING_MISSING_PATH: &str = "mapping.missing-path";
    pub const ACTIVITY_UNDECLARED_OUTCOME: &str = "activity.undeclared-outcome";
    pub const ACTIVITY_ATTEMPTS_EXHAUSTED: &str = "activity.attempts-exhausted";
    pub const ACTIVITY_FAILED: &str = "activity.failed";
    pub const OPERATOR_FAILED: &str = "operator.failed";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical;

    const WORKFLOW: &str = r#"{
      "format": "runspore.workflow/0.1",
      "name": "review-loop",
      "start": "build",
      "actions": {
        "build": {"kind": "command", "argv": ["make"], "effect": "idempotent",
                  "outcomes": ["ok", "red"], "retry": {"maxAttempts": 3, "backoffMs": "500"}}
      },
      "nodes": {
        "build": {"kind": "activity", "action": "build",
                  "input": {"repo": {"$get": ["input", "repo"]}},
                  "outcomes": {"ok": "approve", "red": "build", "failed": "broken"}},
        "approve": {"kind": "await-signal", "signal": "approval",
                    "outcomes": {"approved": "done", "rejected": "build"}},
        "done": {"kind": "complete", "output": {"$get": ["nodes", "build", "output"]}},
        "broken": {"kind": "fail", "error": {"code": "build.broken", "message": "build never passed"}}
      }
    }"#;

    #[test]
    fn workflow_document_parses_with_defaults() {
        let bytes = canonical::to_vec(&canonical::parse(WORKFLOW.as_bytes()).unwrap()).unwrap();
        let workflow: Workflow = canonical::decode(&bytes).unwrap();
        assert_eq!(workflow.limits, WorkflowLimits::default());
        assert!(matches!(
            workflow.nodes["approve"],
            Node::AwaitSignal { .. }
        ));
        let policy: ActionPolicy =
            serde_json::from_value(workflow.actions["build"].clone()).unwrap();
        assert_eq!(policy.effect, Effect::Idempotent);
        assert_eq!(policy.retry.max_attempts, 3);
        assert_eq!(policy.retry.backoff_ms, 500);
        assert_eq!(policy.retry.max_backoff_ms, 60_000);
    }

    #[test]
    fn unknown_node_fields_are_rejected() {
        let text = r#"{"kind":"complete","output":null,"extra":1}"#;
        assert!(serde_json::from_str::<Node>(text).is_err());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let retry = RetryPolicy {
            max_attempts: 9,
            backoff_ms: 1_000,
            max_backoff_ms: 5_000,
        };
        assert_eq!(
            (1..=4).map(|n| retry.delay_after(n)).collect::<Vec<_>>(),
            vec![1_000, 2_000, 4_000, 5_000]
        );
        assert_eq!(retry.delay_after(200), 5_000);
    }

    #[test]
    fn counters_are_canonical_decimal_strings() {
        let retry: RetryPolicy = serde_json::from_str(r#"{"backoffMs":"0"}"#).unwrap();
        assert_eq!(retry.backoff_ms, 0);
        for bad in [
            r#"{"backoffMs":"01"}"#,
            r#"{"backoffMs":5}"#,
            r#"{"backoffMs":"-1"}"#,
            r#"{"backoffMs":"+1"}"#,
            r#"{"backoffMs":"9223372036854775808"}"#,
            r#"{"backoffMs":""}"#,
        ] {
            assert!(serde_json::from_str::<RetryPolicy>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn state_and_payloads_round_trip_canonically() {
        let state = State {
            format: STATE_FORMAT.into(),
            semantics: SEMANTICS_VERSION.into(),
            tenant: "default".into(),
            run_id: "run_1".into(),
            status: RunStatus::NeedsIntervention,
            input: serde_json::json!({"a": 1}),
            nodes: BTreeMap::from([(
                "build".to_string(),
                NodeResult {
                    visit: 1,
                    outcome: "ok".into(),
                    output: Value::Null,
                },
            )]),
            visits: BTreeMap::from([("build".to_string(), 1)]),
            activations: 1,
            position: Some(Position {
                node_id: "build".into(),
                activation_id: "build/1".into(),
                visit: 1,
            }),
            invocation: None,
            signals: vec![],
            result: Some(RunResult::Error(ErrorInfo {
                code: "x".into(),
                message: "y".into(),
                node_id: None,
                details: Value::Null,
            })),
            last_sequence: 3,
            last_accepted_at_ms: 1_700_000_000_000,
        };
        let bytes = canonical::encode(&state).unwrap();
        assert_eq!(canonical::decode::<State>(&bytes).unwrap(), state);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(r#""status":"needs-intervention""#), "{text}");
        assert!(text.contains(r#""result":{"error":{"#), "{text}");
        assert!(text.contains(r#""lastSequence":"3""#), "{text}");

        let resolved: InvocationResolved = serde_json::from_str(
            r#"{"invocationId":"inv_1","resolution":{"action":"complete","outcome":"ok"}}"#,
        )
        .unwrap();
        assert_eq!(
            resolved.resolution,
            Resolution::Complete {
                outcome: "ok".into(),
                output: Value::Null
            }
        );

        let expired = canonical::encode(&ActivityResult::expired("inv_1", 2)).unwrap();
        assert_eq!(
            String::from_utf8(expired).unwrap(),
            r#"{"attempt":2,"attemptId":"inv_1.2","error":null,"invocationId":"inv_1","outcome":null,"output":null,"retryable":true,"status":"expired"}"#
        );
    }
}
