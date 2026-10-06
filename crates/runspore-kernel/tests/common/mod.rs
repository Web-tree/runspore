//! Shared fixtures for the kernel acceptance tests (`spec/kernel.md`).
#![allow(dead_code)]

use runspore_kernel::transition;
use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::model::{event_kind, CODEC_VERSION, SEMANTICS_VERSION, WORKFLOW_FORMAT};
use runspore_types::reducer::{
    Decision, Diagnostic, Envelope, Failure, FailureKind, Identity, Limits, TransitionRequest,
};
use serde_json::{json, Value};

pub const TENANT: &str = "default";
pub const RUN_ID: &str = "run_1";
pub const START_MS: u64 = 1_000;

/// Canonical bytes of a fixture value.
pub fn canon(value: &Value) -> Vec<u8> {
    canonical::to_vec(value).expect("fixture is in the canonical domain")
}

/// Parses kernel output, which must already be canonical.
pub fn parse(bytes: &[u8]) -> Value {
    canonical::parse_canonical(bytes).expect("kernel output is canonical")
}

pub fn identity(graph: &[u8]) -> Identity {
    Identity {
        package_digest: digest::package(graph),
        kernel_digest: "test".to_string(),
        semantics_version: SEMANTICS_VERSION.to_string(),
        codec_version: CODEC_VERSION.to_string(),
    }
}

pub fn event_id(sequence: u64) -> String {
    if sequence == 1 {
        ids::EVENT_STARTED.to_string()
    } else {
        format!("e{sequence}")
    }
}

pub fn envelope(sequence: u64, at: u64, kind: &str, payload: &Value) -> Envelope {
    Envelope {
        event_id: event_id(sequence),
        sequence,
        accepted_at_ms: at,
        kind: kind.to_string(),
        payload: canon(payload),
    }
}

pub fn request(graph: &[u8], snapshot: Option<&[u8]>, event: Envelope) -> TransitionRequest {
    TransitionRequest {
        identity: identity(graph),
        graph: graph.to_vec(),
        snapshot: snapshot.map(<[u8]>::to_vec),
        input_event: event,
        frozen_limits: Limits::default(),
    }
}

pub fn started(input: Value) -> Value {
    json!({"tenant": TENANT, "runId": RUN_ID, "input": input})
}

/// The `run.started` request for a workflow, with the default budget.
pub fn start_request(workflow: &Value, input: Value) -> TransitionRequest {
    request(
        &canon(workflow),
        None,
        envelope(1, START_MS, event_kind::RUN_STARTED, &started(input)),
    )
}

pub fn assert_failure(result: Result<Decision, Failure>, kind: FailureKind, code: &str) {
    match result {
        Ok(decision) => panic!(
            "expected {kind:?} {code}, got a decision with snapshot {}",
            String::from_utf8_lossy(&decision.snapshot)
        ),
        Err(failure) => assert_eq!(
            (failure.kind, failure.code.as_str()),
            (kind, code),
            "{}",
            failure.details
        ),
    }
}

pub fn codes(decision: &Decision) -> Vec<&str> {
    decision
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect()
}

/// A diagnostic as the triple the golden traces compare.
pub fn diagnostic(item: &Diagnostic) -> Value {
    json!({"code": item.code, "nodeId": item.node_id, "details": parse(&item.details)})
}

pub fn diagnostics(decision: &Decision) -> Value {
    Value::Array(decision.diagnostics.iter().map(diagnostic).collect())
}

/// Replaces or inserts the member a JSON pointer names. The parent must be an object.
pub fn set(doc: &mut Value, pointer: &str, value: Value) {
    let (parent, key) = pointer.rsplit_once('/').expect("pointer has a parent");
    doc.pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .expect("parent is an object")
        .insert(key.to_string(), value);
}

/// Removes the member a JSON pointer names.
pub fn unset(doc: &mut Value, pointer: &str) {
    let (parent, key) = pointer.rsplit_once('/').expect("pointer has a parent");
    doc.pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .expect("parent is an object")
        .remove(key);
}

pub fn with(mut doc: Value, pointer: &str, value: Value) -> Value {
    set(&mut doc, pointer, value);
    doc
}

pub fn without(mut doc: Value, pointer: &str) -> Value {
    unset(&mut doc, pointer);
    doc
}

/// The example workflow of `spec/kernel.md` section 2.
pub fn review_loop() -> Value {
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "review-loop",
        "start": "build",
        "limits": {"maxVisitsPerNode": 16, "maxActivations": 256},
        "actions": {
            "build": {"kind": "command", "argv": ["make"], "effect": "idempotent",
                      "outcomes": ["ok", "red"],
                      "retry": {"maxAttempts": 3, "backoffMs": "500"}}
        },
        "nodes": {
            "build": {"kind": "activity", "action": "build",
                      "input": {"repo": {"$get": ["input", "repo"]}},
                      "outcomes": {"ok": "approve", "red": "build", "failed": "broken"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done", "rejected": "build"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "build", "output"]}},
            "broken": {"kind": "fail",
                       "error": {"code": "build.broken", "message": "never passed"}}
        }
    })
}

