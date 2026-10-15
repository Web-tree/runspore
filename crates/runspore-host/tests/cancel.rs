//! Cancelling a run (spec/host.md §2 `cancel`, §3 Async commands, Stop; H36): the
//! store ends the run, a worker's command stops at its next heartbeat, and a held
//! step's stop runs once. The real kernel, SQLite store and guard.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use runspore_host::{ActivityRegistry, Engine, EngineConfig, HostError, NativeRunner, Stopped};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, RunKey, STATUS_CANCELLED};
use serde_json::{json, Value};

/// Prints the holder handle `{"job": 7}` and keeps its token.
const START: &str = "cat > /dev/null; printf %s \"$RUNSPORE_TOKEN\" > token; echo '{\"job\": 7}'";

fn sh(script: &str) -> Value {
    json!(["/bin/sh", "-c", script])
}

/// One step `work` of `kind` with `extra` fields, then `done`.
fn workflow(kind: &str, extra: Value) -> Vec<u8> {
    let mut action = json!({"kind": kind, "effect": "idempotent", "outcomes": ["ok"]});
    for (k, v) in extra.as_object().unwrap() {
        action[k] = v.clone();
    }
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2", "name": "cancel", "start": "work",
        "actions": {"work": action},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "input": {"$get": ["input"]},
                     "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    }))
    .unwrap()
}

/// An async command whose start is `START` and whose stop is `stop`.
fn stopped_by(stop: &str) -> Vec<u8> {
    let spec = json!({"timeoutMs": "60000", "stop": {"command": {"argv": sh(stop)}}});
    workflow("command", json!({"argv": sh(START), "async": spec}))
}

struct World {
    engine: Engine,
    path: PathBuf,
    dir: PathBuf,
    key: RunKey,
}

fn config(path: &Path) -> EngineConfig {
    EngineConfig {
        poll_ms: 5,
        heartbeat_ms: 50,
        stop_grace_ms: 200,
        base_dir: path.parent().unwrap().to_path_buf(),
        db_path: Some(path.to_path_buf()),
        guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
        ..EngineConfig::default()
    }
}

fn engine_on(path: &Path) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(path).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        config(path),
    )
}

async fn world(name: &str, workflow: &[u8]) -> World {
    let path = temp_db(name);
    let dir = path.parent().unwrap().to_path_buf();
    let engine = engine_on(&path);
    let key = engine
        .start(workflow, json!({"change": 7}), name)
        .await
        .unwrap()
        .key;
    World {
        engine,
        path,
        dir,
        key,
    }
}

impl World {
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    async fn status(&self) -> String {
        self.engine
            .get_run(&self.key)
            .await
            .unwrap()
            .unwrap()
            .status
    }
}

