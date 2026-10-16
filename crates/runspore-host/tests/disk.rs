//! Disk pressure (spec/host.md §2): a worker on a full disk takes no new step, and a
//! try that ends on one is not counted.

mod common;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use runspore_host::disk::{Disk, Space};
use runspore_host::progress::Change;
use runspore_host::{
    ActivityOutput, ActivityRegistry, Engine, EngineConfig, NativeRunner, DISK_FULL,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::RunKey;
use serde_json::{json, Value};

const FLOOR: u64 = 1 << 30;
const PLENTY: u64 = 10 << 30;

/// One activity `work` with the given effect, routing `ok` to the end and `red` to a
/// `fix` end, so a red result shows in where the run ends.
fn single(effect: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2",
        "name": "single",
        "start": "work",
        "actions": {
            "work": {"kind": "native", "function": "work", "effect": effect,
                     "outcomes": ["ok", "red"]}
        },
        "nodes": {
            "work": {"kind": "activity", "action": "work", "input": {"$get": ["input"]},
                     "outcomes": {"ok": "done", "red": "fix"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}},
            "fix": {"kind": "fail", "error": {"code": "red", "message": "the tests failed"}}
        }
    }))
    .unwrap()
}

/// The free space every disk reports: what a test sets it to.
fn disk(free: Arc<AtomicU64>) -> Disk {
    Disk::with(move |_| {
        Ok(Space {
            free: free.load(Ordering::SeqCst),
            mount: "/data".into(),
        })
    })
}

struct Rig {
    engine: Engine,
    free: Arc<AtomicU64>,
    calls: Arc<AtomicUsize>,
    changes: Arc<Mutex<Vec<Change>>>,
}

/// An engine on a disk with `free` bytes whose `work` function fills the disk on its
/// first call and answers `red`, as `cargo test` does when SQLite gets `disk I/O
/// error`, and answers `ok` after that.
fn rig(name: &str, free: u64) -> Rig {
    let free = Arc::new(AtomicU64::new(free));
    let calls = Arc::new(AtomicUsize::new(0));
    let (filled, counted) = (free.clone(), calls.clone());
    let functions = NativeRunner::new().function("work", move |_ctx, _input| {
        let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            filled.store(0, Ordering::SeqCst);
        }
        async move {
            ActivityOutput::Success {
                outcome: if first { "red" } else { "ok" }.into(),
                output: json!({"first": first}),
            }
        }
    });
    let config = EngineConfig {
        guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
        poll_ms: 5,
        heartbeat_ms: 20,
        disk_floor: FLOOR,
        disk: disk(free.clone()),
        ..EngineConfig::default()
    };
    let engine = Engine::new(
        Arc::new(SqliteStore::open(temp_db(name)).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(functions),
        config,
    );
    let changes = Arc::new(Mutex::new(Vec::new()));
    let seen = changes.clone();
    engine.observe(Arc::new(move |p| {
        seen.lock().unwrap().push(p.change.clone())
    }));
    Rig {
        engine,
        free,
        calls,
        changes,
    }
}

/// Ticks until `done` holds, at most 5 s.
async fn tick_until(engine: &Engine, done: impl Fn() -> bool) {
    for _ in 0..1000 {
        engine.tick().await.unwrap();
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out");
}

fn paused(changes: &Mutex<Vec<Change>>) -> Vec<(Option<String>, String)> {
    changes
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Change::Paused { node_id, reason } => Some((node_id.clone(), reason.clone())),
            _ => None,
        })
        .collect()
}

async fn status(engine: &Engine, key: &RunKey) -> String {
    engine.get_run(key).await.unwrap().unwrap().status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_on_a_full_disk_takes_no_step_until_space_returns() {
    let rig = rig("disk-before", 0);
    let key = rig
        .engine
        .start(&single("idempotent"), json!({}), "k")
        .await
        .unwrap()
        .key;
    for _ in 0..5 {
        assert_eq!(rig.engine.tick().await.unwrap().dispatched, 0);
    }
    assert_eq!(rig.calls.load(Ordering::SeqCst), 0);
    let full = rig.engine.disk_full().expect("paused");
    assert_eq!(full.to_string(), "disk full (0 B free on /data)");
    // The run is told once per pause, not once per tick.
    let told = vec![(None, "disk full (0 B free on /data)".to_string())];
    assert_eq!(paused(&rig.changes), told);
    assert!(results(&rig.engine, &key).await.is_empty());

    rig.free.store(PLENTY, Ordering::SeqCst);
    assert_eq!(rig.engine.disk_full(), None);
    tick_until(&rig.engine, || rig.calls.load(Ordering::SeqCst) == 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_try_that_ends_on_a_full_disk_is_not_counted_and_runs_again_with_space() {
    let rig = rig("disk-after", PLENTY);
    let key = rig
        .engine
        .start(&single("idempotent"), json!({}), "k")
        .await
        .unwrap()
        .key;
    tick_until(&rig.engine, || !paused(&rig.changes).is_empty()).await;
    let told = vec![(
        Some("work".to_string()),
        "disk full (0 B free on /data)".to_string(),
    )];
    assert_eq!(paused(&rig.changes), told);
    // The red try is held back: no result, and the attempt keeps its lease.
    for _ in 0..10 {
        rig.engine.tick().await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(results(&rig.engine, &key).await.is_empty());
    assert_eq!(rig.calls.load(Ordering::SeqCst), 1);
    assert_eq!(rig.engine.in_flight(), 1);

    rig.free.store(PLENTY, Ordering::SeqCst);
    let view = rig.engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(rig.calls.load(Ordering::SeqCst), 2);
    let results = results(&rig.engine, &key).await;
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["attempt"], json!(1));
    assert_eq!(results[0]["outcome"], json!("ok"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsafe_try_that_ends_on_a_full_disk_is_left_to_a_person() {
    let rig = rig("disk-unsafe", PLENTY);
    let key = rig
        .engine
        .start(&single("unsafe"), json!({}), "k")
        .await
        .unwrap()
        .key;
    let engine = rig.engine.clone();
    let view = tokio::time::timeout(Duration::from_secs(10), engine.run_until_parked(&key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.status, "needs-intervention");
    assert_eq!(rig.calls.load(Ordering::SeqCst), 1);
    let results: Vec<Value> = results(&rig.engine, &key).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], json!("unknown"));
    assert_eq!(results[0]["error"]["code"], json!(DISK_FULL));
    assert_eq!(
        results[0]["error"]["message"],
        json!("the try ended on a disk full (0 B free on /data)")
    );
    assert_eq!(status(&rig.engine, &key).await, "needs-intervention");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_floor_of_zero_never_pauses() {
    let mut rig = rig("disk-zero", 0);
    let config = EngineConfig {
        disk_floor: 0,
        ..rig.engine.config().clone()
    };
    let functions = NativeRunner::new().function("work", |_ctx, _input| async {
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: Value::Null,
        }
    });
    rig.engine = Engine::new(
        Arc::new(SqliteStore::open(temp_db("disk-zero-2")).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(functions),
        config,
    );
    assert_eq!(rig.engine.disk_full(), None);
    let key = rig
        .engine
        .start(&single("idempotent"), json!({}), "k")
        .await
        .unwrap()
        .key;
    let view = rig.engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
}
