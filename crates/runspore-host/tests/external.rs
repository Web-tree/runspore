//! Steps outside the worker (spec/host.md §2, H27, H28): the real kernel and SQLite
//! store, an engine that never runs the step, and an actor that claims and reports it.

mod common;

use std::sync::Arc;

use common::*;
use runspore_host::{
    validate_workflow, ActivityRegistry, Engine, EngineConfig, NativeRunner, StepState,
};
use runspore_kernel::NativeReducer;
use runspore_store_conformance::ManualClock;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, InvocationStatus, RunKey};
use serde_json::{json, Value};

const T0: u64 = 1_000_000;

/// `review`, an external step for actor `reviewer` with a 60 s deadline and a 10 s
/// lease, then `done` on `approved` or `rework` on `changes`, which needs a list.
fn review(effect: &str, max_attempts: u32) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2",
        "name": "review",
        "start": "review",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": effect,
                       "outcomes": ["approved", "changes"],
                       "retry": {"maxAttempts": max_attempts},
                       "async": {"timeoutMs": "60000", "leaseMs": "10000"}}
        },
        "nodes": {
            "review": {"kind": "activity", "action": "review", "input": {"$get": ["input"]},
                       "outcomes": {"approved": "done", "changes": "rework"},
                       "outputSchemas": {"changes": {"type": "object", "required": ["list"],
                                                     "properties": {"list": {"type": "array"}}}}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "review", "output"]}},
            "rework": {"kind": "fail", "error": {"code": "review.changes", "message": "changes"}}
        }
    }))
    .unwrap()
}

struct World {
    engine: Engine,
    clock: Arc<ManualClock>,
    path: std::path::PathBuf,
    key: RunKey,
}

async fn world(name: &str, workflow: &[u8]) -> World {
    let path = temp_db(name);
    let clock = ManualClock::new(T0);
    let store = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    let engine = Engine::new(
        store,
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        EngineConfig {
            poll_ms: 5,
            ..EngineConfig::default()
        },
    );
    let key = engine
        .start(workflow, json!({"change": 7}), name)
        .await
        .unwrap()
        .key;
    World {
        engine,
        clock,
        path,
        key,
    }
}

impl World {
    async fn parked(&self) -> String {
        self.engine
            .run_until_parked(&self.key)
            .await
            .unwrap()
            .status
    }

    async fn step(&self) -> Option<runspore_host::ExternalStep> {
        let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
        self.engine.external_step(&view).await.unwrap()
    }

