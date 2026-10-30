//! SQLite adapter for the Runspore store protocol (`spec/store.md`).
//!
//! One database file, shared by every `SqliteStore` opened on it, in this process or
//! in others on the same host. Each mutation is one `BEGIN IMMEDIATE` transaction.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};

use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::failpoint;
use runspore_types::model::{ActivityResult, AttemptStatus, RetryActivity, ScheduleActivity};
use runspore_types::reducer::Diagnostic;
use runspore_types::store::*;

/// Schema version written to `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 13;
/// The first schema version with the `workers` table.
const SCHEMA_VERSION_WORKERS: i64 = 12;
/// The schema version `spore` 0.1 wrote. 0.2 replaces 0.1 outright: such a file is
/// refused, never upgraded (spec/store.md §3).
const SCHEMA_VERSION_0_1: i64 = 1;
/// The version of the `spore` this adapter ships in (build.rs), the one `spore
/// --version` prints: the release's on the commit its tag names, else a pre-release of
/// the next minor version named by its commit, such as `0.4.0-dev+a65d8d1`. Recorded as
/// the writer of every schema it initializes or upgrades, and named in its messages.
pub const VERSION: &str = env!("SPORE_BUILD_VERSION");
/// [`VERSION`] as `spore --version` prints it: with the commit's date on a pre-release.
pub const VERSION_TEXT: &str = env!("SPORE_BUILD_VERSION_TEXT");
/// The date of the commit a pre-release is built from; `None` for a release.
pub const COMMIT_DATE: Option<&str> = option_env!("SPORE_BUILD_DATE");
const SCHEMA: &str = include_str!("schema.sql");
/// Upgrades a version 2 database to version 3: the run's base folder.
const UPGRADE_2: &str = "ALTER TABLE runs ADD COLUMN base_dir TEXT;";
/// Upgrades a version 3 database to version 4: the artifact table and the run's
/// artifact digest.
const UPGRADE_3: &str = "CREATE TABLE artifacts (digest TEXT PRIMARY KEY, body BLOB NOT NULL);
    ALTER TABLE runs ADD COLUMN artifact_digest TEXT;";
/// Upgrades a version 4 database to version 5: the `meta` table that records the
/// writer.
const UPGRADE_4: &str = "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);";
/// Upgrades a version 5 database to version 6: an attempt's holder handle and deadline.
const UPGRADE_5: &str = "ALTER TABLE attempts ADD COLUMN holder BLOB;
    ALTER TABLE attempts ADD COLUMN deadline_ms INTEGER;";
/// Upgrades a version 6 database to version 7: an invocation's node params.
const UPGRADE_6: &str = "ALTER TABLE invocations ADD COLUMN params BLOB;";
/// Upgrades a version 7 database to version 8: the run's parent.
const UPGRADE_7: &str = "ALTER TABLE runs ADD COLUMN parent_run_id TEXT;
    ALTER TABLE runs ADD COLUMN parent_node_id TEXT;";
/// Upgrades a version 8 database to version 9: when an attempt was claimed.
const UPGRADE_8: &str = "ALTER TABLE attempts ADD COLUMN claimed_at_ms INTEGER;";
/// Upgrades a version 9 database to version 10: the run's result channels.
const UPGRADE_9: &str = "ALTER TABLE runs ADD COLUMN result_channels TEXT;";
/// Upgrades a version 10 database to version 11: the run's cancel record.
const UPGRADE_10: &str = "ALTER TABLE runs ADD COLUMN cancel_reason TEXT;
    ALTER TABLE runs ADD COLUMN cancel_at_ms INTEGER;
    ALTER TABLE runs ADD COLUMN cancel_request_id TEXT;";
/// Upgrades a version 11 database to version 12: the workers that use it.
const UPGRADE_11: &str = "CREATE TABLE workers (worker_id TEXT PRIMARY KEY,
    version TEXT NOT NULL, host TEXT NOT NULL, pid INTEGER NOT NULL,
    beat_at_ms INTEGER NOT NULL, until_ms INTEGER NOT NULL);";
/// Upgrades a version 12 database to version 13: an attempt's needs-person mark.
const UPGRADE_12: &str = "ALTER TABLE attempts ADD COLUMN person_reason TEXT;
    ALTER TABLE attempts ADD COLUMN person_since_ms INTEGER;";
/// The prefix of the IDs the host gives its workers (host.md §3); other owners of an
/// attempt, such as an actor's claim, are not workers.
const WORKER_PREFIX: &str = "worker-";
/// The fix for a database another `spore` wrote.
const FIX: &str = "finish its runs with that spore, or move the file aside";
const MAX_EVENT_BYTES: usize = 64 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;

/// Wall-clock time in milliseconds since the Unix epoch.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// What `migrate` found and left: the schema version the file had (0 for a new file)
/// with the `spore` it recorded as its writer, and the version it has now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub from: i64,
    pub from_writer: Option<String>,
    pub to: i64,
}

/// A `Store` on one SQLite file. Calls run synchronously behind a mutex; the lock is
/// never held across an `.await`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
}

type R<T> = StoreResult<T>;

/// The recorded writer, or `None` when the file has no `meta` table or no entry: a
/// schema this adapter did not write, read without failing on its shape.
fn writer(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = 'writer'", [], |r| {
        r.get(0)
    })
    .ok()
}

fn fail(kind: StoreFailureKind, code: &str, message: impl Into<String>) -> StoreFailure {
    StoreFailure::new(kind, code, message)
}

fn user_version(conn: &Connection) -> R<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db)
}

/// The schema guard of spec/store.md §3: a file `spore` 0.1 wrote is
/// `schema.too-old`, one with a newer schema is `schema.too-new`; both name the
/// version that wrote it and the fix. An older 0.2 schema is `schema.needs-migrate`
/// unless the caller migrates.
fn guard(conn: &Connection, version: i64, migrate: bool) -> R<()> {
    if !migrate && version > SCHEMA_VERSION_0_1 && version < SCHEMA_VERSION {
        let writer = writer(conn)
            .map(|w| format!("spore {w}"))
            .unwrap_or_else(|| "an older spore".to_string());
        return Err(fail(
            StoreFailureKind::Incompatible,
            "schema.needs-migrate",
            format!(
                "this database has schema version {version}, written by {writer}, older \
                 than spore {VERSION} (schema version {SCHEMA_VERSION}): run `spore migrate` \
                 to upgrade it once no older worker uses it, or keep using {writer}"
            ),
        ));
    }
    if version > SCHEMA_VERSION {
        let writer = writer(conn)
            .map(|w| format!("spore {w}"))
            .unwrap_or_else(|| format!("a newer spore (schema version {version})"));
        return Err(fail(
            StoreFailureKind::Incompatible,
            "schema.too-new",
            format!(
                "this database was written by {writer}, newer than spore {VERSION} \
                 (schema version {SCHEMA_VERSION}): {FIX}"
            ),
        ));
    }
    if version == SCHEMA_VERSION_0_1 {
        return Err(fail(
            StoreFailureKind::Incompatible,
            "schema.too-old",
            format!(
                "this database belongs to spore 0.1, which spore {VERSION} does not read: \
                 finish its runs with the 0.1 binary, or move the file aside"
            ),
        ));
    }
    Ok(())
}

