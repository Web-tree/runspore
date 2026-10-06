//! SQLite adapter for the Runspore store protocol (`spec/store.md`).
//!
//! One database file, shared by every `SqliteStore` opened on it, in this process or
//! in others on the same host. Each mutation is one `BEGIN IMMEDIATE` transaction.

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
pub const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = include_str!("schema.sql");
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

/// A `Store` on one SQLite file. Calls run synchronously behind a mutex; the lock is
/// never held across an `.await`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
}

type R<T> = StoreResult<T>;

fn fail(kind: StoreFailureKind, code: &str, message: impl Into<String>) -> StoreFailure {
    StoreFailure::new(kind, code, message)
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
}

fn run_row(c: &Connection, key: &RunKey) -> R<Option<RunRow>> {
    c.query_row(
        "SELECT start_key, package_digest, started_digest, revision, applied_sequence,
                next_sequence, last_accepted_ms, fence, quarantine_code IS NOT NULL
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
}

/// The attempt named by `a` if its invocation, owner and fence all match.
fn attempt_row(c: &Connection, a: &AttemptRef) -> R<Option<AttemptRow>> {
    c.query_row(
        "SELECT status, lease_until_ms, attempt_number FROM attempts
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
        "UPDATE attempts SET status = ?4, result = ?5 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
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
    quarantine_details, quarantine_at_ms, created_at_ms, updated_at_ms FROM runs";

fn run_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunView> {
    let code: Option<String> = r.get(10)?;
    Ok(RunView {
        key: RunKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?),
        start_key: r.get(2)?,
        package_digest: r.get(3)?,
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
    pub fn open(path: impl AsRef<Path>) -> R<Self> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// Opens or initializes the database at `path`. A newer schema version is
    /// `incompatible` / `schema.too-new`.
    pub fn open_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> R<Self> {
        let mut conn = Connection::open(path).map_err(db)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(db)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(db)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(db)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(db)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db)?;
        if version > SCHEMA_VERSION {
            return Err(fail(
                StoreFailureKind::Incompatible,
                "schema.too-new",
                format!("schema version {version} is newer than {SCHEMA_VERSION}"),
            ));
        }
        if version == 0 {
            tx.execute_batch(SCHEMA).map_err(db)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(Self {
            conn: Mutex::new(conn),
            clock,
        })
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
                let inserted = c
                    .execute(
                        "INSERT OR IGNORE INTO invocations VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', 1, ?9)",
                        params![q.key.tenant, q.key.run, s.invocation_id, s.node_id, s.action_id, s.effect_key, input, digest::body(&input), i(s.not_before_ms)],
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
            "SELECT node_id, action_id, effect_key, input, input_digest, status, attempt_number, not_before_ms
             FROM invocations WHERE tenant = ?1 AND run_id = ?2 AND invocation_id = ?3",
            params![q.key.tenant, q.key.run, q.invocation_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, u32>(6)?,
                    r.get::<_, i64>(7)? as u64,
                ))
            },
        )
        .optional()
        .map_err(db)?;
    let Some((node_id, action_id, effect_key, input, input_digest, status, number, not_before)) =
        row
    else {
        return Err(fail(
            StoreFailureKind::NotFound,
            "invocation.not-found",
            "invocation.not-found",
        ));
    };
    let run = existing_run(c, &q.key)?;
    if run.quarantined {
        return Err(stale("run.quarantined"));
    }
    if status != "pending" {
        return Err(conflict("invocation.not-claimable"));
    }
    if now < not_before {
        return Err(conflict("invocation.not-due"));
    }
    let fence = run.fence + 1;
    let attempt_id = ids::attempt(&q.invocation_id, number);
    let lease_until_ms = now.saturating_add(q.lease_duration_ms);
    c.execute(
        "UPDATE runs SET fence = ?3 WHERE tenant = ?1 AND run_id = ?2",
        params![q.key.tenant, q.key.run, i(fence)],
    )
    .map_err(db)?;
    c.execute(
        "INSERT INTO attempts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'running', NULL)",
        params![
            q.key.tenant,
            q.key.run,
            attempt_id,
            q.invocation_id,
            number,
            q.worker,
            i(fence),
            i(lease_until_ms)
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
    })
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
        self.mutate("create_run", Some("create-run"), &q.key, &q.mutation, |c, now| {
            if q.package_digest != digest::package(&q.package) {
                return Err(conflict("package.digest-mismatch"));
            }
            verify(&q.started)?;
            let created = RunCreated { key: q.key.clone(), started_sequence: 1 };
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
            c.execute(
                "INSERT INTO runs VALUES (?1, ?2, ?3, ?4, ?5, 'created', 0, 0, 1, 0, NULL, NULL, 0, NULL, NULL, NULL, ?6, ?6)",
                params![q.key.tenant, q.key.run, q.start_key, q.package_digest, q.started.digest, i(now)],
            )
            .map_err(db)?;
            push_event(c, &q.key, ids::EVENT_STARTED, "run.started", &q.started.body, now)?;
            Ok(Outcome::Applied(created))
        })
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
            if view.quarantine.is_some() || view.applied_sequence + 1 == view.next_sequence {
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
            match attempt_row(c, &q.attempt)? {
                Some(a) if a.status == "running" && now < a.lease_until_ms => {}
                _ => return Err(stale("lease.lost")),
            }
            let lease_until_ms = now.saturating_add(q.extend_ms);
            c.execute(
                "UPDATE attempts SET lease_until_ms = ?4 WHERE tenant = ?1 AND run_id = ?2 AND attempt_id = ?3",
                params![q.attempt.key.tenant, q.attempt.key.run, q.attempt.attempt_id, i(lease_until_ms)],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(LeaseExtended { lease_until_ms }))
        })
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
                if now < row.lease_until_ms {
                    return Err(conflict("lease.not-due"));
                }
                let body = encode(&ActivityResult::expired(
                    &q.attempt.invocation_id,
                    row.attempt_number,
                ))?;
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
            c.execute(
                "INSERT INTO audit (tenant, run_id, attempt_id, evidence_digest, evidence, at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![a.key.tenant, a.key.run, a.attempt_id, q.evidence.digest, q.evidence.body, i(now)],
            )
            .map_err(db)?;
            Ok(Outcome::Applied(AuditRecorded { audit_id: format!("audit_{}", c.last_insert_rowid()) }))
        })
    }

    async fn quarantine_run(&self, q: QuarantineRun) -> R<Receipt<QuarantineChanged>> {
        self.mutate("quarantine_run", None, &q.key, &q.mutation, |c, now| {
            if existing_run(c, &q.key)?.revision != q.expected_revision {
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
                        "SELECT tenant, run_id, invocation_id, attempt_id, owner, fence, lease_until_ms FROM attempts
                         WHERE status = 'running' AND lease_until_ms <= ?1
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
                        Ok((DueItem::Lease { attempt }, cursor))
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
                            a.lease_until_ms, a.owner
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
                    Ok(InvocationView {
                        invocation_id: r.get(0)?,
                        node_id: r.get(1)?,
                        action_id: r.get(2)?,
                        status,
                        attempt_number: r.get(4)?,
                        not_before_ms: r.get::<_, i64>(5)? as u64,
                        lease_until_ms: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                        owner: r.get(7)?,
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
