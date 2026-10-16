//! SQLite-specific scenarios Q01 and Q03 to Q19 of `spec/store.md` section 4.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use runspore_store_conformance::{
    cancel, claim, claim_held, create_run, expire, expire_for, schedule, schedule_with, ManualClock,
};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{
    ExpireReason, InvocationStatus, Mutation, RunKey, RunParent, Store, StoreFailureKind,
    StoredArtifact, Turn, DEFAULT_TENANT,
};
use serde_json::{json, Value};

const OPS: [(&str, &str); 9] = [
    ("create-run", "create"),
    ("append-event", "append"),
    ("commit-turn", "commit"),
    ("claim-attempt", "claim"),
    ("heartbeat", "hb"),
    ("set-process", "process"),
    ("finish-attempt", "finish"),
    ("expire-attempt", "expire"),
    ("cancel-run", "cancel"),
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
        "cancel-run" => run.status == "cancelled" && status == Some(InvocationStatus::Settled),
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
    for (_, request) in OPS.iter().filter(|(op, _)| {
        !["heartbeat", "set-process", "expire-attempt", "cancel-run"].contains(op)
    }) {
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
    drop(store);
    let (store, attempt) = expired_lease(&path, "abandoned").await;
    store
        .expire_attempt(expire_for(&attempt, "expire", ExpireReason::Abandoned))
        .await
        .unwrap();
    assert_eq!(
        audits(&path, "attempt.expired"),
        [
            json!({"reason": "lease"}),
            json!({"reason": "lease", "notes": ["process.unchecked"]}),
            json!({"reason": "abandoned"})
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
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         ALTER TABLE attempts DROP COLUMN process;
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
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         ALTER TABLE runs DROP COLUMN base_dir;
         ALTER TABLE runs DROP COLUMN artifact_digest;
         DROP TABLE artifacts;
         DROP TABLE meta;
         PRAGMA user_version = 2;",
    )
    .unwrap();
    drop(conn);
    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_000)).unwrap().0;
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
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         ALTER TABLE runs DROP COLUMN artifact_digest;
         DROP TABLE artifacts;
         DROP TABLE meta;
         PRAGMA user_version = 3;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, clock).unwrap().0;
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
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         DROP TABLE meta;
         PRAGMA user_version = 4;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_000)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    assert_eq!(
        store.writer().unwrap().as_deref(),
        Some(runspore_store_sqlite::WRITER)
    );
    let kept = store.get_run(&old).await.unwrap().expect("old run kept");
    assert_eq!(kept.revision, 0);
}

/// Q10: a version 5 file gains an attempt's holder handle and deadline; the attempts it
/// holds keep neither.
#[tokio::test]
async fn q10_a_version_5_file_is_upgraded() {
    let path = fresh_path();
    let (store, attempt) = expired_lease(&path, "old").await;
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         PRAGMA user_version = 5;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_100)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let shown = store
        .list_invocations(&attempt.key)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        (shown.attempt.as_ref(), shown.holder, shown.deadline_ms),
        (Some(&attempt), None, None)
    );
    store
        .expire_attempt(expire(&attempt, "expire"))
        .await
        .unwrap();

    let new = RunKey::new(DEFAULT_TENANT, "new");
    store
        .create_run(create_run(&new, "create", 1))
        .await
        .unwrap();
    let turn = store.load_turn(&new).await.unwrap().unwrap();
    let (command, invocation) = schedule(&new, "a", 0);
    let commit = runspore_store_conformance::commit(&turn, "commit", 0, vec![command]);
    store.commit_turn(commit).await.unwrap();
    let holder = Some(json!({"pid": 1}));
    let held = store
        .claim_attempt(claim_held(&new, "claim", &invocation, 100, holder, 50))
        .await
        .unwrap()
        .value;
    assert_eq!(held.deadline_ms, Some(1_150));
    let shown = store.list_invocations(&new).await.unwrap().remove(0);
    assert_eq!(shown.holder, Some(b"{\"pid\":1}".to_vec()));
}