/// The workers of another `spore` that use the file at `now`, one line each
/// (spec/store.md §3): a registered worker of another version whose heartbeat is
/// fresh, and a worker that holds a step under a live lease without being registered,
/// so of a `spore` older than the `workers` table.
fn live_workers(conn: &Connection, version: i64, now: u64) -> R<Vec<String>> {
    let mut found = Vec::new();
    let mut registered = Vec::new();
    if version >= SCHEMA_VERSION_WORKERS {
        let mut rows = conn
            .prepare(
                "SELECT worker_id, version, host, pid, beat_at_ms FROM workers
                 WHERE until_ms > ?1 ORDER BY worker_id",
            )
            .map_err(db)?;
        let rows = rows
            .query_map([i(now)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(db)?;
        for row in rows {
            let (id, writer, host, pid, beat_at_ms) = row.map_err(db)?;
            if writer != VERSION {
                let ago = (i(now) - beat_at_ms).max(0);
                found.push(format!(
                    "worker {id} of spore {writer} (pid {pid} on {host}, last heartbeat {ago} ms ago)"
                ));
            }
            registered.push(id);
        }
    }
    let mut rows = conn
        .prepare(
            "SELECT a.owner, a.run_id, i.node_id FROM attempts a
             JOIN invocations i ON i.tenant = a.tenant AND i.run_id = a.run_id
                               AND i.invocation_id = a.invocation_id
             WHERE a.status = 'running' AND a.lease_until_ms > ?1
               AND substr(a.owner, 1, length(?2)) = ?2
             ORDER BY a.owner, a.run_id",
        )
        .map_err(db)?;
    let rows = rows
        .query_map(params![i(now), WORKER_PREFIX], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(db)?;
    for row in rows {
        let (owner, run, node) = row.map_err(db)?;
        if !registered.contains(&owner) {
            found.push(format!(
                "worker {owner} of an older spore, which records no version, running step \
                 {node} of run {run}"
            ));
        }
    }
    Ok(found)
}

fn conflict(code: &str) -> StoreFailure {
    fail(StoreFailureKind::Conflict, code, code)
}

fn stale(code: &str) -> StoreFailure {
    fail(StoreFailureKind::Stale, code, code)
}

fn db(error: rusqlite::Error) -> StoreFailure {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => fail(
            StoreFailureKind::Unavailable,
            "store.busy",
            error.to_string(),
        ),
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => fail(
            StoreFailureKind::Corrupt,
            "store.corrupt",
            error.to_string(),
        ),
        _ => fail(StoreFailureKind::Unavailable, "store.io", error.to_string()),
    }
}

fn encode<T: serde::Serialize>(value: &T) -> R<Vec<u8>> {
    canonical::encode(value)
        .map_err(|e| fail(StoreFailureKind::Corrupt, "store.encode", e.to_string()))
}

fn verify(evidence: &Evidence) -> R<()> {
    if evidence.digest == digest::body(&evidence.body) {
        Ok(())
    } else {
        Err(conflict("evidence.digest-mismatch"))
    }
}

fn i(value: u64) -> i64 {
    value as i64
}

fn process_blob(process: &Option<ProcessRecord>) -> R<Option<Vec<u8>>> {
    process.as_ref().map(encode).transpose()
}

/// Reads a stored process record; anything else in the column is `rusqlite`'s
/// conversion failure, which maps to `store.io`.
fn process_column(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<ProcessRecord>> {
    let Some(bytes) = r.get::<_, Option<Vec<u8>>>(index)? else {
        return Ok(None);
    };
    canonical::decode(&bytes).map(Some).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Blob, Box::new(e))
    })
}

fn insert_audit(
    c: &Connection,
    a: &AttemptRef,
    kind: &str,
    evidence: &Evidence,
    now: u64,
) -> R<i64> {
    c.execute(
        "INSERT INTO audit (tenant, run_id, attempt_id, evidence_digest, evidence, at_ms, kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            a.key.tenant,
            a.key.run,
            a.attempt_id,
            evidence.digest,
            evidence.body,
            i(now),
            kind
        ],
    )
    .map_err(db)?;
    Ok(c.last_insert_rowid())
}

/// The result of a mutation body: `Applied` writes a receipt, `Same` returns an
/// earlier value as `duplicate` and writes nothing.
enum Outcome<T> {
    Applied(T),
    Same(T),
}

struct RunRow {
    start_key: String,
    package_digest: String,
    started_digest: String,
    revision: u64,
    applied: u64,
    next: u64,
    last_ms: u64,
    fence: u64,
    quarantined: bool,
    status: String,
    cancelled: bool,
}

fn run_row(c: &Connection, key: &RunKey) -> R<Option<RunRow>> {
    c.query_row(
        "SELECT start_key, package_digest, started_digest, revision, applied_sequence,
                next_sequence, last_accepted_ms, fence, quarantine_code IS NOT NULL,
                status, cancel_at_ms IS NOT NULL
         FROM runs WHERE tenant = ?1 AND run_id = ?2",
        params![key.tenant, key.run],
        |r| {
            Ok(RunRow {
                start_key: r.get(0)?,
                package_digest: r.get(1)?,
                started_digest: r.get(2)?,
                revision: r.get::<_, i64>(3)? as u64,
                applied: r.get::<_, i64>(4)? as u64,
                next: r.get::<_, i64>(5)? as u64,
                last_ms: r.get::<_, i64>(6)? as u64,
                fence: r.get::<_, i64>(7)? as u64,
                quarantined: r.get(8)?,
                status: r.get(9)?,
                cancelled: r.get(10)?,
            })
        },
    )
    .optional()
    .map_err(db)
}

fn existing_run(c: &Connection, key: &RunKey) -> R<RunRow> {
    run_row(c, key)?
        .ok_or_else(|| fail(StoreFailureKind::NotFound, "run.not-found", "run.not-found"))
}

/// Appends an event at the run's next sequence with `acceptedAtMs = max(now, previous)`.
fn push_event(
    c: &Connection,
    key: &RunKey,
    id: &str,
    kind: &str,
    body: &[u8],
    now: u64,
) -> R<EventAccepted> {
    let run = existing_run(c, key)?;
    let accepted = now.max(run.last_ms);
    c.execute(
        "INSERT INTO events (tenant, run_id, sequence, event_id, kind, body, body_digest, accepted_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![key.tenant, key.run, i(run.next), id, kind, body, digest::body(body), i(accepted)],
    )
    .map_err(db)?;
    c.execute(
        "UPDATE runs SET next_sequence = ?3, last_accepted_ms = ?4, updated_at_ms = ?5
         WHERE tenant = ?1 AND run_id = ?2",
        params![key.tenant, key.run, i(run.next + 1), i(accepted), i(now)],
    )
    .map_err(db)?;
    Ok(EventAccepted {
        sequence: run.next,
        accepted_at_ms: accepted,
    })
}

struct AttemptRow {
    status: String,
    lease_until_ms: u64,
    attempt_number: u32,
    deadline_ms: Option<u64>,
    /// When its needs-person mark was set; `None` while it has none.
    person_since_ms: Option<u64>,
}

/// The attempt named by `a` if its invocation, owner and fence all match.
fn attempt_row(c: &Connection, a: &AttemptRef) -> R<Option<AttemptRow>> {
    c.query_row(
        "SELECT status, lease_until_ms, attempt_number, deadline_ms, person_since_ms FROM attempts
         WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3
           AND invocation_id = ?4 AND owner = ?5 AND fence = ?6",
        params![
            a.key.tenant,
            a.key.run,
            a.attempt_id,
            a.invocation_id,
            a.owner,
            i(a.fence)
        ],
        |r| {
            Ok(AttemptRow {
                status: r.get(0)?,
                lease_until_ms: r.get::<_, i64>(1)? as u64,
                attempt_number: r.get(2)?,
                deadline_ms: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                person_since_ms: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
            })
        },
    )
    .optional()
    .map_err(db)
}

/// Ends a running attempt, settles its invocation, and appends its result event.
fn settle(
    c: &Connection,
    a: &AttemptRef,
    status: &str,
    body: &[u8],
    now: u64,
) -> R<ResultAccepted> {
    c.execute(
        "UPDATE attempts SET status = ?4, result = ?5, person_reason = NULL, person_since_ms = NULL
         WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
        params![a.key.tenant, a.key.run, a.attempt_id, status, body],
    )
    .map_err(db)?;
    c.execute(
        "UPDATE invocations SET status = 'settled' WHERE tenant = ?1 AND run_id = ?2 AND invocation_id = ?3",
        params![a.key.tenant, a.key.run, a.invocation_id],
    )
    .map_err(db)?;
    let event = ids::event_for_attempt(&a.attempt_id);
    let accepted = push_event(c, &a.key, &event, "activity.result", body, now)?;
    Ok(ResultAccepted {
        result_sequence: accepted.sequence,
    })
}

