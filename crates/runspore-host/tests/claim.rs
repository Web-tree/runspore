//! A `duplicate` claim receipt is not ownership: the engine heartbeats first.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use runspore_host::{ActivityRegistry, Engine, EngineConfig};
use runspore_store_conformance::{expire, ManualClock};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::*;
use serde_json::json;

#[path = "engine.rs"]
#[allow(dead_code)]
mod scripted;

/// Applies the first claim, optionally lets its lease expire, then reports the
/// acknowledgement as lost.
struct LostAck {
    inner: Arc<SqliteStore>,
    clock: Arc<ManualClock>,
    expire_first: bool,
    fired: AtomicBool,
}

#[async_trait]
impl Store for LostAck {
    async fn claim_attempt(&self, r: ClaimAttempt) -> StoreResult<Receipt<Claim>> {
        let receipt = self.inner.claim_attempt(r).await?;
        if self.fired.swap(true, Ordering::SeqCst) {
            return Ok(receipt);
        }
        if self.expire_first {
            self.clock.set(receipt.value.lease_until_ms + 1);
            self.inner
                .expire_attempt(expire(&receipt.value.attempt, "x"))
                .await?;
        }
        Err(StoreFailure::new(
            StoreFailureKind::Unavailable,
            "io",
            "ack lost",
        ))
    }
    async fn capabilities(&self) -> StoreResult<Capabilities> {
        self.inner.capabilities().await
    }
    async fn create_run(&self, r: CreateRun) -> StoreResult<Receipt<RunCreated>> {
        self.inner.create_run(r).await
    }
    async fn append_event(&self, r: AppendEvent) -> StoreResult<Receipt<EventAccepted>> {
        self.inner.append_event(r).await
    }
    async fn load_turn(&self, k: &RunKey) -> StoreResult<Option<Turn>> {
        self.inner.load_turn(k).await
    }
    async fn get_package(&self, d: &str) -> StoreResult<Option<Vec<u8>>> {
        self.inner.get_package(d).await
    }
    async fn commit_turn(&self, r: CommitTurn) -> StoreResult<Receipt<CommitReceipt>> {
        self.inner.commit_turn(r).await
    }
    async fn get_receipt(&self, k: &RunKey, id: &str) -> StoreResult<Option<StoredReceipt>> {
        self.inner.get_receipt(k, id).await
    }
    async fn heartbeat(&self, r: Heartbeat) -> StoreResult<Receipt<LeaseExtended>> {
        self.inner.heartbeat(r).await
    }
    async fn finish_attempt(&self, r: FinishAttempt) -> StoreResult<Receipt<ResultAccepted>> {
        self.inner.finish_attempt(r).await
    }
    async fn expire_attempt(&self, r: ExpireAttempt) -> StoreResult<Receipt<ResultAccepted>> {
        self.inner.expire_attempt(r).await
    }
    async fn record_late_evidence(
        &self,
        r: RecordLateEvidence,
    ) -> StoreResult<Receipt<AuditRecorded>> {
        self.inner.record_late_evidence(r).await
    }
    async fn quarantine_run(&self, r: QuarantineRun) -> StoreResult<Receipt<QuarantineChanged>> {
        self.inner.quarantine_run(r).await
    }
    async fn release_run(&self, r: ReleaseRun) -> StoreResult<Receipt<QuarantineChanged>> {
        self.inner.release_run(r).await
    }
    async fn scan_ready(&self, c: Option<String>, l: u32) -> StoreResult<Page<RunKey>> {
        self.inner.scan_ready(c, l).await
    }
    async fn scan_due(&self, c: Option<String>, l: u32) -> StoreResult<Page<DueItem>> {
        self.inner.scan_due(c, l).await
    }
    async fn get_run(&self, k: &RunKey) -> StoreResult<Option<RunView>> {
        self.inner.get_run(k).await
    }
    async fn list_runs(&self, t: &str, c: Option<String>, l: u32) -> StoreResult<Page<RunView>> {
        self.inner.list_runs(t, c, l).await
    }
    async fn list_events(&self, k: &RunKey, a: u64, l: u32) -> StoreResult<Page<EventRecord>> {
        self.inner.list_events(k, a, l).await
    }
    async fn list_invocations(&self, k: &RunKey) -> StoreResult<Vec<InvocationView>> {
        self.inner.list_invocations(k).await
    }
}

async fn run_once(expire_first: bool) -> (usize, Vec<serde_json::Value>) {
    let path = scripted::temp_db(if expire_first {
        "dup-expired"
    } else {
        "dup-held"
    });
    let clock = ManualClock::new(1_000);
    let inner = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    let store = Arc::new(LostAck {
        inner,
        clock,
        expire_first,
        fired: AtomicBool::new(false),
    });
    let count = Arc::new(AtomicUsize::new(0));
    let reducer = Arc::new(scripted::Scripted(Box::new(scripted::one_step)));
    let registry = ActivityRegistry::with_builtins(scripted::counting_echo(count.clone()));
    let engine = Engine::new(store, reducer, registry, EngineConfig::default());
    let key = engine
        .start(scripted::WORKFLOW, json!({}), "dup")
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    (
        count.load(Ordering::SeqCst),
        scripted::results(&engine, &key).await,
    )
}

#[tokio::test]
async fn a_held_duplicate_claim_is_run_after_a_successful_heartbeat() {
    let (runs, results) = run_once(false).await;
    assert_eq!(runs, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "success");
}

#[tokio::test]
async fn a_duplicate_claim_whose_lease_is_gone_is_not_run() {
    let (runs, results) = run_once(true).await;
    assert_eq!(runs, 0);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "expired");
}