/// Waits until `file` exists in `dir`, for at most 10 s.
async fn until_exists(dir: &Path, file: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.join(file).exists() {
        assert!(Instant::now() < deadline, "{file} never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .is_ok_and(|s| s.success())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h36_a_running_command_stops_at_its_next_heartbeat() {
    let script = "echo $$ > pid; sleep 30";
    let w = world(
        "h36-running",
        &workflow("command", json!({"argv": sh(script)})),
    )
    .await;
    let worker = w.engine.clone();
    let key = w.key.clone();
    let parked = tokio::spawn(async move { worker.run_until_parked(&key).await });
    until_exists(&w.dir, "pid").await;
    let pid = w.read("pid");
    assert!(alive(&pid), "the command runs");

    // Another process cancels: an engine of its own on the same file.
    let other = engine_on(&w.path);
    let cancelled_at = Instant::now();
    let cancelled = other
        .cancel(&w.key, "c1", Some("not needed"))
        .await
        .unwrap();
    assert_eq!(cancelled.disposition, Disposition::Applied);
    assert_eq!(cancelled.status, STATUS_CANCELLED);
    assert_eq!(
        cancelled.cancel.expect("record").reason.as_deref(),
        Some("not needed")
    );
    assert!(cancelled.stops.is_empty(), "a worker's command has no stop");

    let view = tokio::time::timeout(Duration::from_secs(5), parked)
        .await
        .expect("run_until_parked returns")
        .unwrap()
        .unwrap();
    assert_eq!(view.status, STATUS_CANCELLED);
    assert!(
        cancelled_at.elapsed() < Duration::from_millis(50 + 200 + 1_000),
        "stopped within heartbeat_ms + stop_grace_ms + 1 s"
    );
    assert!(!alive(&pid), "the command's group is gone");
    assert_eq!(expiry_audits(&w.path), [json!({"reason": "cancelled"})]);
    assert_eq!(audits(&w.path), 1, "what the stopped command said is kept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h36_a_held_async_command_runs_its_stop_once() {
    let stop = "cat >> stopped; echo >> stopped; printf %s \"$RUNSPORE_DB\" > db; echo bye";
    let w = world("h36-held", &stopped_by(stop)).await;
    let parked = w.engine.run_until_parked(&w.key).await.unwrap();
    assert_eq!(parked.status, "running", "held");

    let cancelled = w.engine.cancel(&w.key, "c1", None).await.unwrap();
    assert_eq!(cancelled.disposition, Disposition::Applied);
    let invocation = w.engine.list_invocations(&w.key).await.unwrap()[0]
        .invocation_id
        .clone();
    assert_eq!(
        cancelled.stops,
        [Stopped {
            invocation_id: invocation,
            attempt: 1,
            exit: Some(0)
        }]
    );
    assert_eq!(w.read("stopped"), "{\"job\":7}\n", "the handle on stdin");
    assert_eq!(w.read("db"), w.path.display().to_string());
    assert_eq!(expiry_audits(&w.path), [json!({"reason": "cancelled"})]);
    assert_eq!(audits(&w.path), 1, "the stop's output is late evidence");

    // The work's own report comes too late and is kept.
    let late = w
        .engine
        .complete_step(&w.read("token"), "ok", json!(null))
        .await;
    match late {
        Err(HostError::Store(f)) => assert_eq!(f.code, "lease.lost"),
        other => panic!("expected lease.lost, got {other:?}"),
    }
    assert_eq!(audits(&w.path), 2);

    // Neither a repeat nor another cancel runs the stop again.
    for request in ["c1", "c2"] {
        let again = w.engine.cancel(&w.key, request, None).await.unwrap();
        assert_eq!(again.disposition, Disposition::Duplicate, "{request}");
        assert!(again.stops.is_empty(), "{request}");
    }
    assert_eq!(w.read("stopped"), "{\"job\":7}\n", "it ran once");
    assert_eq!(
        w.engine.run_until_parked(&w.key).await.unwrap().status,
        STATUS_CANCELLED
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h36_a_failing_stop_is_ignored() {
    let w = world("h36-failing", &stopped_by("cat > /dev/null; exit 3")).await;
    assert_eq!(
        w.engine.run_until_parked(&w.key).await.unwrap().status,
        "running"
    );
    let cancelled = w.engine.cancel(&w.key, "c1", None).await.unwrap();
    assert_eq!(cancelled.stops[0].exit, Some(3));
    assert_eq!(w.status().await, STATUS_CANCELLED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h36_a_held_external_step_is_stopped_with_its_handle() {
    let stop = json!({"command": {"argv": sh("cat > stopped")}});
    let flow = workflow(
        "external",
        json!({"actor": "reviewer", "async": {"stop": stop}}),
    );
    let w = world("h36-external", &flow).await;
    assert_eq!(
        w.engine.run_until_parked(&w.key).await.unwrap().status,
        "running"
    );
    let claim = w
        .engine
        .claim_step(Some(&w.key.run), Some("reviewer"), Some(json!({"pane": 3})))
        .await
        .unwrap()
        .expect("ready");
    let cancelled = w.engine.cancel(&w.key, "c1", Some("gone")).await.unwrap();
    assert_eq!(cancelled.stops.len(), 1);
    assert_eq!(w.read("stopped"), r#"{"pane":3}"#);
    let token = claim.token.to_string();
    match w.engine.complete_step(&token, "ok", json!(null)).await {
        Err(HostError::Store(f)) => assert_eq!(f.code, "lease.lost"),
        other => panic!("expected lease.lost, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h36_an_ended_run_is_left_as_it_is() {
    let w = world("h36-ended", &workflow("command", json!({"argv": ["true"]}))).await;
    assert_eq!(
        w.engine.run_until_parked(&w.key).await.unwrap().status,
        "completed"
    );
    let before = w.engine.get_run(&w.key).await.unwrap().unwrap();
    let cancelled = w.engine.cancel(&w.key, "c1", Some("late")).await.unwrap();
    assert_eq!(
        (
            cancelled.disposition,
            cancelled.status.as_str(),
            cancelled.cancel
        ),
        (Disposition::Duplicate, "completed", None)
    );
    assert_eq!(w.engine.get_run(&w.key).await.unwrap().unwrap(), before);

    let missing = RunKey::new(&w.key.tenant, "run_missing");
    match w.engine.cancel(&missing, "c1", None).await {
        Err(HostError::Store(f)) => assert_eq!(f.code, "run.not-found"),
        other => panic!("expected run.not-found, got {other:?}"),
    }
}