    async fn claim(&self) -> runspore_host::StepClaim {
        self.engine
            .claim_step(Some(&self.key.run), Some("reviewer"), None)
            .await
            .unwrap()
            .expect("a ready step")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h27_the_worker_never_runs_an_external_step_and_parks_there() {
    let w = world("h27-parks", &review("unsafe", 1)).await;
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.expect("at an external step");
    assert_eq!(
        (step.kind, step.actor.as_deref(), step.state, step.attempt),
        ("external", Some("reviewer"), StepState::Ready, 1)
    );
    assert_eq!((step.holder, step.deadline_ms), (None, None));
    let invocations = w.engine.list_invocations(&w.key).await.unwrap();
    assert_eq!(invocations[0].status, InvocationStatus::Pending);
    assert_eq!(w.engine.tick().await.unwrap().dispatched, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h27_only_a_claim_naming_the_label_takes_the_step() {
    let w = world("h27-label", &review("unsafe", 1)).await;
    w.parked().await;
    let e = &w.engine;
    assert_eq!(e.claim_step(None, None, None).await.unwrap(), None);
    let other = e.claim_step(Some(&w.key.run), Some("other"), None).await;
    assert_eq!(other.unwrap(), None);

    let holder = json!({"pid": 42});
    let claim = e
        .claim_step(None, Some("reviewer"), Some(holder.clone()))
        .await
        .unwrap()
        .expect("the oldest matching step");
    assert_eq!((&claim.key, claim.attempt), (&w.key, 1));
    assert_eq!(
        (claim.node_id.as_str(), claim.action_id.as_str()),
        ("review", "review")
    );
    assert_eq!(claim.input, json!({"change": 7}));
    assert_eq!(claim.outcomes, ["approved", "changes"]);
    assert_eq!(claim.output_schemas["changes"]["required"], json!(["list"]));
    assert_eq!(
        (
            claim.deadline_ms,
            claim.lease_until_ms,
            claim.result_channel
        ),
        (T0 + 60_000, T0 + 10_000, "store")
    );
    assert_eq!(
        (claim.files, claim.instructions, claim.work_dir),
        (None, None, None)
    );

    let step = w.step().await.unwrap();
    assert_eq!(
        (step.state, step.holder, step.deadline_ms),
        (StepState::Held, Some(holder), Some(T0 + 60_000))
    );
    let again = e.claim_step(None, Some("reviewer"), None).await.unwrap();
    assert_eq!(again, None, "a held step is not claimed twice");
    let large = json!("x".repeat(5_000));
    let refused = e.claim_step(None, Some("reviewer"), Some(large)).await;
    assert_eq!(refused.unwrap_err().code(), "holder.invalid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h27_a_completion_is_checked_before_anything_is_recorded() {
    let w = world("h27-complete", &review("unsafe", 1)).await;
    w.parked().await;
    let token = w.claim().await.token.to_string();
    let e = &w.engine;
    let refusals = [
        ("typo", Value::Null, "step.undeclared-outcome"),
        ("changes", Value::Null, "output.invalid"),
        (
            "approved",
            json!({"score": 1.5}),
            "step.output-not-canonical",
        ),
        (
            "approved",
            json!("x".repeat(70_000)),
            "step.output-too-large",
        ),
    ];
    for (outcome, output, code) in refusals {
        let refused = e.complete_step(&token, outcome, output).await.unwrap_err();
        assert_eq!(refused.code(), code, "{outcome}");
    }
    assert!(results(e, &w.key).await.is_empty(), "nothing was recorded");
    assert_eq!(w.step().await.unwrap().state, StepState::Held);
    let bad = e
        .complete_step("not a token", "approved", Value::Null)
        .await;
    assert_eq!(bad.unwrap_err().code(), "step.token-invalid");

    let output = json!({"note": "fine"});
    let done = e
        .complete_step(&token, "approved", output.clone())
        .await
        .unwrap();
    assert_eq!((done.attempt, done.disposition), (1, Disposition::Applied));
    let again = e
        .complete_step(&token, "approved", output.clone())
        .await
        .unwrap();
    assert_eq!(again.disposition, Disposition::Duplicate);
    assert_eq!(w.parked().await, "completed");
    let view = e.get_run(&w.key).await.unwrap().unwrap();
    let state: Value = serde_json::from_slice(&view.snapshot.unwrap()).unwrap();
    assert_eq!(state["result"]["output"], output);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h27_a_retryable_failure_brings_a_new_attempt() {
    let w = world("h27-fail", &review("idempotent", 2)).await;
    w.parked().await;
    let token = w.claim().await.token.to_string();
    let e = &w.engine;
    let bad = e.fail_step(&token, "no", Some("Bad Code"), true).await;
    assert_eq!(bad.unwrap_err().code(), "step.bad-code");
    e.fail_step(&token, "the tool crashed", None, true)
        .await
        .unwrap();
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.unwrap();
    assert_eq!((step.state, step.attempt), (StepState::Ready, 2));
    let result = &results(e, &w.key).await[0];
    assert_eq!(result["error"]["code"], "step.failed");

    let second = w.claim().await;
    assert_eq!(second.attempt, 2);
    e.fail_step(
        &second.token.to_string(),
        "still broken",
        Some("tool.crash"),
        true,
    )
    .await
    .unwrap();
    assert_eq!(w.parked().await, "needs-intervention", "the step is stuck");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h27_an_abandoned_unsafe_step_needs_intervention_and_a_late_report_is_kept() {
    let w = world("h27-abandon", &review("unsafe", 1)).await;
    w.parked().await;
    let e = &w.engine;
    assert_eq!(
        e.abandon_step(&w.key).await.unwrap_err().code(),
        "step.not-held"
    );
    let token = w.claim().await.token.to_string();
    let abandoned = e.abandon_step(&w.key).await.unwrap();
    assert_eq!(
        (abandoned.attempt, abandoned.disposition),
        (1, Disposition::Applied)
    );
    assert_eq!(w.parked().await, "needs-intervention");
    assert_eq!(expiry_audits(&w.path), [json!({"reason": "abandoned"})]);

    let late = e.complete_step(&token, "approved", Value::Null).await;
    assert_eq!(late.unwrap_err().code(), "lease.lost");
    assert_eq!(audits(&w.path), 1, "the late report is kept");
    let renew = e.renew_step(&token).await;
    assert_eq!(renew.unwrap_err().code(), "lease.lost");
    assert_eq!(results(e, &w.key).await.len(), 1, "only the expiry");
    assert_eq!(
        e.abandon_step(&w.key).await.unwrap_err().code(),
        "step.not-held"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h28_the_sweep_expires_a_lapsed_lease_and_a_passed_deadline() {
    let w = world("h28-repeatable", &review("idempotent", 3)).await;
    w.parked().await;
    let first = w.claim().await.token.to_string();
    let e = &w.engine;
    w.clock.set(T0 + 5_000);
    assert_eq!(
        e.renew_step(&first).await.unwrap().lease_until_ms,
        T0 + 15_000
    );
    w.clock.set(T0 + 15_000);
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.unwrap();
    assert_eq!(
        (step.state, step.attempt),
        (StepState::Ready, 2),
        "the lease lapsed"
    );

    let second = w.claim().await;
    assert_eq!(second.deadline_ms, T0 + 75_000);
    let token = second.token.to_string();
    let mut now = T0 + 15_000;
    while now + 9_000 < T0 + 75_000 {
        now += 9_000;
        w.clock.set(now);
        let renewed = e.renew_step(&token).await.unwrap();
        assert!(renewed.lease_until_ms <= T0 + 75_000, "past the deadline");
        assert_eq!(renewed.deadline_ms, Some(T0 + 75_000));
    }
    w.clock.set(T0 + 75_000);
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.unwrap();
    assert_eq!(
        (step.state, step.attempt),
        (StepState::Ready, 3),
        "the deadline passed"
    );

    let unsafe_run = world("h28-unsafe", &review("unsafe", 3)).await;
    unsafe_run.parked().await;
    unsafe_run.claim().await;
    unsafe_run.clock.set(T0 + 60_000);
    assert_eq!(unsafe_run.parked().await, "needs-intervention");
}

#[test]
fn validation_checks_an_external_actions_fields() {
    let check = |action: Value| {
        let mut workflow: Value = serde_json::from_slice(&review("unsafe", 1)).unwrap();
        workflow["actions"]["review"] = action;
        validate_workflow(
            &NativeReducer,
            &ActivityRegistry::with_builtins(NativeRunner::new()),
            EngineConfig::default().limits,
            &serde_json::to_vec(&workflow).unwrap(),
        )
        .map(|_| ())
        .map_err(|e| e.code().to_string())
    };
    let ok = json!({"kind": "external", "effect": "unsafe", "outcomes": ["approved", "changes"]});
    assert_eq!(check(ok.clone()), Ok(()));
    for (field, value) in [
        ("actor", json!("Reviewer")),
        ("async", json!({"timeoutMs": "1000", "leaseMs": "1000"})),
        ("async", json!({"timeoutMs": 1000})),
    ] {
        let mut bad = ok.clone();
        bad[field] = value;
        assert_eq!(check(bad), Err("action.invalid".to_string()), "{field}");
    }
}
