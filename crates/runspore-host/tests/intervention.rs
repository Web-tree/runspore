//! Telling a person (spec/host.md §2, H29): the workflow's `onIntervention` action runs
//! once each time a run enters `needs-intervention` or ends `failed`; its result never
//! moves the run, and its failure is an `on-intervention.failed` event.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use runspore_host::{
    validate_workflow, ActivityOutput, ActivityRegistry, Engine, EngineConfig, HostError,
    NativeRunner,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::model::Resolution;
use runspore_types::reducer::Limits;
use runspore_types::store::RunKey;
use serde_json::{json, Value};

/// `work`, which fails for good, then `done`. With `failed_route`, its failure ends the
/// run at the fail node `broken` instead of leaving the step stuck.
fn workflow(failed_route: bool) -> Value {
    let mut outcomes = json!({"ok": "done"});
    if failed_route {
        outcomes["failed"] = json!("broken");
    }
    let mut workflow = json!({
        "format": "runspore.workflow/0.2",
        "name": "intervention",
        "start": "work",
        "actions": {
            "work": {"kind": "native", "function": "work", "effect": "idempotent"},
            "tell": {"kind": "native", "function": "tell", "effect": "idempotent"}
        },
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": outcomes},
            "done": {"kind": "complete"}
        },
        "onIntervention": {"action": "tell",
                           "params": {"issue": {"$get": ["input", "issue"]}}}
    });
    if failed_route {
        workflow["nodes"]["broken"] = json!({"kind": "fail",
            "error": {"code": "work.broken", "message": "work gave up"}});
    }
    workflow
}

/// What `tell` saw on each call: its input, params, node and effect key.
type Calls = Arc<Mutex<Vec<Value>>>;

/// `work` fails without retry, or succeeds with `work_ok`; `tell` records its call and
/// fails with `tell.broken` when `tell_fails`.
fn functions(work_ok: bool, tell_fails: bool, calls: Calls) -> NativeRunner {
    NativeRunner::new()
        .function("work", move |_ctx, _input| async move {
            if work_ok {
                ActivityOutput::Success {
                    outcome: "ok".into(),
                    output: json!(null),
                }
            } else {
                ActivityOutput::Failure {
                    error: ActivityOutput::error("work.failed", "no luck", json!(null)),
                    retryable: false,
                }
            }
        })
        .function("tell", move |ctx, input| {
            calls.lock().unwrap().push(json!({
                "input": input, "params": ctx.params, "node": ctx.node_id,
                "effectKey": ctx.effect_key,
            }));
            async move {
                if tell_fails {
                    ActivityOutput::Failure {
                        error: ActivityOutput::error("tell.broken", "no network", json!(null)),
                        retryable: true,
                    }
                } else {
                    ActivityOutput::Success {
                        outcome: "ok".into(),
                        output: json!(null),
                    }
                }
            }
        })
}

fn engine(name: &str, functions: NativeRunner) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(temp_db(name)).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(functions),
        EngineConfig::default(),
    )
}

async fn start(engine: &Engine, workflow: &Value) -> RunKey {
    let bytes = serde_json::to_vec(workflow).unwrap();
    let input = json!({"issue": "acme/app#7"});
    engine.start(&bytes, input, "k").await.unwrap().key
}

