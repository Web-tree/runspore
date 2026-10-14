//! A signal for the node a run waits at (spec/host.md §2, H30): an outcome the node does
//! not route, or `exhausted` in a 0.2 workflow, is refused with
//! `signal.outcome-unrouted` and records nothing; one sent before the run waits there is
//! recorded as before.

mod common;

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use common::*;
use runspore_host::{ActivityRegistry, Engine, EngineConfig, HostError, Unrouted};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, RunKey};
use serde_json::json;

/// `echo`, then the `approval` signal, which may reject once before it is exhausted.
fn workflow() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2",
        "name": "signal-routes",
        "start": "work",
        "actions": {"echo": {"kind": "native", "function": "echo", "effect": "idempotent"}},
        "nodes": {
            "work": {"kind": "activity", "action": "echo", "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval", "loopLimits": {"rejected": 1},
                        "outcomes": {"approved": "done", "rejected": "work",
                                     "exhausted": "stopped"}},
            "done": {"kind": "complete", "output": null},
            "stopped": {"kind": "fail", "error": {"code": "review.stopped", "message": "stopped"}}
        }
    }))
    .unwrap()
}

fn engine(name: &str) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(temp_db(name)).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(counting_echo(Arc::new(AtomicUsize::new(0)))),
        EngineConfig::default(),
    )
}

async fn start(engine: &Engine) -> RunKey {
    engine
        .start(&workflow(), json!(null), "k")
        .await
        .unwrap()
        .key
}

fn unrouted(result: Result<impl std::fmt::Debug, HostError>) -> Unrouted {
    match result {
        Err(HostError::OutcomeUnrouted(unrouted)) => unrouted,
        other => panic!("expected signal.outcome-unrouted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_node_refuses_an_outcome_it_does_not_route() {
    let engine = engine("signal-routes-waiting");
    let key = start(&engine).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "waiting");
    let recorded = events(&engine, &key).await.len();

    for (outcome, refused) in [
        (Some("approve"), "approve"),
        (Some("exhausted"), "exhausted"),
        (None, "received"),
    ] {
        let result = engine
            .signal(&key, "m", "approval", outcome.map(Into::into), json!(null))
            .await;
        let error = unrouted(result);
        assert_eq!(
            error,
            Unrouted {
                node_id: "approve".into(),
                signal: "approval".into(),
                outcome: refused.into(),
                outcomes: vec!["approved".into(), "rejected".into()],
            }
        );
        assert_eq!(
            HostError::OutcomeUnrouted(error).code(),
            "signal.outcome-unrouted"
        );
    }
    assert_eq!(
        events(&engine, &key).await.len(),
        recorded,
        "nothing was recorded"
    );

    // Another signal's name is not the waiting node's business.
    let other = engine
        .signal(&key, "o", "other", Some("approve".into()), json!(null))
        .await
        .unwrap();
    assert_eq!(other.disposition, Disposition::Applied);

    let approved = engine
        .signal(&key, "m", "approval", Some("approved".into()), json!(null))
        .await
        .unwrap();
    assert_eq!(approved.disposition, Disposition::Applied);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signal_before_the_run_waits_is_recorded_and_dropped() {
    let engine = engine("signal-routes-early");
    let key = start(&engine).await;
    let early = engine
        .signal(&key, "m", "approval", Some("approve".into()), json!(null))
        .await
        .unwrap();
    assert_eq!(early.disposition, Disposition::Applied);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "waiting");
    let dropped = events(&engine, &key)
        .await
        .iter()
        .flat_map(|e| e.diagnostics.iter())
        .any(|d| d.code == "signal.outcome-unrouted");
    assert!(dropped, "the kernel drops it at the node");
}
