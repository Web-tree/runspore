//! Engine behaviour with a scripted reducer and the real SQLite store.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use common::*;
use runspore_host::{ActivityOutput, EngineConfig, NativeRunner, RunIds, TickReport};
use runspore_store_conformance::ManualClock;
use runspore_store_sqlite::SqliteStore;
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::reducer::*;
use runspore_types::store::{
    Disposition, InvocationStatus, RunKey, RunParent, StoreFailure, StoreFailureKind,
};
use serde_json::{json, Value};

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

/// A reducer that parks every run in `waiting` with a snapshot carrying `tag`.
fn tagged(tag: &'static str) -> impl Fn(&TransitionRequest) -> Result<Decision, Failure> {
    move |request| {
        let seq = request.input_event.sequence;
        let state = json!({"status": "waiting", "seq": seq, "tag": tag});
        let snapshot = canonical::encode(&state).unwrap();
        Ok(Decision {
            snapshot_digest: digest::state(&snapshot),
            snapshot,
            commands: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_different_decision_for_a_committed_event_is_counted_as_diverged() {
    let path = temp_db("diverged");
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let gate = Mutex::new(Some((entered_tx, go_rx)));
    let ours = tagged("a");
    let slow = move |request: &TransitionRequest| {
        if let Some((entered, go)) = gate.lock().unwrap().take() {
            entered.send(()).unwrap();
            go.recv().unwrap();
        }
        ours(request)
    };
    let a = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        slow,
        counting_echo(Arc::default()),
        EngineConfig::default(),
    );
    let b = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        tagged("b"),
        counting_echo(Arc::default()),
        EngineConfig::default(),
    );
    let key = b.start(WORKFLOW, json!({}), "diverged").await.unwrap().key;
    let loser = tokio::spawn(async move { a.tick().await.unwrap() });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    assert_eq!(b.tick().await.unwrap().turns, 1);
    go_tx.send(()).unwrap();
    let report = loser.await.unwrap();
    assert_eq!(
        report,
        TickReport {
            diverged: 1,
            ..TickReport::default()
        }
    );
    assert!(report.did_work());
    let view = b.get_run(&key).await.unwrap().unwrap();
    assert_eq!(view.revision, 1);
    let snapshot: Value = serde_json::from_slice(&view.snapshot.unwrap()).unwrap();
    assert_eq!(
        snapshot["tag"], "b",
        "the diverging decision replaced the committed one"
    );
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
    assert_eq!(audits(&path), 1, "late evidence was not recorded");
}

/// Answers every `get_run` with a `corrupt` failure while armed, and counts them.
struct Unreadable(AtomicBool, AtomicUsize);

impl Hook for Unreadable {
    fn get_run(&self) -> Option<StoreFailure> {
        self.0.load(Ordering::SeqCst).then(|| {
            self.1.fetch_add(1, Ordering::SeqCst);
            StoreFailure::new(StoreFailureKind::Corrupt, "db.corrupt", "unreadable")
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attempt_whose_run_cannot_be_read_is_abandoned_to_lease_expiry() {
    let path = temp_db("abandon");
    let clock = ManualClock::new(1_000_000);
    let store = Arc::new(Hooked {
        inner: Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap()),
        hook: Unreadable(AtomicBool::new(false), AtomicUsize::new(0)),
    });
    let count = Arc::new(AtomicUsize::new(0));
    let config = EngineConfig {
        lease_ms: 1_000,
        ..EngineConfig::default()
    };
    let engine = engine(
        store.clone(),
        one_step,
        counting_echo(count.clone()),
        config,
    );
    let key = engine
        .start(WORKFLOW, json!({}), "abandon")
        .await
        .unwrap()
        .key;
    store.hook.0.store(true, Ordering::SeqCst);
    assert_eq!(engine.tick().await.unwrap().dispatched, 1);
    while engine.in_flight() > 0 {
        tokio::task::yield_now().await;
    }
    store.hook.0.store(false, Ordering::SeqCst);
    assert!(
        store.hook.1.load(Ordering::SeqCst) > 0,
        "the attempt never read its run"
    );
    assert!(
        results(&engine, &key).await.is_empty(),
        "a result was reported"
    );
    assert_eq!(audits(&path), 0, "late evidence was recorded");
    let invocations = engine.list_invocations(&key).await.unwrap();
    assert_eq!(invocations[0].status, InvocationStatus::Running);
    clock.set(1_002_000);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let results = results(&engine, &key).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "expired");
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

/// Starts `key` twice in a fresh store, runs it, and returns the run key and the IDs of
/// its invocations, which are also its effect keys.
async fn start_twice_and_run(name: &str, key: &str) -> (RunKey, Vec<String>) {
    let store = Arc::new(SqliteStore::open(temp_db(name)).unwrap());
    let engine = engine(
        store,
        one_step,
        counting_echo(Arc::default()),
        EngineConfig::default(),
    );
    let first = engine.start(WORKFLOW, json!({}), key).await.unwrap();
    let again = engine.start(WORKFLOW, json!({}), key).await.unwrap();
    assert!(first.created && !again.created);
    assert_eq!(first.key, again.key);
    let view = engine.run_until_parked(&first.key).await.unwrap();
    assert_eq!(view.status, "completed");
    let invocations = engine.list_invocations(&first.key).await.unwrap();
    (
        first.key,
        invocations.into_iter().map(|i| i.invocation_id).collect(),
    )
}

#[tokio::test]
async fn a_start_key_names_one_run_per_store_and_run_ids_are_random() {
    let (a, effects_a) = start_twice_and_run("key-a", "same").await;
    let (b, effects_b) = start_twice_and_run("key-b", "same").await;
    for key in [&a, &b] {
        let hex = key.run.strip_prefix("run_").unwrap();
        assert_eq!(hex.len(), 32);
        assert!(hex
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    }
    assert_ne!(a.run, b.run);
    assert_eq!((effects_a.len(), effects_b.len()), (1, 1));
    assert_ne!(effects_a, effects_b);
}

/// H38: working one run claims none of another's due steps, so a caller that exits when
/// its run parks cuts off no other run's try.
#[tokio::test]
async fn run_until_parked_claims_only_its_own_runs_steps() {
    let store: Arc<SqliteStore> = Arc::new(SqliteStore::open(temp_db("only-its-own")).unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let engine = engine(
        store.clone(),
        one_step,
        counting_echo(count.clone()),
        EngineConfig::default(),
    );
    let other = engine
        .start(WORKFLOW, json!({}), "other")
        .await
        .unwrap()
        .key;
    // An engine with no slot takes the other run's first turn: its step is due.
    let no_slot = EngineConfig {
        max_concurrent_activities: 0,
        ..EngineConfig::default()
    };
    let coordinator = common::engine(store, one_step, NativeRunner::new(), no_slot);
    coordinator.tick().await.unwrap();
    let due = engine.list_invocations(&other).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].status, InvocationStatus::Pending);

    let mine = engine.start(WORKFLOW, json!({}), "mine").await.unwrap().key;
    let view = engine.run_until_parked(&mine).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let due = engine.list_invocations(&other).await.unwrap();
    assert_eq!(due[0].status, InvocationStatus::Pending);
    assert!(due[0].attempt.is_none(), "the other run's step was claimed");
    let view = engine.run_until_parked(&other).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

/// `run.started` with input `{"wait": true}` leaves the run waiting for a signal; any
/// other run takes `one_step`.
fn waits_when_asked(request: &TransitionRequest) -> Result<Decision, Failure> {
    let started: Value = serde_json::from_slice(&request.input_event.payload).unwrap();
    if request.input_event.kind == "run.started" && started["input"]["wait"] == true {
        return Ok(decision(request, "waiting", Vec::new()));
    }
    one_step(request)
}

/// H39: working a run works the runs it started and theirs, as Temporal's worker runs a
/// workflow's child workflows, and still claims nothing of a run outside its tree.
#[tokio::test]
async fn run_until_parked_works_the_runs_below_the_run() {
    let path = temp_db("below");
    let count = Arc::new(AtomicUsize::new(0));
    let engine = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        waits_when_asked,
        counting_echo(count.clone()),
        EngineConfig::default(),
    );
    let parent = engine
        .start(WORKFLOW, json!({"wait": true}), "parent")
        .await
        .unwrap()
        .key;
    let started_in = |run: &RunKey| {
        let config = EngineConfig {
            parent: Some(RunParent {
                run_id: run.run.clone(),
                node_id: "spawn".into(),
            }),
            ..EngineConfig::default()
        };
        common::engine(
            Arc::new(SqliteStore::open(&path).unwrap()),
            waits_when_asked,
            counting_echo(count.clone()),
            config,
        )
    };
    let child = started_in(&parent)
        .start(WORKFLOW, json!({}), "child")
        .await
        .unwrap()
        .key;
    let grandchild = started_in(&child)
        .start(WORKFLOW, json!({}), "grandchild")
        .await
        .unwrap()
        .key;
    let other = engine
        .start(WORKFLOW, json!({}), "other")
        .await
        .unwrap()
        .key;

    let view = engine.run_until_parked(&parent).await.unwrap();
    assert_eq!(view.status, "waiting");
    for below in [&child, &grandchild] {
        let below = engine.get_run(below).await.unwrap().unwrap();
        assert_eq!(below.status, "completed", "{}", below.key.run);
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let view = engine.get_run(&other).await.unwrap().unwrap();
    assert_eq!(view.status, "created", "a run outside the tree took a turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_starts_with_one_key_end_on_one_run() {
    let path = temp_db("start-race");
    // `a` looks the key up, misses, and stops in its dry run until `b` has started.
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
    let config = |run: &str| EngineConfig {
        run_ids: RunIds::fixed([run]),
        ..EngineConfig::default()
    };
    let a = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        slow,
        counting_echo(Arc::default()),
        config("run_a"),
    );
    let b = engine(
        Arc::new(SqliteStore::open(&path).unwrap()),
        one_step,
        counting_echo(Arc::default()),
        config("run_b"),
    );
    let late = tokio::spawn(async move { a.start(WORKFLOW, json!({}), "race").await });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();
    let won = b.start(WORKFLOW, json!({}), "race").await.unwrap();
    go_tx.send(()).unwrap();
    let lost = late.await.unwrap().unwrap();
    assert!(won.created && !lost.created);
    assert_eq!(won.key.run, "run_b");
    assert_eq!(lost.key, won.key);
    assert_eq!(b.list_runs(None, 10).await.unwrap().items.len(), 1);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_finishes_an_attempt_that_returns_after_the_stop_signal() {
    let path = temp_db("serve-stop");
    let clock = ManualClock::new(1_000_000);
    let store = Arc::new(SqliteStore::open_with_clock(&path, clock).unwrap());
    let native = NativeRunner::new().function("echo", move |ctx, _input| async move {
        ctx.stopped().await;
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: json!("finished anyway"),
        }
    });
    let config = EngineConfig {
        shutdown_grace_ms: 20,
        ..EngineConfig::default()
    };
    let engine = engine(store, one_step, native, config);
    let key = engine.start(WORKFLOW, json!({}), "stop").await.unwrap().key;
    engine.serve(std::future::ready(())).await.unwrap();
    assert_eq!(engine.in_flight(), 0);
    let results = results(&engine, &key).await;
    assert_eq!(results.len(), 1, "the stopped attempt was not finished");
    assert_eq!(results[0]["status"], "success");
    assert_eq!(results[0]["output"], "finished anyway");
    assert_eq!(audits(&path), 0);
    let view = engine.get_run(&key).await.unwrap().unwrap();
    assert_eq!(
        view.status, "completed",
        "the drained result was not applied"
    );
}
