//! Process records: a command's record is stored before its program runs and cleared
//! after its group is stopped; the sweep stops a recorded group on this machine before
//! it expires the attempt; H16, a record from another host is expired unchecked.

mod common;

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::*;
use runspore_host::{
    ActivityContext, ActivityOutput, ActivityRunner, CommandRunner, EngineConfig, NativeRunner,
};
use runspore_store_conformance::{process_record, ManualClock};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{ProcessRecord, RunKey};
use serde_json::{json, Value};
use tokio::sync::watch;

/// What a test's recorder saw: each record, and whether the program had run by then.
type Seen = Arc<Mutex<Vec<(Option<ProcessRecord>, bool)>>>;

/// A context whose records go to `seen`, refused when `refuse` is set. `marker` is the
/// file the program writes, so the recorder can tell whether it ran yet.
fn ctx(stop: watch::Receiver<bool>, seen: Seen, marker: PathBuf, refuse: bool) -> ActivityContext {
    let mut ctx = ActivityContext::new(
        RunKey::new("default", "run-1"),
        "node-a".into(),
        "act".into(),
        "inv-1".into(),
        1,
        1,
        "effect-1".into(),
        std::env::temp_dir(),
        stop,
    );
    ctx.guard = env!("CARGO_BIN_EXE_runspore-guard").into();
    ctx.process = Arc::new(move |process| {
        seen.lock().unwrap().push((process, marker.exists()));
        Box::pin(async move {
            if refuse {
                Err("lease.lost".to_string())
            } else {
                Ok(())
            }
        })
    });
    ctx
}

fn marker(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("runspore-process-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// Writes `marker`, then prints its own process ID.
fn action(marker: &Path) -> Value {
    let script = format!("touch '{}'; echo $$", marker.display());
    json!({"kind": "command", "argv": ["sh", "-c", script]})
}

#[tokio::test]
async fn a_commands_record_is_stored_before_it_runs_and_cleared_after() {
    let marker = marker("stored");
    let seen = Seen::default();
    let (_stop, rx) = watch::channel(false);
    let ctx = ctx(rx, seen.clone(), marker.clone(), false);
    let out = CommandRunner.run(&ctx, &action(&marker), &json!({})).await;
    let ActivityOutput::Success { output, .. } = out else {
        panic!("expected success, got {out:?}");
    };
    let pid: i32 = output["stdout"].as_str().unwrap().trim().parse().unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    let (Some(record), ran) = &seen[0] else {
        panic!("first call cleared: {seen:?}");
    };
    assert!(!ran, "the program ran before its record was stored");
    assert_eq!(record.pgid, pid);
    assert_eq!(
        record.host,
        nix::unistd::gethostname().unwrap().to_string_lossy()
    );
    assert_eq!(seen[1], (None, true));
}

#[tokio::test]
async fn a_record_that_is_not_stored_fails_the_spawn_and_nothing_runs() {
    let marker = marker("refused");
    let seen = Seen::default();
    let (_stop, rx) = watch::channel(false);
    let ctx = ctx(rx, seen.clone(), marker.clone(), true);
    let out = CommandRunner.run(&ctx, &action(&marker), &json!({})).await;
    let ActivityOutput::Failure { error, retryable } = out else {
        panic!("expected failure, got {out:?}");
    };
    assert_eq!((error.code.as_str(), retryable), ("command.spawn", true));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!marker.exists(), "the program ran without a stored record");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h16_a_record_from_another_host_expires_unchecked() {
    let path = temp_db("h16");
    let clock = ManualClock::new(1_000_000);
    let store = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    let (recorded_tx, mut recorded_rx) = tokio::sync::mpsc::unbounded_channel();
    let native = NativeRunner::new().function("echo", move |ctx, _input| {
        let recorded = recorded_tx.clone();
        async move {
            ctx.record_process(Some(process_record("another-host", 4242)))
                .await
                .unwrap();
            recorded.send(()).unwrap();
            ctx.stopped().await;
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
    let key = engine.start(WORKFLOW, json!({}), "h16").await.unwrap().key;
    assert_eq!(engine.tick().await.unwrap().dispatched, 1);
    recorded_rx.recv().await.unwrap();
    let shown = engine.list_invocations(&key).await.unwrap();
    assert_eq!(
        shown[0].process.as_ref().map(|p| p.host.as_str()),
        Some("another-host")
    );
    clock.set(1_002_000);
    assert_eq!(engine.tick().await.unwrap().expired, 1);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(results(&engine, &key).await[0]["status"], "expired");
    assert_eq!(
        expiry_audits(&path),
        [json!({"reason": "lease", "notes": ["process.unchecked"]})]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sweep_stops_a_recorded_group_on_this_machine_before_it_expires() {
    let path = temp_db("leftover");
    let clock = ManualClock::new(1_000_000);
    let store = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    // A leftover group nobody waits for but the reaper thread, as init would.
    let mut leftover = std::process::Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let group = leftover.id() as i32;
    let record = runspore_host::process::record(group).unwrap();
    let reaper = std::thread::spawn(move || leftover.wait());
    let (recorded_tx, mut recorded_rx) = tokio::sync::mpsc::unbounded_channel();
    let native = NativeRunner::new().function("echo", move |ctx, _input| {
        let (recorded, record) = (recorded_tx.clone(), record.clone());
        async move {
            ctx.record_process(Some(record)).await.unwrap();
            recorded.send(()).unwrap();
            ctx.stopped().await;
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: Value::Null,
            }
        }
    });
    let config = EngineConfig {
        lease_ms: 1_000,
        heartbeat_ms: 10,
        stop_grace_ms: 2_000,
        ..EngineConfig::default()
    };
    let engine = engine(store, one_step, native, config);
    let key = engine
        .start(WORKFLOW, json!({}), "leftover")
        .await
        .unwrap()
        .key;
    assert_eq!(engine.tick().await.unwrap().dispatched, 1);
    recorded_rx.recv().await.unwrap();
    clock.set(1_002_000);
    assert_eq!(engine.tick().await.unwrap().expired, 1);
    assert_eq!(
        nix::sys::signal::killpg(nix::unistd::Pid::from_raw(group), None),
        Err(nix::errno::Errno::ESRCH),
        "the attempt expired while its group ran"
    );
    let _ = reaper.join();
    engine.run_until_parked(&key).await.unwrap();
    assert_eq!(expiry_audits(&path), [json!({"reason": "lease"})]);
}