fn kinds(events: &[runspore_types::store::EventRecord]) -> Vec<&str> {
    events.iter().map(|e| e.event.kind.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stuck_step_tells_once_with_the_run_the_node_and_the_error() {
    let calls = Calls::default();
    let engine = engine("intervention-stuck", functions(false, false, calls.clone()));
    let key = start(&engine, &workflow(false)).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    assert_eq!(
        seen[0]["input"],
        json!({
            "run": {"id": key.run, "tenant": "default"},
            "node": "work",
            "status": "needs-intervention",
            "reason": "stuck",
            "error": {"code": "work.failed", "message": "no luck", "nodeId": "work",
                      "details": null}
        })
    );
    assert_eq!(seen[0]["params"], json!({"issue": "acme/app#7"}));
    assert_eq!(seen[0]["node"], "work");
    // A success records nothing: the run's events are its own.
    let events = events(&engine, &key).await;
    assert_eq!(kinds(&events), ["run.started", "activity.result"]);
    // Working the run again tells nobody again.
    engine.tick().await.unwrap();
    engine.run_until_parked(&key).await.unwrap();
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_entry_into_needs_intervention_tells_again() {
    let calls = Calls::default();
    let engine = engine("intervention-again", functions(false, false, calls.clone()));
    let key = start(&engine, &workflow(false)).await;
    engine.run_until_parked(&key).await.unwrap();
    let invocation = engine.list_invocations(&key).await.unwrap()[0]
        .invocation_id
        .clone();
    engine
        .resolve(&key, "retry-1", &invocation, Resolution::Retry)
        .await
        .unwrap();
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    // Each entry is its own notice, with its own effect key.
    assert_ne!(seen[0]["effectKey"], seen[1]["effectKey"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_action_is_an_event_and_the_run_does_not_move() {
    let calls = Calls::default();
    let engine = engine("intervention-fails", functions(false, true, calls.clone()));
    let key = start(&engine, &workflow(false)).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    assert_eq!(calls.lock().unwrap().len(), 1, "a failure is not retried");
    let events = events(&engine, &key).await;
    assert_eq!(
        kinds(&events),
        ["run.started", "activity.result", "on-intervention.failed"]
    );
    let failed = &events[2];
    assert!(failed.consumed_revision.is_some(), "the kernel consumed it");
    assert!(failed.diagnostics.is_empty());
    let body: Value = serde_json::from_slice(&failed.event.body).unwrap();
    assert_eq!(body["action"], "tell");
    assert_eq!(body["status"], "needs-intervention");
    assert_eq!(body["error"]["code"], "tell.broken");
    // The step is still stuck, and resolving it works as before.
    let state: Value = serde_json::from_slice(view.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(state["invocation"]["state"], "stuck");
    assert_eq!(state["invocation"]["error"]["code"], "work.failed");
    assert_eq!(calls.lock().unwrap().len(), 1, "its own event tells nobody");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_run_tells_with_the_node_it_ended_at() {
    let calls = Calls::default();
    let engine = engine(
        "intervention-failed",
        functions(false, false, calls.clone()),
    );
    let key = start(&engine, &workflow(true)).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "failed");
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    let input = &seen[0]["input"];
    assert_eq!(input["status"], "failed");
    assert_eq!(input["reason"], "failed");
    assert_eq!(input["node"], "broken");
    assert_eq!(input["error"]["code"], "work.broken");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_completes_tells_nobody() {
    let calls = Calls::default();
    let engine = engine("intervention-none", functions(true, false, calls.clone()));
    let key = start(&engine, &workflow(false)).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn validate_refuses_an_on_intervention_the_worker_cannot_run() {
    let calls = Calls::default();
    let activities = ActivityRegistry::with_builtins(functions(false, false, calls));
    let refused = |workflow: Value| {
        let bytes = serde_json::to_vec(&workflow).unwrap();
        match validate_workflow(&NativeReducer, &activities, Limits::default(), &bytes) {
            Err(e) => e,
            Ok(_) => panic!("accepted {workflow}"),
        }
    };
    let mut unknown = workflow(false);
    unknown["onIntervention"]["action"] = json!("tel");
    let e = refused(unknown);
    assert_eq!(e.code(), "workflow.invalid");
    assert!(e.to_string().contains("`tel`"), "{e}");

    let mut external = workflow(false);
    external["actions"]["tell"] = json!({"kind": "external", "effect": "idempotent"});
    let e = refused(external);
    assert_eq!(e.code(), "workflow.invalid");
    assert!(e.to_string().contains("external"), "{e}");

    let mut root = workflow(false);
    root["onIntervention"]["params"] = json!({"x": {"$get": ["from", "node"]}});
    let e = refused(root);
    assert_eq!(e.code(), "workflow.invalid");
    assert!(e.to_string().contains("onIntervention.params"), "{e}");

    let mut old = workflow(false);
    old["format"] = json!("runspore.workflow/0.1");
    let e = refused(old);
    assert!(matches!(e, HostError::Kernel(_)), "{e:?}");
    assert!(e.to_string().contains("onIntervention"), "{e}");

    let bytes = serde_json::to_vec(&workflow(false)).unwrap();
    let activities = ActivityRegistry::with_builtins(functions(false, false, Calls::default()));
    validate_workflow(&NativeReducer, &activities, Limits::default(), &bytes).unwrap();
}