/// Expires every running attempt of the run as `expire_attempt` does with reason
/// `cancelled`; returns each with its holder handle.
fn cancel_attempts(c: &Connection, key: &RunKey, now: u64) -> R<Vec<CancelledAttempt>> {
    let mut stmt = c
        .prepare(
            "SELECT invocation_id, attempt_id, owner, fence, attempt_number, holder FROM attempts
             WHERE tenant = ?1 AND run_id = ?2 AND status = 'running' ORDER BY attempt_id",
        )
        .map_err(db)?;
    let running: Vec<(CancelledAttempt, u32)> = stmt
        .query_map(params![key.tenant, key.run], |r| {
            let attempt = AttemptRef {
                key: key.clone(),
                invocation_id: r.get(0)?,
                attempt_id: r.get(1)?,
                owner: r.get(2)?,
                fence: r.get::<_, i64>(3)? as u64,
            };
            Ok((
                CancelledAttempt {
                    attempt,
                    holder: r.get(5)?,
                },
                r.get(4)?,
            ))
        })
        .and_then(Iterator::collect)
        .map_err(db)?;
    let audit = Evidence::from_canonical(encode(
        &json!({"reason": ExpireReason::Cancelled.as_str()}),
    )?);
    let mut expired = Vec::with_capacity(running.len());
    for (cancelled, number) in running {
        let a = &cancelled.attempt;
        let body = encode(&ActivityResult::expired(&a.invocation_id, number))?;
        insert_audit(c, a, "attempt.expired", &audit, now)?;
        settle(c, a, "expired", &body, now)?;
        expired.push(cancelled);
    }
    Ok(expired)
}

fn cursor_value(cursor: Option<String>) -> R<Option<Value>> {
    cursor
        .map(|c| serde_json::from_str(&c).map_err(|_| conflict("cursor.invalid")))
        .transpose()
}

fn cursor_str(value: &Value, index: usize) -> R<String> {
    value[index]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| conflict("cursor.invalid"))
}

fn cursor_u64(value: &Value, index: usize) -> R<i64> {
    value[index]
        .as_u64()
        .map(i)
        .ok_or_else(|| conflict("cursor.invalid"))
}

const RUN_VIEW: &str = "SELECT tenant, run_id, start_key, package_digest, status, revision,
    applied_sequence, next_sequence, snapshot, snapshot_digest, quarantine_code,
    quarantine_details, quarantine_at_ms, created_at_ms, updated_at_ms, base_dir,
    artifact_digest, parent_run_id, parent_node_id, result_channels, cancel_reason,
    cancel_at_ms, cancel_request_id FROM runs";

fn run_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunView> {
    let code: Option<String> = r.get(10)?;
    Ok(RunView {
        key: RunKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
        start_key: r.get(2)?,
        package_digest: r.get(3)?,
        artifact_digest: r.get(16)?,
        status: r.get(4)?,
        revision: r.get::<_, i64>(5)? as u64,
        applied_sequence: r.get::<_, i64>(6)? as u64,
        next_sequence: r.get::<_, i64>(7)? as u64,
        snapshot: r.get(8)?,
        snapshot_digest: r.get(9)?,
        quarantine: match code {
            Some(code) => Some(Quarantine {
                code,
                details: r.get(11)?,
                at_ms: r.get::<_, i64>(12)? as u64,
            }),
            None => None,
        },
        created_at_ms: r.get::<_, i64>(13)? as u64,
        updated_at_ms: r.get::<_, i64>(14)? as u64,
        base_dir: r.get(15)?,
        parent: match (r.get(17)?, r.get(18)?) {
            (Some(run_id), Some(node_id)) => Some(RunParent { run_id, node_id }),
            _ => None,
        },
        result_channels: match r.get::<_, Option<String>>(19)? {
            Some(text) => serde_json::from_str(&text).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(19, rusqlite::types::Type::Text, e.into())
            })?,
            None => BTreeMap::new(),
        },
        cancel: match r.get::<_, Option<i64>>(21)? {
            Some(at_ms) => Some(RunCancel {
                reason: r.get(20)?,
                at_ms: at_ms as u64,
                request_id: r.get(22)?,
            }),
            None => None,
        },
    })
}

fn page<T>(mut items: Vec<T>, limit: u32, cursor: impl Fn(&T) -> Value) -> Page<T> {
    let next = if items.len() > limit as usize {
        items.truncate(limit as usize);
        items.last().map(|last| cursor(last).to_string())
    } else {
        None
    };
    Page { items, next }
}

impl SqliteStore {
    /// Opens or initializes the database at `path` with the system clock.
    /// Never upgrades it: see `open_with_clock`.
    pub fn open(path: impl AsRef<Path>) -> R<Self> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// Opens or initializes the database at `path`; never upgrades it. A file written
    /// by `spore` 0.1 is `incompatible` / `schema.too-old`; one with a newer schema is
    /// `incompatible` / `schema.too-new`; one an earlier 0.2 `spore` wrote is
    /// `incompatible` / `schema.needs-migrate`. Each names the version that wrote it and
    /// the fix, and leaves the file as it was (spec/store.md §3).
    pub fn open_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> R<Self> {
        Self::open_inner(path.as_ref(), clock, false).map(|(store, _)| store)
    }

    /// Opens or initializes the database at `path` with the system clock, and upgrades
    /// a file an earlier 0.2 `spore` wrote.
    pub fn migrate(path: impl AsRef<Path>) -> R<(Self, Migration)> {
        Self::migrate_with_clock(path, Arc::new(SystemClock))
    }

    /// Opens or initializes the database at `path`, and upgrades a file an earlier 0.2
    /// `spore` wrote in one transaction. While a worker of another `spore` uses the
    /// file the upgrade is `unavailable` / `migrate.workers-live`, naming each, and the
    /// file is left as it was (spec/store.md §3). Refuses a 0.1 or a newer file as
    /// `open_with_clock` does.
    pub fn migrate_with_clock(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
    ) -> R<(Self, Migration)> {
        Self::open_inner(path.as_ref(), clock, true)
    }

    fn open_inner(path: &Path, clock: Arc<dyn Clock>, migrate: bool) -> R<(Self, Migration)> {
        let mut conn = Connection::open(path).map_err(db)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(db)?;
        // Before any pragma changes the file: a refused file is left as it was.
        guard(&conn, user_version(&conn)?, migrate)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(db)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        // Again under the write lock, in case another process changed it meanwhile.
        let version = user_version(&tx)?;
        guard(&tx, version, migrate)?;
        let from_writer = writer(&tx);
        if version == 0 {
            tx.execute_batch(SCHEMA).map_err(db)?;
        } else if version < SCHEMA_VERSION {
            let live = live_workers(&tx, version, clock.now_ms())?;
            if !live.is_empty() {
                return Err(fail(
                    StoreFailureKind::Unavailable,
                    "migrate.workers-live",
                    format!(
                        "spore {VERSION} does not upgrade this database while a worker of \
                         another spore uses it: {}; stop them, or let them finish, then \
                         migrate again",
                        live.join("; ")
                    ),
                ));
            }
            for (from, upgrade) in [
                (2, UPGRADE_2),
                (3, UPGRADE_3),
                (4, UPGRADE_4),
                (5, UPGRADE_5),
                (6, UPGRADE_6),
                (7, UPGRADE_7),
                (8, UPGRADE_8),
                (9, UPGRADE_9),
                (10, UPGRADE_10),
                (11, UPGRADE_11),
                (12, UPGRADE_12),
            ] {
                if version <= from {
                    tx.execute_batch(upgrade).map_err(db)?;
                    failpoint::hit("store.upgrade.after-step");
                }
            }
        }
        if version < SCHEMA_VERSION {
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(db)?;
            tx.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('writer', ?1)",
                [VERSION],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        let migration = Migration {
            from: version,
            from_writer,
            to: SCHEMA_VERSION,
        };
        let store = Self {
            conn: Mutex::new(conn),
            clock,
        };
        Ok((store, migration))
    }

    /// Records that worker `worker_id` of this `spore`, process `pid` on `host`, uses
    /// the file, with a heartbeat fresh for `fresh_ms` from now. `migrate` refuses
    /// while a worker of another version has a fresh one (spec/store.md §3).
    /// The record is advisory: a beat a crash loses costs nothing, so it is committed
    /// without waiting for the disk and never holds the write lock through an fsync
    /// while workers renew their leases.
    pub fn beat_worker(&self, worker_id: &str, host: &str, pid: u32, fresh_ms: u64) -> R<()> {
        let mut conn = self.lock();
        conn.pragma_update(None, "synchronous", "OFF").map_err(db)?;
        let beat = (|| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            let now = self.clock.now_ms();
            tx.execute(
                "INSERT OR REPLACE INTO workers (worker_id, version, host, pid, beat_at_ms, until_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    worker_id,
                    VERSION,
                    host,
                    pid,
                    i(now),
                    i(now.saturating_add(fresh_ms))
                ],
            )
            .map_err(db)?;
            tx.commit().map_err(db)
        })();
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(db)?;
        beat
    }

    /// Removes the record of a worker that stopped.
    pub fn end_worker(&self, worker_id: &str) -> R<()> {
        self.lock()
            .execute("DELETE FROM workers WHERE worker_id = ?1", [worker_id])
            .map(|_| ())
            .map_err(db)
    }

    /// The `spore` that wrote the schema, as the file records it.
    pub fn writer(&self) -> R<Option<String>> {
        self.read(|conn| Ok(writer(conn)))
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(db)?;
        f(&tx)
    }

    /// Runs one mutation: receipt first, then `body`, then the receipt insert, all in
    /// one `BEGIN IMMEDIATE` transaction with "now" sampled under the write lock.
    fn mutate<T>(
        &self,
        operation: &str,
        failpoint_op: Option<&str>,
        key: &RunKey,
        mutation: &Mutation,
        body: impl FnOnce(&Connection, u64) -> R<Outcome<T>>,
    ) -> R<Receipt<T>>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let mut conn = self.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let now = self.clock.now_ms();
        let receipt = |disposition, value| Receipt {
            request_id: mutation.request_id.clone(),
            request_digest: mutation.request_digest.clone(),
            disposition,
            value,
        };
        let stored: Option<(String, Vec<u8>)> = tx
            .query_row(
                "SELECT request_digest, value FROM receipts WHERE tenant = ?1 AND run_id = ?2 AND request_id = ?3",
                params![key.tenant, key.run, mutation.request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        if let Some((stored_digest, value)) = stored {
            if stored_digest != mutation.request_digest {
                return Err(conflict("request.digest-mismatch"));
            }
            let value = canonical::decode(&value)
                .map_err(|e| fail(StoreFailureKind::Corrupt, "receipt.corrupt", e.to_string()))?;
            return Ok(receipt(Disposition::Duplicate, value));
        }
        let value = match body(&tx, now)? {
            Outcome::Same(value) => return Ok(receipt(Disposition::Duplicate, value)),
            Outcome::Applied(value) => value,
        };
        tx.execute(
            "INSERT INTO receipts (tenant, run_id, request_id, request_digest, operation, value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key.tenant,
                key.run,
                mutation.request_id,
                mutation.request_digest,
                operation,
                encode(&value)?
            ],
        )
        .map_err(db)?;
        if let Some(op) = failpoint_op {
            failpoint::hit(&format!("store.{op}.before-commit"));
        }
        tx.commit().map_err(db)?;
        if let Some(op) = failpoint_op {
            failpoint::hit(&format!("store.{op}.after-commit"));
        }
        Ok(receipt(Disposition::Applied, value))
    }
}