/// Q12: a version 6 file gains an invocation's params; the invocations it holds have
/// none, and a new schedule records them.
#[tokio::test]
async fn q12_a_version_6_file_is_upgraded() {
    let path = fresh_path();
    let store = SqliteStore::open_with_clock(&path, ManualClock::new(1_000)).unwrap();
    let old = RunKey::new(DEFAULT_TENANT, "old");
    store
        .create_run(create_run(&old, "create", 0))
        .await
        .unwrap();
    let turn = store.load_turn(&old).await.unwrap().unwrap();
    let (command, scheduled) = schedule(&old, "a", 0);
    let commit = runspore_store_conformance::commit(&turn, "commit", 0, vec![command]);
    store.commit_turn(commit).await.unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         PRAGMA user_version = 6;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_100)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store
        .claim_attempt(claim(&old, "claim", &scheduled, "w", 100))
        .await
        .unwrap()
        .value;
    assert_eq!(kept.params, None);

    let new = RunKey::new(DEFAULT_TENANT, "new");
    store
        .create_run(create_run(&new, "create", 1))
        .await
        .unwrap();
    let turn = store.load_turn(&new).await.unwrap().unwrap();
    let (command, invocation) = schedule_with(&new, "a", 0, Some(json!({"tools": ["Read"]})));
    let commit = runspore_store_conformance::commit(&turn, "commit", 0, vec![command]);
    store.commit_turn(commit).await.unwrap();
    let claimed = store
        .claim_attempt(claim(&new, "claim", &invocation, "w", 100))
        .await
        .unwrap()
        .value;
    assert_eq!(claimed.params, Some(br#"{"tools":["Read"]}"#.to_vec()));
}

/// Q13: a version 7 file gains the run's parent; the runs it holds have none, and a
/// new run records its own.
#[tokio::test]
async fn q13_a_version_7_file_is_upgraded() {
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
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         PRAGMA user_version = 7;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_000)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store.get_run(&old).await.unwrap().expect("old run kept");
    assert_eq!((kept.revision, kept.parent), (0, None));
    let new = RunKey::new(DEFAULT_TENANT, "new");
    let mut request = create_run(&new, "create-new", 0);
    let parent = RunParent {
        run_id: old.run.clone(),
        node_id: "ticket".into(),
    };
    request.parent = Some(parent.clone());
    store.create_run(request).await.unwrap();
    let run = store.get_run(&new).await.unwrap().expect("new run");
    assert_eq!(run.parent, Some(parent));
}

/// Q14: a version 8 file gains when an attempt was claimed; the attempts it holds show
/// none, and a new claim records its time.
#[tokio::test]
async fn q14_a_version_8_file_is_upgraded() {
    let path = fresh_path();
    let (store, attempt) = expired_lease(&path, "old").await;
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         PRAGMA user_version = 8;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_100)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let shown = store
        .list_invocations(&attempt.key)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        (shown.attempt.as_ref(), shown.claimed_at_ms),
        (Some(&attempt), None)
    );

    let new = RunKey::new(DEFAULT_TENANT, "new");
    store
        .create_run(create_run(&new, "create", 1))
        .await
        .unwrap();
    let turn = store.load_turn(&new).await.unwrap().unwrap();
    let (command, invocation) = schedule(&new, "a", 0);
    let commit = runspore_store_conformance::commit(&turn, "commit", 0, vec![command]);
    store.commit_turn(commit).await.unwrap();
    store
        .claim_attempt(claim(&new, "claim", &invocation, "w", 100))
        .await
        .unwrap();
    let shown = store.list_invocations(&new).await.unwrap().remove(0);
    assert_eq!(shown.claimed_at_ms, Some(1_100));
}

