//! SQLite-specific scenarios Q01, Q03 and Q04 of `spec/store.md` section 4.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{InvocationStatus, RunKey, Store, StoreFailureKind, DEFAULT_TENANT};

const OPS: [(&str, &str); 7] = [
    ("create-run", "create"),
    ("append-event", "append"),
    ("commit-turn", "commit"),
    ("claim-attempt", "claim"),
    ("heartbeat", "hb"),
    ("finish-attempt", "finish"),
    ("expire-attempt", "expire"),
];

fn fresh_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join("runspore-store-sqlite-q");
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(format!(
        "{}-{}.db",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ))
}

/// Runs the newest build of the crash helper example for `op`, with `failpoint` armed if given.
fn child(path: &Path, op: &str, failpoint: Option<&str>) -> std::process::Output {
    let examples = std::env::current_exe()
        .expect("exe")
        .parent()
        .and_then(Path::parent)
        .expect("target dir")
        .join("examples");
    let helper = std::fs::read_dir(&examples)
        .expect("examples dir")
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("crash-") && e.path().extension().is_none()
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).expect("mtime"))
        .expect("crash helper built by cargo test")
        .path();
    let mut command = Command::new(helper);
    command
        .arg(path)
        .arg(op)
        .env_remove(runspore_types::failpoint::ENV);
    if let Some(name) = failpoint {
        command.env(runspore_types::failpoint::ENV, name);
    }
    command.output().expect("run crash helper")
}

/// Whether the effect of `op` is visible in the reopened store.
async fn effect(store: &SqliteStore, op: &str) -> bool {
    let key = RunKey::new(DEFAULT_TENANT, "crash");
    let Some(run) = store.get_run(&key).await.unwrap() else {
        return false;
    };
    let events = store.list_events(&key, 0, 10).await.unwrap().items;
    let invocations = store.list_invocations(&key).await.unwrap();
    let status = invocations.first().map(|i| i.status);
    match op {
        "create-run" => events.len() == 1 && run.revision == 0,
        "append-event" => events.len() == 2,
        "commit-turn" => {
            run.revision == 1 && events[0].consumed_revision == Some(1) && invocations.len() == 1
        }
        "claim-attempt" => status == Some(InvocationStatus::Running),
        "heartbeat" => invocations[0].lease_until_ms == Some(1_200),
        _ => {
            status == Some(InvocationStatus::Settled)
                && events
                    .iter()
                    .filter(|e| e.event.kind == "activity.result")
                    .count()
                    == 1
        }
    }
}

#[tokio::test]
async fn q01_records_survive_reopen() {
    let path = fresh_path();
    assert!(child(&path, "finish-attempt", None).status.success());
    let store = SqliteStore::open(&path).unwrap();
    assert!(effect(&store, "finish-attempt").await);
    let key = RunKey::new(DEFAULT_TENANT, "crash");
    assert_eq!(store.list_events(&key, 0, 10).await.unwrap().items.len(), 3);
    for (_, request) in OPS
        .iter()
        .filter(|(op, _)| !["heartbeat", "expire-attempt"].contains(op))
    {
        assert!(
            store.get_receipt(&key, request).await.unwrap().is_some(),
            "receipt {request}"
        );
    }
}

#[tokio::test]
async fn q03_crash_at_every_failpoint() {
    let key = RunKey::new(DEFAULT_TENANT, "crash");
    for (op, request) in OPS {
        for position in ["before-commit", "after-commit"] {
            let path = fresh_path();
            let name = format!("store.{op}.{position}");
            let crashed = child(&path, op, Some(&name));
            assert!(!crashed.status.success(), "{name}: child did not abort");
            {
                let store = SqliteStore::open(&path).unwrap();
                let present = position == "after-commit";
                assert_eq!(effect(&store, op).await, present, "{name}: effect");
                assert_eq!(
                    store.get_receipt(&key, request).await.unwrap().is_some(),
                    present,
                    "{name}: receipt"
                );
            }
            let retried = child(&path, op, None);
            assert!(retried.status.success(), "{name}: retry failed");
            let expected = if position == "after-commit" {
                "duplicate"
            } else {
                "applied"
            };
            assert_eq!(
                String::from_utf8_lossy(&retried.stdout).trim(),
                expected,
                "{name}: disposition"
            );
            let store = SqliteStore::open(&path).unwrap();
            assert!(effect(&store, op).await, "{name}: converged");
        }
    }
}

#[tokio::test]
async fn q04_newer_schema_is_refused() {
    let path = fresh_path();
    drop(SqliteStore::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(
        None,
        "user_version",
        runspore_store_sqlite::SCHEMA_VERSION + 1,
    )
    .unwrap();
    drop(conn);
    let error = SqliteStore::open(&path).err().expect("refused");
    assert_eq!(
        (error.kind, error.code.as_str()),
        (StoreFailureKind::Incompatible, "schema.too-new")
    );
}
