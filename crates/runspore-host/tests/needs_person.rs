//! The needs-person mark (spec/host.md §2, Client operations, H45): the real kernel and
//! SQLite store, an actor that claims an external step and marks it. The async command
//! marked through its `RUNSPORE_TOKEN` is checked through the binary, in
//! `runspore-cli/tests/needs_person.rs`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use runspore_host::{ActivityRegistry, Engine, EngineConfig, NativeRunner, StepState};
use runspore_kernel::NativeReducer;
use runspore_store_conformance::ManualClock;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, NeedsPerson, RunKey};
use serde_json::json;

const T0: u64 = 1_000_000;
const PROMPT: &str = "Claude needs your permission to use Bash";

/// `review`, an `idempotent` external step for actor `reviewer`, tried at most twice,
/// then `done`.
fn review() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2",
        "name": "review",
        "start": "review",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "idempotent",
                       "outcomes": ["approved"], "retry": {"maxAttempts": 2},
                       "async": {"timeoutMs": "60000", "leaseMs": "10000"}}
        },
        "nodes": {
            "review": {"kind": "activity", "action": "review", "outcomes": {"approved": "done"}},
            "done": {"kind": "complete"}
        }
    }))
    .unwrap()
}

struct World {
    engine: Arc<Engine>,
    clock: Arc<ManualClock>,
    key: RunKey,
}

async fn world(name: &str) -> World {
    let clock = ManualClock::new(T0);
    let store = Arc::new(SqliteStore::open_with_clock(temp_db(name), clock.clone()).unwrap());
    let engine = Engine::new(
        store,
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        EngineConfig {
            poll_ms: 5,
            ..EngineConfig::default()
        },
    );
    let key = engine.start(&review(), json!({}), name).await.unwrap().key;
    engine.run_until_parked(&key).await.unwrap();
    World {
        engine: Arc::new(engine),
        clock,
        key,
    }
}

impl World {
    /// Claims the review step; its token.
    async fn claim(&self) -> String {
        let claim = self
            .engine
            .claim_step(Some(&self.key.run), Some("reviewer"), None)
            .await
            .unwrap()
            .expect("a ready step");
        claim.token.to_string()
    }

    async fn marked(&self) -> Option<NeedsPerson> {
        let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
        let step = self.engine.async_step(&view).await.unwrap();
        step.and_then(|s| s.needs_person)
    }

    async fn mark(&self, token: &str, reason: Option<&str>, id: &str) -> Disposition {
        self.engine
            .needs_person(token, reason, id)
            .await
            .unwrap()
            .disposition
    }
}

fn mark(reason: &str, since_ms: u64) -> Option<NeedsPerson> {
    Some(NeedsPerson {
        reason: reason.into(),
        since_ms,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h45_a_mark_is_set_kept_and_cleared_and_moves_nothing() {
    let w = world("h45-mark").await;
    let token = w.claim().await;
    let before = w.engine.get_run(&w.key).await.unwrap().unwrap();
    let invocation = w.engine.list_invocations(&w.key).await.unwrap().remove(0);
    let events_before = events(&w.engine, &w.key).await.len();

    let set = w
        .engine
        .needs_person(&token, Some(PROMPT), "n1")
        .await
        .unwrap();
    assert_eq!(
        (set.node_id.as_str(), set.attempt, set.disposition),
        ("review", 1, Disposition::Applied)
    );
    assert_eq!(set.needs_person, mark(PROMPT, T0));
    let view = w.engine.get_run(&w.key).await.unwrap().unwrap();
    let step = w.engine.async_step(&view).await.unwrap().unwrap();
    assert_eq!(
        (step.state, step.needs_person),
        (StepState::Held, mark(PROMPT, T0))
    );

    w.clock.set(T0 + 500);
    assert_eq!(
        w.mark(&token, Some(PROMPT), "n1").await,
        Disposition::Duplicate
    );
    assert_eq!(
        w.mark(&token, Some("asks"), "n2").await,
        Disposition::Applied
    );
    assert_eq!(w.marked().await, mark("asks", T0));
    assert_eq!(w.mark(&token, None, "c1").await, Disposition::Applied);
    assert_eq!(w.mark(&token, None, "c2").await, Disposition::Duplicate);
    assert_eq!(w.marked().await, None);

    let after = w.engine.get_run(&w.key).await.unwrap().unwrap();
    assert_eq!(
        (after.revision, after.updated_at_ms),
        (before.revision, before.updated_at_ms)
    );
    let shown = w.engine.list_invocations(&w.key).await.unwrap().remove(0);
    assert_eq!(shown.lease_until_ms, invocation.lease_until_ms);
    assert_eq!(events(&w.engine, &w.key).await.len(), events_before);

    for reason in ["", &"x".repeat(1025)] {
        let refused = w.engine.needs_person(&token, Some(reason), "bad").await;
        assert_eq!(refused.unwrap_err().code(), "step.reason-invalid");
    }
    let unknown = w
        .engine
        .needs_person("not-a-token", Some(PROMPT), "x")
        .await;
    assert_eq!(unknown.unwrap_err().code(), "step.token-invalid");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h45_the_mark_ends_with_its_attempt() {
    let w = world("h45-ends").await;
    let first = w.claim().await;
    w.mark(&first, Some(PROMPT), "n1").await;
    w.engine.abandon_step(&w.key).await.unwrap();
    assert_eq!(w.mark(&first, None, "c1").await, Disposition::Duplicate);
    let late = w.engine.needs_person(&first, Some(PROMPT), "n2").await;
    assert_eq!(late.unwrap_err().code(), "lease.lost");

    w.engine.run_until_parked(&w.key).await.unwrap();
    let second = w.claim().await;
    assert_eq!(w.marked().await, None, "the next attempt starts unmarked");
    w.mark(&second, Some(PROMPT), "n3").await;
    w.engine
        .complete_step(&second, "approved", json!(null))
        .await
        .unwrap();
    assert_eq!(w.mark(&second, None, "c2").await, Disposition::Duplicate);
    let done = w.engine.run_until_parked(&w.key).await.unwrap();
    assert_eq!(done.status, "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h45_follow_returns_at_a_new_mark_only() {
    let w = world("h45-follow").await;
    let token = w.claim().await;
    let follow = |w: &World| {
        let (engine, key) = (w.engine.clone(), w.key.clone());
        tokio::spawn(async move { engine.follow(&key).await.unwrap() })
    };

    let first = follow(&w);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!first.is_finished(), "follow returned at a held step");
    w.mark(&token, Some(PROMPT), "n1").await;
    let view = tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .expect("follow returns at the mark")
        .unwrap();
    assert_eq!(view.status, "running");

    // Begun with the mark set, it waits on through another reason, and returns once
    // the mark is set anew.
    let second = follow(&w);
    tokio::time::sleep(Duration::from_millis(100)).await;
    w.mark(&token, Some("asks"), "n2").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !second.is_finished(),
        "follow returned at the mark it began with"
    );
    w.mark(&token, None, "c1").await;
    w.clock.set(T0 + 10);
    w.mark(&token, Some(PROMPT), "n3").await;
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("follow returns at the new mark")
        .unwrap();
}