/// Q15: a version 9 file gains the run's result channels; the runs it holds have none,
/// and a new run records its own.
#[tokio::test]
async fn q15_a_version_9_file_is_upgraded() {
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
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         PRAGMA user_version = 9;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_000)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store.get_run(&old).await.unwrap().expect("old run kept");
    assert!(kept.result_channels.is_empty());
    let new = RunKey::new(DEFAULT_TENANT, "new");
    let mut request = create_run(&new, "create-new", 0);
    request.result_channels = BTreeMap::from([("review".into(), "store".into())]);
    store.create_run(request.clone()).await.unwrap();
    let run = store.get_run(&new).await.unwrap().expect("new run");
    assert_eq!(run.result_channels, request.result_channels);
}

/// Q16: a version 10 file gains the run's cancel record; the runs it holds have none,
/// and one of them can then be cancelled, its running attempt expired with reason
/// `cancelled`.
#[tokio::test]
async fn q16_a_version_10_file_is_upgraded() {
    let path = fresh_path();
    let (store, attempt) = expired_lease(&path, "old").await;
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         PRAGMA user_version = 10;",
    )
    .unwrap();
    drop(conn);

    let store = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_200)).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    let kept = store
        .get_run(&attempt.key)
        .await
        .unwrap()
        .expect("old run kept");
    assert_eq!((kept.status.as_str(), kept.cancel), ("running", None));
    let cancelled = store
        .cancel_run(cancel(&attempt.key, "cancel", Some("old")))
        .await
        .unwrap();
    assert_eq!(cancelled.value.expired.len(), 1);
    let run = store.get_run(&attempt.key).await.unwrap().unwrap();
    assert_eq!(run.status, "cancelled");
    assert_eq!(run.cancel.expect("record").at_ms, 1_200);
    drop(store);
    assert_eq!(
        audits(&path, "attempt.expired"),
        [json!({"reason": "cancelled"})]
    );
}

/// Q11: an upgrade that stops after one of its steps leaves the file at its old
/// version, with nothing of the upgrade in it; the next open upgrades it whole.
#[tokio::test]
async fn q11_an_upgrade_stopped_part_way_leaves_the_old_version() {
    let path = fresh_path();
    assert!(child(&path, "create-run", None).status.success());
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         ALTER TABLE runs DROP COLUMN cancel_reason;
         ALTER TABLE runs DROP COLUMN cancel_at_ms;
         ALTER TABLE runs DROP COLUMN cancel_request_id;
         ALTER TABLE runs DROP COLUMN result_channels;
         ALTER TABLE attempts DROP COLUMN claimed_at_ms;
         ALTER TABLE runs DROP COLUMN parent_run_id;
         ALTER TABLE runs DROP COLUMN parent_node_id;
         ALTER TABLE invocations DROP COLUMN params;
         ALTER TABLE attempts DROP COLUMN holder;
         ALTER TABLE attempts DROP COLUMN deadline_ms;
         DROP TABLE meta;
         PRAGMA user_version = 4;",
    )
    .unwrap();
    drop(conn);

    let crashed = child(&path, "create-run", Some("store.upgrade.after-step"));
    assert!(!crashed.status.success(), "child did not abort");
    assert_eq!(user_version(&path), 4, "the file keeps its old version");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let (meta, runs): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM sqlite_master WHERE name = 'meta'),
                    (SELECT count(*) FROM runs)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((meta, runs), (0, 1), "the first step was rolled back");
    drop(conn);

    let store = SqliteStore::migrate(&path).unwrap().0;
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    assert!(effect(&store, "create-run").await);
}

/// Turns the file at `path` into the version 11 file `spore` 0.3.0 wrote: no
/// `workers` table.
fn as_version_11(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "DROP TABLE workers;
         UPDATE meta SET value = '0.3.0' WHERE key = 'writer';
         PRAGMA user_version = 11;",
    )
    .unwrap();
}

