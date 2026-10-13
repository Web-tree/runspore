//! SQLite-specific scenarios Q01 and Q03 to Q09 of `spec/store.md` section 4.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use runspore_store_conformance::{
    claim, create_run, expire, process_record, schedule, set_process, ManualClock,
};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{
    InvocationStatus, Mutation, RunKey, Store, StoreFailureKind, StoredArtifact, Turn,
    DEFAULT_TENANT,
};
use serde_json::{json, Value};

const OPS: [(&str, &str); 8] = [
    ("create-run", "create"),
    ("append-event", "append"),
    ("commit-turn", "commit"),
    ("claim-attempt", "claim"),
    ("heartbeat", "hb"),
    ("set-process", "process"),
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
        "set-process" => invocations[0].process.is_some(),
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
        .filter(|(op, _)| !["heartbeat", "set-process", "expire-attempt"].contains(op))
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

/// `PRAGMA user_version` of the file, read outside any store.
fn user_version(path: &Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn q04_newer_schema_is_refused_naming_its_writer() {
    let path = fresh_path();
    drop(SqliteStore::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let newer = runspore_store_sqlite::SCHEMA_VERSION + 1;
    conn.pragma_update(None, "user_version", newer).unwrap();
    conn.execute("UPDATE meta SET value = '9.9.9' WHERE key = 'writer'", [])
        .unwrap();
    drop(conn);
    let error = SqliteStore::open(&path).err().expect("refused");
    assert_eq!(
        (error.kind, error.code.as_str()),
        (StoreFailureKind::Incompatible, "schema.too-new")
    );
    for wanted in [
        "spore 9.9.9",
        runspore_store_sqlite::WRITER,
        "move the file aside",
    ] {
        assert!(
            error.message.contains(wanted),
            "{wanted}: {}",
            error.message
        );
    }
    assert_eq!(user_version(&path), newer, "the file is left as it was");

    // A newer schema that dropped the meta table is still refused, by its number.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE meta;").unwrap();
    drop(conn);
    let error = SqliteStore::open(&path).err().expect("refused");
    assert_eq!(error.code, "schema.too-new");
    assert!(
        error.message.contains(&format!("schema version {newer}")),
        "{}",
        error.message
    );
}

/// The audit records of `kind`, oldest first, as JSON.
fn audits(path: &Path, kind: &str) -> Vec<Value> {
    let db = rusqlite::Connection::open(path).unwrap();
    let mut stmt = db
        .prepare("SELECT evidence FROM audit WHERE kind = ?1 ORDER BY audit_id")
        .unwrap();
    stmt.query_map([kind], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|bytes| serde_json::from_slice(&bytes.unwrap()).unwrap())
        .collect()
}

/// A store at `path` with run `run`, one invocation of node `a` claimed with a lease of
/// 100 ms at time 1000, and the clock past that lease.
async fn expired_lease(path: &Path, run: &str) -> (SqliteStore, runspore_types::store::AttemptRef) {
    let clock = ManualClock::new(1_000);
    let store = SqliteStore::open_with_clock(path, clock.clone()).unwrap();
    let key = RunKey::new(DEFAULT_TENANT, run);
    store
        .create_run(create_run(&key, "create", 0))
        .await
        .unwrap();
    let event = store
        .list_events(&key, 0, 1)
        .await
        .unwrap()
        .items
        .remove(0)
        .event;
    let turn = Turn {
        key: key.clone(),
        revision: 0,
        applied_sequence: 0,
        package_digest: String::new(),
        snapshot: None,
        snapshot_digest: None,
        next_event: event,
    };
    let (command, invocation) = schedule(&key, "a", 0);
    store
        .commit_turn(runspore_store_conformance::commit(
            &turn,
            "commit",
            0,
            vec![command],
        ))
        .await
        .unwrap();
    let attempt = store
        .claim_attempt(claim(&key, "claim", &invocation, "w1", 100))
        .await
        .unwrap()
        .value
        .attempt;
    clock.set(1_100);
    (store, attempt)
}

#[tokio::test]
async fn q05_an_expiry_writes_its_audit_record() {
    let path = fresh_path();
    let (store, plain) = expired_lease(&path, "plain").await;
    store
        .expire_attempt(expire(&plain, "expire"))
        .await
        .unwrap();
    drop(store);
    let (store, attempt) = expired_lease(&path, "unchecked").await;
    let mut unchecked = expire(&attempt, "expire");
    unchecked.process_unchecked = true;
    unchecked.mutation =
        Mutation::new("expire_attempt", "expire", &json!([attempt, true])).unwrap();
    store.expire_attempt(unchecked).await.unwrap();
    assert_eq!(
        audits(&path, "attempt.expired"),
        [
            json!({"reason": "lease"}),
            json!({"reason": "lease", "notes": ["process.unchecked"]})
        ]
    );
    assert!(audits(&path, "late-evidence").is_empty());
}

/// Q06: a version 1 file is what `spore` 0.1 wrote. It is refused, not upgraded, and
/// left exactly as it was.
#[tokio::test]
async fn q06_a_version_1_file_is_refused_as_0_1() {
    let path = fresh_path();
    let (store, _attempt) = expired_lease(&path, "old").await;
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE attempts DROP COLUMN process;
         ALTER TABLE audit DROP COLUMN kind;
         ALTER TABLE runs DROP COLUMN base_dir;
         ALTER TABLE runs DROP COLUMN artifact_digest;
         DROP TABLE artifacts;
         DROP TABLE meta;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);
    let error = SqliteStore::open_with_clock(&path, ManualClock::new(1_050))
        .err()
        .expect("refused");
    assert_eq!(
        (error.kind, error.code.as_str()),
        (StoreFailureKind::Incompatible, "schema.too-old")
    );
    for wanted in ["spore 0.1", "0.1 binary", "move the file aside"] {
        assert!(
            error.message.contains(wanted),
            "{wanted}: {}",
            error.message
        );
    }
    assert_eq!(user_version(&path), 1, "the file is left as it was");
    let tables: Vec<String> = {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('meta', 'artifacts')")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(tables.is_empty(), "nothing was added: {tables:?}");
}

#[tokio::test]
async fn q07_a_version_2_file_is_upgraded() {
    let path = fresh_path();
    let store = SqliteStore::open_with_clock(&path, ManualClock::new(1_000)).unwrap();
    let old = RunKey::new(DEFAULT_TENANT, "old");
    store
        .create_run(create_run(&old, "create", 0))
        .await
        .unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE runs DROP COLUMN base_dir;
         ALTER TABLE runs DROP COLUMN artifact_digest;
         DROP TABLE artifacts;
         DROP TABLE meta;
         PRAGMA user_version = 2;",
    )
    .unwrap();
    drop(conn);
    let store = SqliteStore::open_with_clock(&path, ManualClock::new(1_000)).unwrap();
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store.get_run(&old).await.unwrap().expect("old run kept");
    assert_eq!((kept.revision, kept.base_dir), (0, None));
    let new = RunKey::new(DEFAULT_TENANT, "new");
    let mut request = create_run(&new, "create-new", 0);
    request.base_dir = Some("/repo".into());
    store.create_run(request).await.unwrap();
    let run = store.get_run(&new).await.unwrap().expect("new run");
    assert_eq!(run.base_dir.as_deref(), Some("/repo"));
}

#[tokio::test]
async fn q08_a_version_3_file_is_upgraded() {
    let path = fresh_path();
    let clock = ManualClock::new(1_000);
    let store = SqliteStore::open_with_clock(&path, clock.clone()).unwrap();
    let old = RunKey::new(DEFAULT_TENANT, "old");
    store
        .create_run(create_run(&old, "create", 0))
        .await
        .unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE runs DROP COLUMN artifact_digest;
         DROP TABLE artifacts;
         DROP TABLE meta;
         PRAGMA user_version = 3;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::open_with_clock(&path, clock).unwrap();
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store
        .get_run(&old)
        .await
        .unwrap()
        .expect("the old run is kept");
    assert_eq!(kept.artifact_digest, None);

    let new = RunKey::new(DEFAULT_TENANT, "new");
    let mut request = create_run(&new, "create", 1);
    let bytes = b"artifact".to_vec();
    request.artifact = Some(StoredArtifact {
        digest: runspore_types::digest::artifact(&bytes),
        bytes: bytes.clone(),
    });
    store.create_run(request).await.unwrap();
    let view = store.get_run(&new).await.unwrap().unwrap();
    let digest = view
        .artifact_digest
        .expect("the new run records its artifact");
    assert_eq!(store.get_artifact(&digest).await.unwrap(), Some(bytes));
}

#[tokio::test]
async fn q09_a_version_4_file_is_upgraded_and_records_its_writer() {
    let path = fresh_path();
    let store = SqliteStore::open_with_clock(&path, ManualClock::new(1_000)).unwrap();
    assert_eq!(
        store.writer().unwrap().as_deref(),
        Some(runspore_store_sqlite::WRITER),
        "a fresh file records its writer"
    );
    let old = RunKey::new(DEFAULT_TENANT, "old");
    store
        .create_run(create_run(&old, "create", 0))
        .await
        .unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE meta; PRAGMA user_version = 4;")
        .unwrap();
    drop(conn);

    let store = SqliteStore::open_with_clock(&path, ManualClock::new(1_000)).unwrap();
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    assert_eq!(
        store.writer().unwrap().as_deref(),
        Some(runspore_store_sqlite::WRITER)
    );
    let kept = store.get_run(&old).await.unwrap().expect("old run kept");
    assert_eq!(kept.revision, 0);
}