/// One activity `work` bound to action `act`, completing with the node's latest result.
/// With `failed_route` the node routes `failed` to a second complete node, `recover`.
pub fn single_activity(effect: &str, retry: Value, failed_route: bool) -> Value {
    let mut outcomes = json!({"ok": "done"});
    if failed_route {
        set(&mut outcomes, "/failed", json!("recover"));
    }
    json!({
        "format": WORKFLOW_FORMAT,
        "name": "single-activity",
        "start": "work",
        "actions": {"act": {"effect": effect, "retry": retry}},
        "nodes": {
            "work": {"kind": "activity", "action": "act", "outcomes": outcomes},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work"]}},
            "recover": {"kind": "complete", "output": {"$get": ["nodes", "work"]}}
        }
    })
}

/// A run driven one event at a time. A failed transition commits nothing.
pub struct Run {
    pub graph: Vec<u8>,
    pub limits: Limits,
    pub snapshot: Option<Vec<u8>>,
    pub sequence: u64,
    pub log: Vec<(Envelope, Decision)>,
}

impl Run {
    pub fn new(workflow: &Value) -> Self {
        Self {
            graph: canon(workflow),
            limits: Limits::default(),
            snapshot: None,
            sequence: 0,
            log: Vec::new(),
        }
    }

    /// A run whose `run.started` event has been applied at [`START_MS`].
    pub fn started(workflow: &Value, input: Value) -> Self {
        let mut run = Self::new(workflow);
        run.start(input);
        run
    }

    /// The request for the next event, without applying it.
    pub fn next(&self, kind: &str, payload: &Value, at: u64) -> TransitionRequest {
        let mut request = request(
            &self.graph,
            self.snapshot.as_deref(),
            envelope(self.sequence + 1, at, kind, payload),
        );
        request.frozen_limits = self.limits;
        request
    }

    pub fn apply(&mut self, kind: &str, payload: &Value, at: u64) -> Result<Decision, Failure> {
        let request = self.next(kind, payload, at);
        let decision = transition(&request)?;
        assert_eq!(
            decision.snapshot_digest,
            digest::state(&decision.snapshot),
            "snapshot digest is digest::state of the snapshot bytes"
        );
        self.snapshot = Some(decision.snapshot.clone());
        self.sequence += 1;
        self.log.push((request.input_event, decision.clone()));
        Ok(decision)
    }

    pub fn step(&mut self, kind: &str, payload: &Value, at: u64) -> Decision {
        self.apply(kind, payload, at)
            .unwrap_or_else(|failure| panic!("transition failed: {failure}"))
    }

    pub fn start(&mut self, input: Value) -> Decision {
        self.step(event_kind::RUN_STARTED, &started(input), START_MS)
    }

    pub fn state(&self) -> Value {
        parse(self.snapshot.as_deref().expect("run has a snapshot"))
    }

    pub fn status(&self) -> String {
        self.state()["status"]
            .as_str()
            .expect("status is a string")
            .to_string()
    }

    pub fn invocation_id(&self) -> String {
        self.state()["invocation"]["invocationId"]
            .as_str()
            .expect("an invocation is outstanding")
            .to_string()
    }

    pub fn attempt(&self) -> u32 {
        let attempt = self.state()["invocation"]["attempt"]
            .as_u64()
            .expect("an invocation is outstanding");
        u32::try_from(attempt).expect("attempt fits u32")
    }

    /// An `activity.result` payload for the attempt the state currently authorizes.
    pub fn result(&self, status: &str) -> Value {
        result_for(&self.invocation_id(), self.attempt(), status)
    }

    pub fn succeed(&mut self, outcome: &str, output: Value, at: u64) -> Decision {
        let mut payload = self.result("success");
        set(&mut payload, "/outcome", json!(outcome));
        set(&mut payload, "/output", output);
        self.step(event_kind::ACTIVITY_RESULT, &payload, at)
    }

    /// A `failure` result carrying the error `io.broken`.
    pub fn fail(&mut self, retryable: bool, at: u64) -> Decision {
        let mut payload = self.result("failure");
        set(&mut payload, "/retryable", json!(retryable));
        set(
            &mut payload,
            "/error",
            json!({"code": "io.broken", "message": "boom"}),
        );
        self.step(event_kind::ACTIVITY_RESULT, &payload, at)
    }

    /// An `unknown` or `expired` result.
    pub fn lose(&mut self, status: &str, at: u64) -> Decision {
        let payload = self.result(status);
        self.step(event_kind::ACTIVITY_RESULT, &payload, at)
    }

    pub fn signal(&mut self, name: &str, outcome: Option<&str>, data: Value, at: u64) -> Decision {
        self.step(
            event_kind::SIGNAL_RECEIVED,
            &json!({"name": name, "outcome": outcome, "data": data}),
            at,
        )
    }

    pub fn resolve(&mut self, invocation_id: &str, resolution: Value, at: u64) -> Decision {
        self.step(
            event_kind::INVOCATION_RESOLVED,
            &json!({"invocationId": invocation_id, "resolution": resolution}),
            at,
        )
    }
}

pub fn result_for(invocation_id: &str, attempt: u32, status: &str) -> Value {
    json!({
        "invocationId": invocation_id,
        "attemptId": ids::attempt(invocation_id, attempt),
        "attempt": attempt,
        "status": status
    })
}