/// The names of the tables in the file at `path`.
fn tables(path: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Q17: `migrate` upgrades a version 11 file, adds the `workers` table and says what
/// it did; on a current file it changes nothing.
#[tokio::test]
async fn q17_a_version_11_file_is_migrated() {
    let path = fresh_path();
    let (store, attempt) = expired_lease(&path, "old").await;
    drop(store);
    as_version_11(&path);

    let (store, migration) =
        SqliteStore::migrate_with_clock(&path, ManualClock::new(1_200)).unwrap();
    assert_eq!(
        migration,
        runspore_store_sqlite::Migration {
            from: 11,
            from_writer: Some("0.3.0".into()),
            to: runspore_store_sqlite::SCHEMA_VERSION,
        }
    );
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    assert!(tables(&path).contains(&"workers".to_string()));
    assert!(store.get_run(&attempt.key).await.unwrap().is_some());
    assert_eq!(
        store.writer().unwrap().as_deref(),
        Some(runspore_store_sqlite::WRITER)
    );
    drop(store);

    let (_, again) = SqliteStore::migrate(&path).unwrap();
    assert_eq!(again.from, again.to, "nothing left to migrate");
}

/// Q18: opening an older 0.2 file without migrating is refused before anything is
/// read or written, naming its writer and `spore migrate`.
#[tokio::test]
async fn q18_an_older_file_is_not_upgraded_by_opening_it() {
    let path = fresh_path();
    let (store, _) = expired_lease(&path, "old").await;
    drop(store);
    as_version_11(&path);
    let before = tables(&path);

    let error = SqliteStore::open_with_clock(&path, ManualClock::new(1_200))
        .err()
        .expect("refused");
    assert_eq!(
        (error.kind, error.code.as_str()),
        (StoreFailureKind::Incompatible, "schema.needs-migrate")
    );
    for wanted in [
        "schema version 11",
        "spore 0.3.0",
        runspore_store_sqlite::WRITER,
        "spore migrate",
    ] {
        assert!(error.message.contains(wanted), "{wanted}: {}", error.message);
    }
    assert_eq!(user_version(&path), 11, "the file is left as it was");
    assert_eq!(tables(&path), before);
}

/// Q19: `migrate` refuses while a worker of an older `spore` holds a step under a live
/// lease, naming it and the step, and leaves the file; an actor's claim does not stop
/// it, and once the lease has passed the file is migrated.
#[tokio::test]
async fn q19_migrate_waits_for_the_older_workers() {
    let path = fresh_path();
    let clock = ManualClock::new(1_000);
    let store = SqliteStore::open_with_clock(&path, clock.clone()).unwrap();
    let mut keys = Vec::new();
    for (run, owner) in [("busy", "worker-old"), ("claimed", "actor-ab")] {
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
        let (command, invocation) = schedule(&key, "build", 0);
        store
            .commit_turn(runspore_store_conformance::commit(
                &turn,
                "commit",
                0,
                vec![command],
            ))
            .await
            .unwrap();
        store
            .claim_attempt(claim(&key, "claim", &invocation, owner, 100))
            .await
            .unwrap();
        keys.push(key);
    }
    drop(store);
    as_version_11(&path);

    let error = SqliteStore::migrate_with_clock(&path, ManualClock::new(1_050))
        .err()
        .expect("refused");
    assert_eq!(
        (error.kind, error.code.as_str()),
        (StoreFailureKind::Unavailable, "migrate.workers-live")
    );
    for wanted in ["worker worker-old", "step build of run busy", "older spore"] {
        assert!(error.message.contains(wanted), "{wanted}: {}", error.message);
    }
    assert!(!error.message.contains("actor-ab"), "{}", error.message);
    assert_eq!(user_version(&path), 11, "the file is left as it was");

    let (store, migration) =
        SqliteStore::migrate_with_clock(&path, ManualClock::new(1_100)).unwrap();
    assert_eq!(migration.from, 11);
    assert_eq!(user_version(&path), runspore_store_sqlite::SCHEMA_VERSION);
    for key in &keys {
        assert!(store.get_run(key).await.unwrap().is_some());
    }
}
