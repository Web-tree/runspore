//! Engine behaviour with a scripted reducer and the real SQLite store.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use runspore_host::{
    ActivityOutput, ActivityRegistry, Engine, EngineConfig, NativeRunner, TickReport,
};
use runspore_store_conformance::{schedule, ManualClock};
use runspore_store_sqlite::SqliteStore;
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::reducer::*;
use runspore_types::store::{Disposition, RunKey, Store};
use serde_json::{json, Value};

type Step = dyn Fn(&TransitionRequest) -> Result<Decision, Failure> + Send + Sync;

/// A reducer that answers with whatever its closure decides.
struct Scripted(Box<Step>);

impl Reducer for Scripted {
    fn describe(&self) -> Descriptor {
        Descriptor {
            abi_version: "test".into(),
            semantics_version: "test".into(),
            graph_format_version: "test".into(),
            codec_version: "test".into(),
        }
    }
    fn kernel_digest(&self) -> String {
        "scripted".into()
    }
    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        (self.0)(request)
    }
}

fn decision(request: &TransitionRequest, status: &str, commands: Vec<Command>) -> Decision {
    let seq = request.input_event.sequence;
    let snapshot = canonical::encode(&json!({"status": status, "seq": seq})).unwrap();
    Decision {
        snapshot_digest: digest::state(&snapshot),
        snapshot,
        commands,
        diagnostics: Vec::new(),
    }
}

fn run_key(request: &TransitionRequest) -> RunKey {
    let started: Value = serde_json::from_slice(&request.input_event.payload).unwrap();
    RunKey::new("default", started["runId"].as_str().unwrap_or_default())
}

/// `run.started` schedules node `a`; anything else completes the run.
fn one_step(request: &TransitionRequest) -> Result<Decision, Failure> {
    if request.input_event.kind == "run.started" {
        let (command, _) = schedule(&run_key(request), "a", 0);
        Ok(decision(request, "running", vec![command]))
    } else {
        Ok(decision(request, "completed", Vec::new()))
    }
}

const WORKFLOW: &[u8] = br#"{"actions": {"echo": {"kind": "native", "function": "echo"}}}"#;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runspore-host-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("store.db")
}

fn engine(
    store: Arc<dyn Store>,
    reducer: impl Fn(&TransitionRequest) -> Result<Decision, Failure> + Send + Sync + 'static,
    native: NativeRunner,
    config: EngineConfig,
) -> Engine {
    let reducer = Arc::new(Scripted(Box::new(reducer)));
    Engine::new(
        store,
        reducer,
        ActivityRegistry::with_builtins(native),
        config,
    )
}

fn counting_echo(count: Arc<AtomicUsize>) -> NativeRunner {
    NativeRunner::new().function("echo", move |_ctx, input| {
        count.fetch_add(1, Ordering::SeqCst);
        async move {
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: input,
            }
        }
    })
}