fn apply_commit(c: &Connection, q: &CommitTurn, now: u64) -> R<CommitReceipt> {
    let run = existing_run(c, &q.key)?;
    if run.cancelled {
        return Err(stale("run.cancelled"));
    }
    if run.quarantined {
        return Err(stale("run.quarantined"));
    }
    if run.revision != q.expected_revision {
        return Err(stale("revision.mismatch"));
    }
    let pending: Option<String> = c
        .query_row(
            "SELECT event_id FROM events WHERE tenant = ?1 AND run_id = ?2 AND sequence = ?3",
            params![q.key.tenant, q.key.run, i(run.applied + 1)],
            |r| r.get(0),
        )
        .optional()
        .map_err(db)?;
    if pending.as_deref() != Some(q.event_id.as_str()) || q.event_sequence != run.applied + 1 {
        return Err(stale("event.mismatch"));
    }
    if q.snapshot_digest != digest::state(&q.snapshot) {
        return Err(conflict("snapshot.digest-mismatch"));
    }
    if q.snapshot.len() > MAX_SNAPSHOT_BYTES {
        return Err(fail(
            StoreFailureKind::Quota,
            "snapshot.too-large",
            "snapshot.too-large",
        ));
    }
    for (n, command) in q.commands.iter().enumerate() {
        let taken: bool = c
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM commands WHERE tenant = ?1 AND run_id = ?2 AND command_id = ?3)",
                params![q.key.tenant, q.key.run, command.command_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if taken
            || q.commands[..n]
                .iter()
                .any(|o| o.command_id == command.command_id)
        {
            return Err(conflict("command.id-conflict"));
        }
    }
    let revision = run.revision + 1;
    let diagnostics = serde_json::to_string(&q.diagnostics)
        .map_err(|e| fail(StoreFailureKind::Corrupt, "store.encode", e.to_string()))?;
    c.execute(
        "INSERT INTO transitions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            q.key.tenant,
            q.key.run,
            i(revision),
            i(q.event_sequence),
            q.decision_digest,
            q.snapshot_digest,
            diagnostics,
            i(now)
        ],
    )
    .map_err(db)?;
    c.execute(
        "UPDATE runs SET revision = ?3, applied_sequence = ?4, snapshot = ?5, snapshot_digest = ?6,
                status = ?7, updated_at_ms = ?8 WHERE tenant = ?1 AND run_id = ?2",
        params![
            q.key.tenant,
            q.key.run,
            i(revision),
            i(q.event_sequence),
            q.snapshot,
            q.snapshot_digest,
            q.status,
            i(now)
        ],
    )
    .map_err(db)?;
    c.execute(
        "UPDATE events SET consumed_revision = ?4 WHERE tenant = ?1 AND run_id = ?2 AND sequence = ?3",
        params![q.key.tenant, q.key.run, i(q.event_sequence), i(revision)],
    )
    .map_err(db)?;
    let invalid = |_| conflict("command.payload-invalid");
    for (ordinal, command) in q.commands.iter().enumerate() {
        c.execute(
            "INSERT INTO commands VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                q.key.tenant,
                q.key.run,
                command.command_id,
                i(revision),
                ordinal as i64,
                command.activation_id,
                command.kind,
                command.payload
            ],
        )
        .map_err(db)?;
        match command.kind.as_str() {
            "activity.schedule" => {
                let s: ScheduleActivity = canonical::decode(&command.payload).map_err(invalid)?;
                let input = encode(&s.input)?;
                let node_params = s.params.as_ref().map(encode).transpose()?;
                let inserted = c
                    .execute(
                        "INSERT OR IGNORE INTO invocations (tenant, run_id, invocation_id, node_id, action_id,
                                effect_key, input, input_digest, status, attempt_number, not_before_ms, params)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 1, ?9, ?10)",
                        params![q.key.tenant, q.key.run, s.invocation_id, s.node_id, s.action_id, s.effect_key, input, digest::body(&input), i(s.not_before_ms), node_params],
                    )
                    .map_err(db)?;
                if inserted == 0 {
                    return Err(conflict("invocation.exists"));
                }
            }
            "activity.retry" => {
                let r: RetryActivity = canonical::decode(&command.payload).map_err(invalid)?;
                let updated = c
                    .execute(
                        "UPDATE invocations SET status = 'pending', attempt_number = ?4, not_before_ms = ?5
                         WHERE tenant = ?1 AND run_id = ?2 AND invocation_id = ?3
                           AND status = 'settled' AND attempt_number + 1 = ?4",
                        params![q.key.tenant, q.key.run, r.invocation_id, r.attempt, i(r.not_before_ms)],
                    )
                    .map_err(db)?;
                if updated == 0 {
                    return Err(conflict("invocation.retry-invalid"));
                }
            }
            _ => {
                return Err(fail(
                    StoreFailureKind::Incompatible,
                    "command.unknown-kind",
                    command.kind.clone(),
                ));
            }
        }
    }
    Ok(CommitReceipt {
        key: q.key.clone(),
        revision,
        applied_sequence: q.event_sequence,
        decision_digest: q.decision_digest.clone(),
    })
}

