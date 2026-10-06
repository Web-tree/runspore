//! A `duplicate` claim receipt is not ownership: the engine heartbeats first.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use runspore_host::{ActivityRegistry, Engine, EngineConfig};
use runspore_store_conformance::{expire, ManualClock};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::*;
use serde_json::json;

/// Applies the first claim, optionally lets its lease expire, then reports the
/// acknowledgement as lost.
struct LostAck {
    clock: Arc<ManualClock>,
    expire_first: bool,
    fired: AtomicBool,
}

#[async_trait]
impl Hook for LostAck {
    async fn claimed(
        &self,
        inner: &SqliteStore,
        receipt: Receipt<Claim>,
    ) -> StoreResult<Receipt<Claim>> {
        if self.fired.swap(true, Ordering::SeqCst) {
            return Ok(receipt);
        }
        if self.expire_first {
            self.clock.set(receipt.value.lease_until_ms + 1);
            inner
                .expire_attempt(expire(&receipt.value.attempt, "x"))
                .await?;
        }
        Err(StoreFailure::new(
            StoreFailureKind::Unavailable,
            "io",
            "ack lost",
        ))
    }
}

async fn run_once(expire_first: bool) -> (usize, Vec<serde_json::Value>) {
    let path = temp_db(if expire_first {
        "dup-expired"
    } else {
        "dup-held"
    });
    let clock = ManualClock::new(1_000);
    let inner = Arc::new(SqliteStore::open_with_clock(&path, clock.clone()).unwrap());
    let store = Arc::new(Hooked {
        inner,
        hook: LostAck {
            clock,
            expire_first,
            fired: AtomicBool::new(false),
        },
    });
    let count = Arc::new(AtomicUsize::new(0));
    let reducer = Arc::new(Scripted(Box::new(one_step)));
    let registry = ActivityRegistry::with_builtins(counting_echo(count.clone()));
    let engine = Engine::new(store, reducer, registry, EngineConfig::default());
    let key = engine.start(WORKFLOW, json!({}), "dup").await.unwrap().key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    (count.load(Ordering::SeqCst), results(&engine, &key).await)
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