async fn results(engine: &Engine, key: &RunKey) -> Vec<Value> {
    let events = engine.list_events(key, 0, 100).await.unwrap().items;
    events
        .iter()
        .filter(|e| e.event.kind == "activity.result")
        .map(|e| serde_json::from_slice(&e.event.body).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_the_losing_coordinator_dispatches_nothing() {
    let path = temp_db("h2");
    let count = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let gate = Mutex::new(Some((entered_tx, go_rx)));
    let slow = move |request: &TransitionRequest| {
        if let Some((entered, go)) = gate.lock().unwrap().take() {
            entered.send(()).unwrap();
            go.recv().unwrap();
        }
        one_step(request)
    };
    let a = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        slow,
        counting_echo(count.clone()),
        EngineConfig::default(),
    );
    let b = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        one_step,
        counting_echo(count.clone()),
        EngineConfig::default(),
    );
    let started = b.start(WORKFLOW, json!({}), "h2").await.unwrap();
    let loser = tokio::spawn(async move { a.tick().await.unwrap() });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    let view = b.run_until_parked(&started.key).await.unwrap();
    assert_eq!(view.status, "completed");
    go_tx.send(()).unwrap();
    let report = loser.await.unwrap();
    assert_eq!(
        report,
        TickReport::default(),
        "the loser committed or dispatched"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(results(&b, &started.key).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h3_a_lost_lease_stops_the_runner_and_keeps_late_evidence() {
    let path = temp_db("h3");
    let clock = ManualClock::new(1_000_000);
    let store = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    let told_to_stop = Arc::new(AtomicBool::new(false));
    let flag = told_to_stop.clone();
    let native = NativeRunner::new().function("echo", move |ctx, _input| {
        let started = started_tx.clone();
        let flag = flag.clone();
        async move {
            started.send(()).unwrap();
            ctx.stopped().await;
            flag.store(true, Ordering::SeqCst);
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: Value::Null,
            }
        }
    });
    let config = EngineConfig {
        lease_ms: 1_000,
        heartbeat_ms: 10,
        ..EngineConfig::default()
    };
    let engine = engine(store, one_step, native, config);
    let key = engine.start(WORKFLOW, json!({}), "h3").await.unwrap().key;
    assert_eq!(engine.tick().await.unwrap().dispatched, 1);
    started_rx.recv().await.unwrap();
    clock.set(1_002_000);
    assert_eq!(engine.tick().await.unwrap().expired, 1);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert!(told_to_stop.load(Ordering::SeqCst));
    let results = results(&engine, &key).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "expired");
    let db = rusqlite::Connection::open(&path).unwrap();
    let audits: i64 = db
        .query_row("SELECT count(*) FROM audit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(audits, 1, "late evidence was not recorded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h4_a_kernel_failure_quarantines_without_spinning_and_release_resumes() {
    let path = temp_db("h4");
    let broken = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(AtomicUsize::new(0));
    let (b, c) = (broken.clone(), calls.clone());
    let reducer = move |request: &TransitionRequest| {
        c.fetch_add(1, Ordering::SeqCst);
        match request.input_event.kind.as_str() {
            "run.started" => Ok(decision(request, "waiting", Vec::new())),
            _ if b.load(Ordering::SeqCst) => Err(Failure {
                kind: FailureKind::ResourceLimit,
                code: "budget.microsteps".into(),
                details: "too many microsteps".into(),
            }),
            _ => Ok(decision(request, "completed", Vec::new())),
        }
    };
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let engine = engine(
        store,
        reducer,
        counting_echo(Arc::default()),
        EngineConfig::default(),
    );
    let key = engine.start(WORKFLOW, json!({}), "h4").await.unwrap().key;
    assert_eq!(
        engine.run_until_parked(&key).await.unwrap().status,
        "waiting"
    );
    engine
        .signal(&key, "m1", "go", None, Value::Null)
        .await
        .unwrap();
    assert_eq!(engine.tick().await.unwrap().quarantined, 1);
    let view = engine.get_run(&key).await.unwrap().unwrap();
    assert_eq!(view.quarantine.unwrap().code, "budget.microsteps");
    let events = engine.list_events(&key, 0, 10).await.unwrap().items;
    assert_eq!(events.last().unwrap().consumed_revision, None);
    let before = calls.load(Ordering::SeqCst);
    for _ in 0..3 {
        assert!(!engine.tick().await.unwrap().did_work());
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "the engine retried a quarantined run"
    );
    broken.store(false, Ordering::SeqCst);
    engine.release(&key).await.unwrap();
    assert_eq!(
        engine.run_until_parked(&key).await.unwrap().status,
        "completed"
    );
}

#[tokio::test]
async fn client_operations_are_idempotent_by_their_keys() {
    let path = temp_db("idem");
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let reducer = |r: &TransitionRequest| Ok(decision(r, "waiting", Vec::new()));
    let engine = engine(
        store,
        reducer,
        counting_echo(Arc::default()),
        EngineConfig::default(),
    );
    let first = engine
        .start(WORKFLOW, json!({"x": 1}), "same")
        .await
        .unwrap();
    let again = engine
        .start(WORKFLOW, json!({"x": 1}), "same")
        .await
        .unwrap();
    assert!(first.created && !again.created);
    assert_eq!(first.key, again.key);
    let s1 = engine
        .signal(&first.key, "m", "go", None, Value::Null)
        .await
        .unwrap();
    let s2 = engine
        .signal(&first.key, "m", "go", None, Value::Null)
        .await
        .unwrap();
    assert_eq!(s2.disposition, Disposition::Duplicate);
    assert_eq!(s1.value, s2.value);
    let resolution = runspore_types::model::Resolution::Retry;
    engine
        .resolve(&first.key, "r", "inv", resolution.clone())
        .await
        .unwrap();
    let r2 = engine
        .resolve(&first.key, "r", "inv", resolution)
        .await
        .unwrap();
    assert_eq!(r2.disposition, Disposition::Duplicate);
    assert_eq!(
        engine
            .list_events(&first.key, 0, 10)
            .await
            .unwrap()
            .items
            .len(),
        3
    );
}

#[tokio::test]
async fn start_rejects_unregistered_kinds_and_kernel_failures() {
    let path = temp_db("reject");
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let reducer = |_: &TransitionRequest| {
        Err(Failure {
            kind: FailureKind::InvalidInput,
            code: "graph.invalid".into(),
            details: String::new(),
        })
    };
    let engine = engine(store, reducer, NativeRunner::new(), EngineConfig::default());
    let bad = br#"{"actions": {"x": {"kind": "teleport"}}}"#;
    let err = engine.start(bad, json!({}), "k").await.unwrap_err();
    assert_eq!(err.code(), "action.kind-unregistered");
    let err = engine
        .start(br#"{"actions": {}}"#, json!({}), "k")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "graph.invalid");
    assert!(engine.list_runs(None, 10).await.unwrap().items.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_finishes_in_flight_attempts_on_shutdown() {
    let path = temp_db("serve");
    let store = Arc::new(SqliteStore::open(&path).unwrap());
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let started_tx = Mutex::new(Some(started_tx));
    let native = NativeRunner::new().function("echo", move |_ctx, _input| {
        let started = started_tx.lock().unwrap().take();
        async move {
            if let Some(started) = started {
                started.send(()).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: Value::Null,
            }
        }
    });
    let config = EngineConfig {
        poll_ms: 10,
        ..EngineConfig::default()
    };
    let engine = engine(store, one_step, native, config);
    let key = engine
        .start(WORKFLOW, json!({}), "serve")
        .await
        .unwrap()
        .key;
    engine
        .serve(async {
            started_rx.await.unwrap();
        })
        .await
        .unwrap();
    assert_eq!(engine.in_flight(), 0);
    let results = results(&engine, &key).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "success");
}