fn apply_claim(c: &Connection, q: &ClaimAttempt, now: u64) -> R<Claim> {
    let row = c
        .query_row(
            "SELECT node_id, action_id, effect_key, input, input_digest, status, attempt_number, not_before_ms,
                    params
             FROM invocations WHERE tenant = ?1 AND run_id = ?2 AND invocation_id = ?3",
            params![q.key.tenant, q.key.run, q.invocation_id],
            |r| {
                Ok((
                    (
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ),
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, u32>(6)?,
                    r.get::<_, i64>(7)? as u64,
                    r.get::<_, Option<Vec<u8>>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(db)?;
    let Some((
        (node_id, action_id, effect_key),
        input,
        input_digest,
        status,
        number,
        not_before,
        node_params,
    )) = row
    else {
        return Err(fail(
            StoreFailureKind::NotFound,
            "invocation.not-found",
            "invocation.not-found",
        ));
    };
    let run = existing_run(c, &q.key)?;
    if run.cancelled {
        return Err(stale("run.cancelled"));
    }
    if run.quarantined {
        return Err(stale("run.quarantined"));
    }
    if status != "pending" {
        return Err(conflict("invocation.not-claimable"));
    }
    if now < not_before {
        return Err(conflict("invocation.not-due"));
    }
    if q.holder
        .as_ref()
        .is_some_and(|h| h.len() > MAX_HOLDER_BYTES)
    {
        return Err(fail(
            StoreFailureKind::Quota,
            "holder.too-large",
            "holder.too-large",
        ));
    }
    let fence = run.fence + 1;
    let attempt_id = ids::attempt(&q.invocation_id, number);
    let deadline_ms = q.deadline_duration_ms.map(|d| now.saturating_add(d));
    let lease_until_ms = capped(now.saturating_add(q.lease_duration_ms), deadline_ms);
    c.execute(
        "UPDATE runs SET fence = ?3, updated_at_ms = ?4 WHERE tenant = ?1 AND run_id = ?2",
        params![q.key.tenant, q.key.run, i(fence), i(now)],
    )
    .map_err(db)?;
    c.execute(
        "INSERT INTO attempts (tenant, run_id, attempt_id, invocation_id, attempt_number, owner,
                               fence, lease_until_ms, status, holder, deadline_ms, claimed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'running', ?9, ?10, ?11)",
        params![
            q.key.tenant,
            q.key.run,
            attempt_id,
            q.invocation_id,
            number,
            q.worker,
            i(fence),
            i(lease_until_ms),
            q.holder,
            deadline_ms.map(i),
            i(now)
        ],
    )
    .map_err(db)?;
    c.execute(
        "UPDATE invocations SET status = 'running' WHERE tenant = ?1 AND run_id = ?2 AND invocation_id = ?3",
        params![q.key.tenant, q.key.run, q.invocation_id],
    )
    .map_err(db)?;
    Ok(Claim {
        attempt: AttemptRef {
            key: q.key.clone(),
            invocation_id: q.invocation_id.clone(),
            attempt_id,
            owner: q.worker.clone(),
            fence,
        },
        attempt_number: number,
        lease_until_ms,
        node_id,
        action_id,
        effect_key,
        input_digest,
        input,
        params: node_params,
        holder: q.holder.clone(),
        deadline_ms,
    })
}

/// A lease end that never passes the attempt's deadline.
fn capped(lease_until_ms: u64, deadline_ms: Option<u64>) -> u64 {
    deadline_ms.map_or(lease_until_ms, |d| lease_until_ms.min(d))
}

fn set_quarantine(
    c: &Connection,
    key: &RunKey,
    value: Option<(&str, &str)>,
    now: u64,
) -> R<QuarantineChanged> {
    let run = existing_run(c, key)?;
    if run.quarantined != value.is_some() {
        c.execute(
            "UPDATE runs SET quarantine_code = ?3, quarantine_details = ?4, quarantine_at_ms = ?5, updated_at_ms = ?6
             WHERE tenant = ?1 AND run_id = ?2",
            params![key.tenant, key.run, value.map(|v| v.0), value.map(|v| v.1), value.map(|_| i(now)), i(now)],
        )
        .map_err(db)?;
    }
    Ok(QuarantineChanged {
        quarantined: value.is_some(),
    })
}

#[async_trait]
impl Store for SqliteStore {
    async fn capabilities(&self) -> R<Capabilities> {
        Ok(Capabilities {
            protocol: STORE_PROTOCOL.to_string(),
            atomic_run_mutation: true,
            conditional_writes: true,
            unique_insert: true,
            authoritative_readback: true,
            ordered_recovery_scan: true,
            multiworker_claims: true,
            persistent_wakeup: false,
            failure_model: "crash-stop".to_string(),
            transaction_scope: "database".to_string(),
            max_record_bytes: MAX_SNAPSHOT_BYTES as u64,
        })
    }

    async fn create_run(&self, q: CreateRun) -> R<Receipt<RunCreated>> {
        self.mutate(
            "create_run",
            Some("create-run"),
            &q.key,
            &q.mutation,
            |c, now| {
                if q.package_digest != digest::package(&q.package) {
                    return Err(conflict("package.digest-mismatch"));
                }
                if q.artifact
                    .as_ref()
                    .is_some_and(|a| a.digest != digest::artifact(&a.bytes))
                {
                    return Err(conflict("artifact.digest-mismatch"));
                }
                verify(&q.started)?;
                let created = RunCreated {
                    key: q.key.clone(),
                    started_sequence: 1,
                };
                let by_start: Option<String> = c
                    .query_row(
                        "SELECT run_id FROM runs WHERE tenant = ?1 AND start_key = ?2",
                        params![q.key.tenant, q.start_key],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(db)?;
                if by_start.is_some_and(|run| run != q.key.run) {
                    return Err(conflict("run.start-key-mismatch"));
                }
                if let Some(run) = run_row(c, &q.key)? {
                    return if run.start_key == q.start_key
                        && run.package_digest == q.package_digest
                        && run.started_digest == q.started.digest
                    {
                        Ok(Outcome::Same(created))
                    } else {
                        Err(conflict("run.exists"))
                    };
                }
                c.execute(
                    "INSERT OR IGNORE INTO packages (digest, body) VALUES (?1, ?2)",
                    params![q.package_digest, q.package],
                )
                .map_err(db)?;
                if let Some(artifact) = &q.artifact {
                    c.execute(
                        "INSERT OR IGNORE INTO artifacts (digest, body) VALUES (?1, ?2)",
                        params![artifact.digest, artifact.bytes],
                    )
                    .map_err(db)?;
                }
                let channels =
                    (!q.result_channels.is_empty()).then(|| json!(q.result_channels).to_string());
                c.execute(
                    "INSERT INTO runs (tenant, run_id, start_key, package_digest, started_digest,
                     status, revision, applied_sequence, next_sequence, last_accepted_ms,
                     snapshot, snapshot_digest, fence, quarantine_code, quarantine_details,
                     quarantine_at_ms, created_at_ms, updated_at_ms, base_dir, artifact_digest,
                     parent_run_id, parent_node_id, result_channels)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'created', 0, 0, 1, 0, NULL, NULL, 0, NULL, NULL,
                     NULL, ?6, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        q.key.tenant,
                        q.key.run,
                        q.start_key,
                        q.package_digest,
                        q.started.digest,
                        i(now),
                        q.base_dir,
                        q.artifact.as_ref().map(|a| &a.digest),
                        q.parent.as_ref().map(|p| &p.run_id),
                        q.parent.as_ref().map(|p| &p.node_id),
                        channels
                    ],
                )
                .map_err(db)?;
                push_event(
                    c,
                    &q.key,
                    ids::EVENT_STARTED,
                    "run.started",
                    &q.started.body,
                    now,
                )?;
                Ok(Outcome::Applied(created))
            },
        )
    }

    async fn append_event(&self, q: AppendEvent) -> R<Receipt<EventAccepted>> {
        self.mutate(
            "append_event",
            Some("append-event"),
            &q.key,
            &q.mutation,
            |c, now| {
                verify(&q.body)?;
                existing_run(c, &q.key)?;
                if q.kind == "run.started" || q.kind == "activity.result" {
                    return Err(conflict("event.reserved-kind"));
                }
                if q.body.body.len() > MAX_EVENT_BYTES {
                    return Err(fail(
                        StoreFailureKind::Quota,
                        "event.too-large",
                        "event.too-large",
                    ));
                }
                let prior = c
                    .query_row(
                        "SELECT kind, body_digest, sequence, accepted_at_ms FROM events
                     WHERE tenant = ?1 AND run_id = ?2 AND event_id = ?3",
                        params![q.key.tenant, q.key.run, q.event_id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, i64>(2)?,
                                r.get::<_, i64>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(db)?;
                match prior {
                    Some((kind, body_digest, sequence, at))
                        if kind == q.kind && body_digest == q.body.digest =>
                    {
                        Ok(Outcome::Same(EventAccepted {
                            sequence: sequence as u64,
                            accepted_at_ms: at as u64,
                        }))
                    }
                    Some(_) => Err(conflict("event.id-conflict")),
                    None => push_event(c, &q.key, &q.event_id, &q.kind, &q.body.body, now)
                        .map(Outcome::Applied),
                }
            },
        )
    }

    async fn load_turn(&self, key: &RunKey) -> R<Option<Turn>> {
        self.read(|c| {
            let Some(view) = c
                .query_row(
                    &format!("{RUN_VIEW} WHERE tenant = ?1 AND run_id = ?2"),
                    params![key.tenant, key.run],
                    run_view,
                )
                .optional()
                .map_err(db)?
            else {
                return Ok(None);
            };
            if view.quarantine.is_some()
                || view.cancel.is_some()
                || view.applied_sequence + 1 == view.next_sequence
            {
                return Ok(None);
            }
            let event = list_events(c, key, view.applied_sequence, 1)?
                .items
                .remove(0)
                .event;
            Ok(Some(Turn {
                key: view.key,
                revision: view.revision,
                applied_sequence: view.applied_sequence,
                package_digest: view.package_digest,
                snapshot: view.snapshot,
                snapshot_digest: view.snapshot_digest,
                next_event: event,
            }))
        })
    }

    async fn get_package(&self, digest: &str) -> R<Option<Vec<u8>>> {
        self.read(|c| {
            c.query_row(
                "SELECT body FROM packages WHERE digest = ?1",
                params![digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)
        })
    }

    async fn get_artifact(&self, digest: &str) -> R<Option<Vec<u8>>> {
        self.read(|c| {
            c.query_row(
                "SELECT body FROM artifacts WHERE digest = ?1",
                params![digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)
        })
    }

    async fn commit_turn(&self, q: CommitTurn) -> R<Receipt<CommitReceipt>> {
        self.mutate(
            "commit_turn",
            Some("commit-turn"),
            &q.key,
            &q.mutation,
            |c, now| apply_commit(c, &q, now).map(Outcome::Applied),
        )
    }

    async fn get_receipt(&self, key: &RunKey, request_id: &str) -> R<Option<StoredReceipt>> {
        self.read(|c| {
            c.query_row(
                "SELECT request_digest, operation, value FROM receipts WHERE tenant = ?1 AND run_id = ?2 AND request_id = ?3",
                params![key.tenant, key.run, request_id],
                |r| {
                    Ok(StoredReceipt {
                        request_id: request_id.to_string(),
                        request_digest: r.get(0)?,
                        operation: r.get(1)?,
                        value: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(db)
        })
    }

    async fn claim_attempt(&self, q: ClaimAttempt) -> R<Receipt<Claim>> {
        self.mutate(
            "claim_attempt",
            Some("claim-attempt"),
            &q.key,
            &q.mutation,
            |c, now| apply_claim(c, &q, now).map(Outcome::Applied),
        )
    }

    async fn heartbeat(&self, q: Heartbeat) -> R<Receipt<LeaseExtended>> {
        self.mutate("heartbeat", Some("heartbeat"), &q.attempt.key, &q.mutation, |c, now| {
            let deadline_ms = match attempt_row(c, &q.attempt)? {
                Some(a) if a.status == "running" && now < a.lease_until_ms => a.deadline_ms,
                _ => return Err(stale("lease.lost")),
            };
            let lease_until_ms = capped(now.saturating_add(q.extend_ms), deadline_ms);
            c.execute(
                "UPDATE attempts SET lease_until_ms = ?4 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
                params![q.attempt.key.tenant, q.attempt.key.run, q.attempt.attempt_id, i(lease_until_ms)],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(LeaseExtended { lease_until_ms }))
        })
    }

    async fn hold_attempt(&self, q: HoldAttempt) -> R<Receipt<AttemptHeld>> {
        self.mutate("hold_attempt", Some("hold-attempt"), &q.attempt.key, &q.mutation, |c, now| {
            match attempt_row(c, &q.attempt)? {
                Some(a) if a.status == "running" && now < a.lease_until_ms => {}
                _ => return Err(stale("lease.lost")),
            }
            if q.holder.len() > MAX_HOLDER_BYTES {
                return Err(fail(StoreFailureKind::Quota, "holder.too-large", "holder.too-large"));
            }
            let deadline_ms = now.saturating_add(q.deadline_duration_ms);
            let lease_until_ms = capped(now.saturating_add(q.lease_duration_ms), Some(deadline_ms));
            c.execute(
                "UPDATE attempts SET holder = ?4, deadline_ms = ?5, lease_until_ms = ?6, process = NULL
                 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
                params![
                    q.attempt.key.tenant,
                    q.attempt.key.run,
                    q.attempt.attempt_id,
                    q.holder,
                    i(deadline_ms),
                    i(lease_until_ms)
                ],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(AttemptHeld {
                holder: q.holder.clone(),
                lease_until_ms,
                deadline_ms,
            }))
        })
    }

    async fn set_process(&self, q: SetProcess) -> R<Receipt<ProcessSet>> {
        self.mutate("set_process", Some("set-process"), &q.attempt.key, &q.mutation, |c, now| {
            match attempt_row(c, &q.attempt)? {
                Some(a) if a.status == "running" && now < a.lease_until_ms => {}
                _ => return Err(stale("lease.lost")),
            }
            c.execute(
                "UPDATE attempts SET process = ?4 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
                params![q.attempt.key.tenant, q.attempt.key.run, q.attempt.attempt_id, process_blob(&q.process)?],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(ProcessSet { process: q.process.clone() }))
        })
    }

    async fn set_needs_person(&self, q: SetNeedsPerson) -> R<Receipt<NeedsPersonSet>> {
        self.mutate(
            "set_needs_person",
            Some("set-needs-person"),
            &q.attempt.key,
            &q.mutation,
            |c, now| {
                let since_ms = match attempt_row(c, &q.attempt)? {
                    Some(a) if a.status == "running" && now < a.lease_until_ms => a.person_since_ms,
                    _ => return Err(stale("lease.lost")),
                };
                if q.reason
                    .as_ref()
                    .is_some_and(|r| r.len() > MAX_PERSON_REASON_BYTES)
                {
                    return Err(fail(
                        StoreFailureKind::Quota,
                        "person.reason-too-large",
                        "person.reason-too-large",
                    ));
                }
                let mark = q.reason.as_ref().map(|reason| PersonMark {
                    reason: reason.clone(),
                    since_ms: since_ms.unwrap_or(now),
                });
                c.execute(
                    "UPDATE attempts SET person_reason = ?4, person_since_ms = ?5
                 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
                    params![
                        q.attempt.key.tenant,
                        q.attempt.key.run,
                        q.attempt.attempt_id,
                        mark.as_ref().map(|m| &m.reason),
                        mark.as_ref().map(|m| i(m.since_ms))
                    ],
                )
                .map_err(db)?;
                Ok(Outcome::Applied(NeedsPersonSet { mark }))
            },
        )
    }

    async fn finish_attempt(&self, q: FinishAttempt) -> R<Receipt<ResultAccepted>> {
        self.mutate(
            "finish_attempt",
            Some("finish-attempt"),
            &q.attempt.key,
            &q.mutation,
            |c, now| {
                verify(&q.result)?;
                let a = &q.attempt;
                let result: ActivityResult =
                    canonical::decode(&q.result.body).map_err(|_| conflict("result.invalid"))?;
                let number = ids::attempt(&a.invocation_id, result.attempt) == a.attempt_id;
                if result.invocation_id != a.invocation_id
                    || result.attempt_id != a.attempt_id
                    || !number
                    || result.status == AttemptStatus::Expired
                {
                    return Err(conflict("result.invalid"));
                }
                match attempt_row(c, a)? {
                    Some(row)
                        if row.status == "running"
                            && now < row.lease_until_ms
                            && row.attempt_number == result.attempt => {}
                    _ => return Err(stale("lease.lost")),
                }
                settle(c, a, "finished", &q.result.body, now).map(Outcome::Applied)
            },
        )
    }

    async fn expire_attempt(&self, q: ExpireAttempt) -> R<Receipt<ResultAccepted>> {
        self.mutate(
            "expire_attempt",
            Some("expire-attempt"),
            &q.attempt.key,
            &q.mutation,
            |c, now| {
                let Some(row) = attempt_row(c, &q.attempt)? else {
                    return Err(fail(
                        StoreFailureKind::NotFound,
                        "attempt.not-found",
                        "attempt.not-found",
                    ));
                };
                if row.status != "running" {
                    return Err(stale("lease.lost"));
                }
                if q.reason == ExpireReason::Lease && now < row.lease_until_ms {
                    return Err(conflict("lease.not-due"));
                }
                let body = encode(&ActivityResult::expired(
                    &q.attempt.invocation_id,
                    row.attempt_number,
                ))?;
                let mut audit = json!({"reason": q.reason.as_str()});
                if q.process_unchecked {
                    audit["notes"] = json!(["process.unchecked"]);
                }
                let audit = Evidence::from_canonical(encode(&audit)?);
                insert_audit(c, &q.attempt, "attempt.expired", &audit, now)?;
                settle(c, &q.attempt, "expired", &body, now).map(Outcome::Applied)
            },
        )
    }

    async fn record_late_evidence(&self, q: RecordLateEvidence) -> R<Receipt<AuditRecorded>> {
        self.mutate("record_late_evidence", None, &q.attempt.key, &q.mutation, |c, now| {
            verify(&q.evidence)?;
            let a = &q.attempt;
            let exists: bool = c
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM attempts WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3)",
                    params![a.key.tenant, a.key.run, a.attempt_id],
                    |r| r.get(0),
                )
                .map_err(db)?;
            if !exists {
                return Err(fail(StoreFailureKind::NotFound, "attempt.not-found", "attempt.not-found"));
            }
            let id = insert_audit(c, a, "late-evidence", &q.evidence, now)?;
            Ok(Outcome::Applied(AuditRecorded { audit_id: format!("audit_{id}") }))
        })
    }

    async fn quarantine_run(&self, q: QuarantineRun) -> R<Receipt<QuarantineChanged>> {
        self.mutate("quarantine_run", None, &q.key, &q.mutation, |c, now| {
            let run = existing_run(c, &q.key)?;
            if run.cancelled {
                return Err(stale("run.cancelled"));
            }
            if run.revision != q.expected_revision {
                return Err(stale("revision.mismatch"));
            }
            set_quarantine(c, &q.key, Some((&q.code, &q.details)), now).map(Outcome::Applied)
        })
    }

    async fn release_run(&self, q: ReleaseRun) -> R<Receipt<QuarantineChanged>> {
        self.mutate("release_run", None, &q.key, &q.mutation, |c, now| {
            set_quarantine(c, &q.key, None, now).map(Outcome::Applied)
        })
    }

    async fn cancel_run(&self, q: CancelRun) -> R<Receipt<RunCancelled>> {
        self.mutate("cancel_run", Some("cancel-run"), &q.key, &q.mutation, |c, now| {
            let run = existing_run(c, &q.key)?;
            if matches!(run.status.as_str(), "completed" | "failed" | STATUS_CANCELLED) {
                let cancel = c
                    .query_row(
                        &format!("{RUN_VIEW} WHERE tenant = ?1 AND run_id = ?2"),
                        params![q.key.tenant, q.key.run],
                        run_view,
                    )
                    .map_err(db)?
                    .cancel;
                return Ok(Outcome::Same(RunCancelled { status: run.status, cancel, expired: Vec::new() }));
            }
            let expired = cancel_attempts(c, &q.key, now)?;
            c.execute(
                "UPDATE runs SET status = ?3, cancel_reason = ?4, cancel_at_ms = ?5, cancel_request_id = ?6,
                        quarantine_code = NULL, quarantine_details = NULL, quarantine_at_ms = NULL,
                        updated_at_ms = ?5
                 WHERE tenant = ?1 AND run_id = ?2",
                params![q.key.tenant, q.key.run, STATUS_CANCELLED, q.reason, i(now), q.mutation.request_id],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(RunCancelled {
                status: STATUS_CANCELLED.to_string(),
                cancel: Some(RunCancel {
                    reason: q.reason.clone(),
                    at_ms: now,
                    request_id: q.mutation.request_id.clone(),
                }),
                expired,
            }))
        })
    }

    async fn scan_ready(&self, cursor: Option<String>, limit: u32) -> R<Page<RunKey>> {
        let after = cursor_value(cursor)?;
        self.read(|c| {
            let (tenant, run) = match &after {
                Some(v) => (cursor_str(v, 0)?, cursor_str(v, 1)?),
                None => (String::new(), String::new()),
            };
            let mut stmt = c
                .prepare(
                    "SELECT tenant, run_id FROM runs
                     WHERE applied_sequence + 1 < next_sequence AND quarantine_code IS NULL
                       AND cancel_at_ms IS NULL
                       AND (tenant, run_id) > (?1, ?2) ORDER BY tenant, run_id LIMIT ?3",
                )
                .map_err(db)?;
            let items = stmt
                .query_map(params![tenant, run, i64::from(limit) + 1], |r| {
                    Ok(RunKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .and_then(Iterator::collect)
                .map_err(db)?;
            Ok(page(items, limit, |k: &RunKey| json!([k.tenant, k.run])))
        })
    }

    async fn scan_due(&self, cursor: Option<String>, limit: u32) -> R<Page<DueItem>> {
        let after = cursor_value(cursor)?;
        let now = self.clock.now_ms();
        self.read(|c| {
            let in_outbox = after.as_ref().is_some_and(|v| v[0] == "outbox");
            let (at, id, tenant, run) = match &after {
                Some(v) => (cursor_u64(v, 1)?, cursor_str(v, 2)?, cursor_str(v, 3)?, cursor_str(v, 4)?),
                None => (-1, String::new(), String::new(), String::new()),
            };
            let take = i64::from(limit) + 1;
            let mut items: Vec<(DueItem, Value)> = Vec::new();
            if !in_outbox {
                let mut stmt = c
                    .prepare(
                        "SELECT tenant, run_id, invocation_id, attempt_id, owner, fence, lease_until_ms, process FROM attempts
                         WHERE status = 'running' AND lease_until_ms <= ?1
                           AND NOT EXISTS (SELECT 1 FROM runs r WHERE r.tenant = attempts.tenant
                                AND r.run_id = attempts.run_id AND r.cancel_at_ms IS NOT NULL)
                           AND (lease_until_ms, attempt_id, tenant, run_id) > (?2, ?3, ?4, ?5)
                         ORDER BY lease_until_ms, attempt_id, tenant, run_id LIMIT ?6",
                    )
                    .map_err(db)?;
                let rows = stmt
                    .query_map(params![i(now), at, id, tenant, run, take], |r| {
                        let attempt = AttemptRef {
                            key: RunKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
                            invocation_id: r.get(2)?,
                            attempt_id: r.get(3)?,
                            owner: r.get(4)?,
                            fence: r.get::<_, i64>(5)? as u64,
                        };
                        let cursor = json!(["lease", r.get::<_, i64>(6)?, attempt.attempt_id, attempt.key.tenant, attempt.key.run]);
                        let process = process_column(r, 7)?;
                        Ok((DueItem::Lease { attempt, process }, cursor))
                    })
                    .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
                    .map_err(db)?;
                items.extend(rows);
            }
            let (at, id, tenant, run) = if in_outbox { (at, id, tenant, run) } else { (-1, String::new(), String::new(), String::new()) };
            if items.len() <= limit as usize {
                let mut stmt = c
                    .prepare(
                        "SELECT i.tenant, i.run_id, i.invocation_id, i.not_before_ms FROM invocations i
                         JOIN runs r ON r.tenant = i.tenant AND r.run_id = i.run_id
                         WHERE i.status = 'pending' AND i.not_before_ms <= ?1 AND r.quarantine_code IS NULL
                           AND r.cancel_at_ms IS NULL
                           AND (i.not_before_ms, i.invocation_id, i.tenant, i.run_id) > (?2, ?3, ?4, ?5)
                         ORDER BY i.not_before_ms, i.invocation_id, i.tenant, i.run_id LIMIT ?6",
                    )
                    .map_err(db)?;
                let rows = stmt
                    .query_map(params![i(now), at, id, tenant, run, take], |r| {
                        let key = RunKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?);
                        let invocation_id: String = r.get(2)?;
                        let cursor = json!(["outbox", r.get::<_, i64>(3)?, invocation_id, key.tenant, key.run]);
                        Ok((DueItem::Outbox { key, invocation_id }, cursor))
                    })
                    .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
                    .map_err(db)?;
                items.extend(rows);
            }
            let page = page(items, limit, |(_, cursor)| cursor.clone());
            Ok(Page { items: page.items.into_iter().map(|(item, _)| item).collect(), next: page.next })
        })
    }

    async fn get_run(&self, key: &RunKey) -> R<Option<RunView>> {
        self.read(|c| {
            c.query_row(
                &format!("{RUN_VIEW} WHERE tenant = ?1 AND run_id = ?2"),
                params![key.tenant, key.run],
                run_view,
            )
            .optional()
            .map_err(db)
        })
    }

    async fn find_run(&self, tenant: &str, start_key: &str) -> R<Option<RunKey>> {
        self.read(|c| {
            c.query_row(
                "SELECT run_id FROM runs WHERE tenant = ?1 AND start_key = ?2",
                params![tenant, start_key],
                |r| r.get(0),
            )
            .optional()
            .map(|run: Option<String>| run.map(|run| RunKey::new(tenant, run)))
            .map_err(db)
        })
    }

    async fn list_runs(
        &self,
        tenant: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> R<Page<RunView>> {
        let after = cursor_value(cursor)?;
        self.read(|c| {
            let run = match &after {
                Some(v) => cursor_str(v, 0)?,
                None => String::new(),
            };
            let mut stmt = c
                .prepare(&format!(
                    "{RUN_VIEW} WHERE tenant = ?1 AND run_id > ?2 ORDER BY run_id LIMIT ?3"
                ))
                .map_err(db)?;
            let items = stmt
                .query_map(params![tenant, run, i64::from(limit) + 1], run_view)
                .and_then(Iterator::collect)
                .map_err(db)?;
            Ok(page(items, limit, |v: &RunView| json!([v.key.run])))
        })
    }

    async fn list_events(
        &self,
        key: &RunKey,
        after_sequence: u64,
        limit: u32,
    ) -> R<Page<EventRecord>> {
        self.read(|c| list_events(c, key, after_sequence, limit))
    }

    async fn list_invocations(&self, key: &RunKey) -> R<Vec<InvocationView>> {
        self.read(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT i.invocation_id, i.node_id, i.action_id, i.status, i.attempt_number, i.not_before_ms,
                            a.lease_until_ms, a.owner, a.process, a.attempt_id, a.fence, a.holder, a.deadline_ms,
                            a.claimed_at_ms, i.input, i.params, a.person_reason, a.person_since_ms
                     FROM invocations i LEFT JOIN attempts a ON a.tenant = i.tenant AND a.run_id = i.run_id
                       AND a.invocation_id = i.invocation_id AND a.attempt_number = i.attempt_number
                     WHERE i.tenant = ?1 AND i.run_id = ?2 ORDER BY i.invocation_id",
                )
                .map_err(db)?;
            let rows = stmt
                .query_map(params![key.tenant, key.run], |r| {
                    let status = match r.get::<_, String>(3)?.as_str() {
                        "pending" => InvocationStatus::Pending,
                        "running" => InvocationStatus::Running,
                        _ => InvocationStatus::Settled,
                    };
                    let invocation_id: String = r.get(0)?;
                    let owner: Option<String> = r.get(7)?;
                    let attempt = match (r.get::<_, Option<String>>(9)?, &owner) {
                        (Some(attempt_id), Some(owner)) => Some(AttemptRef {
                            key: key.clone(),
                            invocation_id: invocation_id.clone(),
                            attempt_id,
                            owner: owner.clone(),
                            fence: r.get::<_, i64>(10)? as u64,
                        }),
                        _ => None,
                    };
                    Ok(InvocationView {
                        invocation_id,
                        node_id: r.get(1)?,
                        action_id: r.get(2)?,
                        status,
                        attempt_number: r.get(4)?,
                        not_before_ms: r.get::<_, i64>(5)? as u64,
                        lease_until_ms: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                        owner,
                        process: process_column(r, 8)?,
                        attempt,
                        holder: r.get(11)?,
                        deadline_ms: r.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                        claimed_at_ms: r.get::<_, Option<i64>>(13)?.map(|v| v as u64),
                        input: r.get(14)?,
                        params: r.get(15)?,
                        needs_person: match (r.get::<_, Option<String>>(16)?, r.get::<_, Option<i64>>(17)?) {
                            (Some(reason), Some(since_ms)) => Some(PersonMark {
                                reason,
                                since_ms: since_ms as u64,
                            }),
                            _ => None,
                        },
                    })
                })
                .and_then(Iterator::collect)
                .map_err(db)?;
            Ok(rows)
        })
    }
}

fn list_events(c: &Connection, key: &RunKey, after: u64, limit: u32) -> R<Page<EventRecord>> {
    let mut stmt = c
        .prepare(
            "SELECT e.event_id, e.sequence, e.accepted_at_ms, e.kind, e.body, e.body_digest,
                    e.consumed_revision, t.diagnostics
             FROM events e LEFT JOIN transitions t ON t.tenant = e.tenant AND t.run_id = e.run_id
               AND t.revision = e.consumed_revision
             WHERE e.tenant = ?1 AND e.run_id = ?2 AND e.sequence > ?3 ORDER BY e.sequence LIMIT ?4",
        )
        .map_err(db)?;
    let rows: Vec<(EventRecord, Option<String>)> = stmt
        .query_map(
            params![key.tenant, key.run, i(after), i64::from(limit) + 1],
            |r| {
                let event = StoredEvent {
                    id: r.get(0)?,
                    sequence: r.get::<_, i64>(1)? as u64,
                    accepted_at_ms: r.get::<_, i64>(2)? as u64,
                    kind: r.get(3)?,
                    body: r.get(4)?,
                    body_digest: r.get(5)?,
                };
                let consumed_revision = r.get::<_, Option<i64>>(6)?.map(|v| v as u64);
                Ok((
                    EventRecord {
                        event,
                        consumed_revision,
                        diagnostics: Vec::new(),
                    },
                    r.get(7)?,
                ))
            },
        )
        .and_then(Iterator::collect)
        .map_err(db)?;
    let mut items = Vec::with_capacity(rows.len());
    for (mut record, diagnostics) in rows {
        if let Some(text) = diagnostics {
            record.diagnostics = serde_json::from_str::<Vec<Diagnostic>>(&text)
                .map_err(|e| fail(StoreFailureKind::Corrupt, "store.corrupt", e.to_string()))?;
        }
        items.push(record);
    }
    Ok(page(items, limit, |e: &EventRecord| {
        json!(e.event.sequence)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use runspore_store_conformance::ManualClock;

    /// No released schema has the `workers` table below the current version yet, so
    /// the registered half of the check is read directly: only a fresh heartbeat of
    /// another version counts, and a registered worker is not listed again by its
    /// attempts.
    #[test]
    fn registered_workers_of_another_version_are_live_while_their_heartbeat_is_fresh() {
        let path = std::env::temp_dir().join(format!("runspore-workers-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let clock = ManualClock::new(1_000);
        let store = SqliteStore::open_with_clock(&path, clock.clone()).unwrap();
        store.beat_worker("worker-this", "box", 7, 100).unwrap();
        let beat_old = || {
            store.beat_worker("worker-old", "box", 8, 100).unwrap();
            store
                .lock()
                .execute(
                    "UPDATE workers SET version = '0.2.9' WHERE worker_id = 'worker-old'",
                    [],
                )
                .unwrap();
        };
        beat_old();
        let live = |now| live_workers(&store.lock(), SCHEMA_VERSION_WORKERS, now).unwrap();
        assert_eq!(
            live(1_040),
            ["worker worker-old of spore 0.2.9 (pid 8 on box, last heartbeat 40 ms ago)"]
        );
        assert!(live(1_100).is_empty(), "a stale heartbeat is not live");
        clock.set(1_090);
        beat_old();
        assert_eq!(live(1_150).len(), 1, "a new heartbeat is fresh again");
        store.end_worker("worker-old").unwrap();
        assert!(live(1_150).is_empty(), "a worker that stopped is gone");
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}
