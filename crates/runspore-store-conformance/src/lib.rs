//! Adapter-independent conformance suite for `runspore_types::store::Store`.
//!
//! Scenarios S01 to S14 of `spec/store.md` section 4. An adapter implements
//! [`StoreFactory`] and invokes [`store_conformance_tests!`].

use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Barrier;

use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::model::{ActivityResult, AttemptStatus, RetryActivity, ScheduleActivity};
use runspore_types::reducer::Command;
use runspore_types::store::*;

/// A test clock that moves only when told, forwards or backwards.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
    pub fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Opens a second instance on the storage of a fixture.
pub type Reopen = Box<dyn Fn(Arc<dyn Clock>) -> Arc<dyn Store> + Send + Sync>;

/// Fresh storage and its first instance.
pub struct Fixture {
    pub store: Arc<dyn Store>,
    reopen: Reopen,
}

impl Fixture {
    pub fn new(store: Arc<dyn Store>, reopen: Reopen) -> Self {
        Self { store, reopen }
    }
    /// Another instance on the same underlying storage, as a second process would hold.
    pub fn second(&self, clock: Arc<dyn Clock>) -> Arc<dyn Store> {
        (self.reopen)(clock)
    }
}

/// Creates empty storage for one scenario. Every instance it yields uses the given clock.
pub trait StoreFactory: Send + Sync {
    fn fresh(&self, clock: Arc<dyn Clock>) -> Fixture;
}

/// The failure kind and code of `result`, which must be an error.
pub fn failure<T: Debug>(result: StoreResult<T>) -> (StoreFailureKind, String) {
    let error = result.expect_err("expected a store failure");
    (error.kind, error.code)
}

fn mutation(operation: &str, request_id: &str, body: Value) -> Mutation {
    Mutation::new(operation, request_id, &body).expect("canonical request")
}

fn evidence(value: Value) -> Evidence {
    Evidence::encode(&value).expect("canonical evidence")
}

/// A valid `create_run` for `key` whose package depends on `variant`.
pub fn create_run(key: &RunKey, request_id: &str, variant: u32) -> CreateRun {
    let package =
        canonical::encode(&json!({"format": "test", "variant": variant})).expect("package");
    let started = evidence(json!({"input": {}}));
    let package_digest = digest::package(&package);
    CreateRun {
        mutation: mutation(
            "create_run",
            request_id,
            json!([key, "start", package_digest, started.digest]),
        ),
        key: key.clone(),
        start_key: format!("start-{}", key.run),
        package_digest,
        package,
        started,
    }
}

/// An `append_event` of a signal with the given event ID and body.
pub fn append(key: &RunKey, request_id: &str, event_id: &str, body: Value) -> AppendEvent {
    let body = evidence(body);
    AppendEvent {
        mutation: mutation(
            "append_event",
            request_id,
            json!([key, event_id, "signal.received", body.digest]),
        ),
        key: key.clone(),
        event_id: event_id.to_string(),
        kind: "signal.received".to_string(),
        body,
    }
}

/// An `activity.schedule` command for the first visit of `node`, and its invocation ID.
pub fn schedule(key: &RunKey, node: &str, not_before_ms: u64) -> (Command, String) {
    let activation_id = ids::activation(node, 1);
    let invocation_id = ids::invocation(&key.tenant, &key.run, &activation_id);
    let payload = ScheduleActivity {
        invocation_id: invocation_id.clone(),
        activation_id: activation_id.clone(),
        node_id: node.to_string(),
        action_id: "echo".to_string(),
        effect_key: ids::effect_key(&invocation_id),
        attempt: 1,
        not_before_ms,
        input: json!({"node": node}),
    };
    let command = Command {
        command_id: ids::command(&key.tenant, &key.run, &activation_id, 0),
        activation_id,
        kind: "activity.schedule".to_string(),
        payload: canonical::encode(&payload).expect("payload"),
    };
    (command, invocation_id)
}

/// An `activity.retry` command authorizing `attempt` of the invocation for `node`.
pub fn retry(key: &RunKey, node: &str, attempt: u32, not_before_ms: u64) -> Command {
    let activation_id = ids::activation(node, 1);
    let invocation_id = ids::invocation(&key.tenant, &key.run, &activation_id);
    let payload = RetryActivity {
        invocation_id,
        attempt,
        not_before_ms,
    };
    Command {
        command_id: ids::command(&key.tenant, &key.run, &activation_id, attempt - 1),
        activation_id,
        kind: "activity.retry".to_string(),
        payload: canonical::encode(&payload).expect("payload"),
    }
}

/// A `commit_turn` consuming `turn`'s pending event with the given commands.
pub fn commit(turn: &Turn, request_id: &str, variant: u32, commands: Vec<Command>) -> CommitTurn {
    let revision = turn.revision + 1;
    let snapshot =
        canonical::encode(&json!({"revision": revision, "variant": variant})).expect("snapshot");
    let decision_digest = digest::body(
        &canonical::encode(&json!({"variant": variant, "commands": commands})).expect("decision"),
    );
    CommitTurn {
        mutation: mutation(
            "commit_turn",
            request_id,
            json!([turn.key, turn.revision, turn.next_event.id, decision_digest]),
        ),
        key: turn.key.clone(),
        expected_revision: turn.revision,
        event_id: turn.next_event.id.clone(),
        event_sequence: turn.next_event.sequence,
        snapshot_digest: digest::state(&snapshot),
        snapshot,
        decision_digest,
        status: "running".to_string(),
        commands,
        diagnostics: Vec::new(),
    }
}

pub fn claim(
    key: &RunKey,
    request_id: &str,
    invocation_id: &str,
    worker: &str,
    lease_ms: u64,
) -> ClaimAttempt {
    ClaimAttempt {
        mutation: mutation(
            "claim_attempt",
            request_id,
            json!([key, invocation_id, worker, lease_ms]),
        ),
        key: key.clone(),
        invocation_id: invocation_id.to_string(),
        worker: worker.to_string(),
        lease_duration_ms: lease_ms,
    }
}

pub fn heartbeat(attempt: &AttemptRef, request_id: &str, extend_ms: u64) -> Heartbeat {
    Heartbeat {
        mutation: mutation("heartbeat", request_id, json!([attempt, extend_ms])),
        attempt: attempt.clone(),
        extend_ms,
    }
}

/// A successful `ActivityResult` for the attempt, as evidence.
pub fn success(attempt: &AttemptRef, number: u32) -> Evidence {
    let result = ActivityResult {
        invocation_id: attempt.invocation_id.clone(),
        attempt_id: attempt.attempt_id.clone(),
        attempt: number,
        status: AttemptStatus::Success,
        outcome: None,
        output: json!({"ok": true}),
        error: None,
        retryable: true,
    };
    Evidence::encode(&result).expect("result")
}

pub fn finish(attempt: &AttemptRef, request_id: &str, number: u32) -> FinishAttempt {
    let result = success(attempt, number);
    FinishAttempt {
        mutation: mutation(
            "finish_attempt",
            request_id,
            json!([attempt, result.digest]),
        ),
        attempt: attempt.clone(),
        result,
    }
}

pub fn expire(attempt: &AttemptRef, request_id: &str) -> ExpireAttempt {
    ExpireAttempt {
        mutation: mutation("expire_attempt", request_id, json!([attempt])),
        attempt: attempt.clone(),
    }
}

pub fn quarantine(key: &RunKey, request_id: &str, revision: u64) -> QuarantineRun {
    QuarantineRun {
        mutation: mutation("quarantine_run", request_id, json!([key, revision])),
        key: key.clone(),
        expected_revision: revision,
        code: "kernel.trap".to_string(),
        details: "test".to_string(),
    }
}

pub fn release(key: &RunKey, request_id: &str) -> ReleaseRun {
    ReleaseRun {
        mutation: mutation("release_run", request_id, json!([key])),
        key: key.clone(),
    }
}

/// A store with one created run whose `start` event is pending.
pub struct World {
    pub fixture: Fixture,
    pub clock: Arc<ManualClock>,
    pub key: RunKey,
}

impl World {
    pub async fn new(factory: &dyn StoreFactory, run: &str) -> Self {
        let clock = ManualClock::new(1_000);
        let fixture = factory.fresh(clock.clone());
        let key = RunKey::new(DEFAULT_TENANT, run);
        let created = fixture
            .store
            .create_run(create_run(&key, "create", 0))
            .await
            .expect("create");
        assert_eq!(created.disposition, Disposition::Applied);
        Self {
            fixture,
            clock,
            key,
        }
    }
    pub fn store(&self) -> &dyn Store {
        self.fixture.store.as_ref()
    }
    pub async fn turn(&self) -> Turn {
        self.store()
            .load_turn(&self.key)
            .await
            .expect("load")
            .expect("pending event")
    }
    /// Commits the pending event with `commands`.
    pub async fn commit(&self, request_id: &str, commands: Vec<Command>) -> Receipt<CommitReceipt> {
        let turn = self.turn().await;
        self.store()
            .commit_turn(commit(&turn, request_id, 0, commands))
            .await
            .expect("commit")
    }
    /// Commits `start` scheduling node `a` due at `not_before_ms`; returns its invocation ID.
    pub async fn scheduled(&self, not_before_ms: u64) -> String {
        let (command, invocation) = schedule(&self.key, "a", not_before_ms);
        self.commit("commit-1", vec![command]).await;
        invocation
    }
    pub async fn events(&self) -> Vec<StoredEvent> {
        let page = self
            .store()
            .list_events(&self.key, 0, 1_000)
            .await
            .expect("events");
        page.items.into_iter().map(|r| r.event).collect()
    }
    pub async fn run(&self) -> RunView {
        self.store()
            .get_run(&self.key)
            .await
            .expect("get")
            .expect("run")
    }
    pub async fn invocations(&self) -> Vec<InvocationView> {
        self.store()
            .list_invocations(&self.key)
            .await
            .expect("invocations")
    }
    pub async fn due(&self) -> Vec<DueItem> {
        self.store()
            .scan_due(None, 100)
            .await
            .expect("scan_due")
            .items
    }
    pub async fn ready(&self) -> Vec<RunKey> {
        self.store()
            .scan_ready(None, 100)
            .await
            .expect("scan_ready")
            .items
    }
}

const RACES: usize = 30;

/// Runs `a` and `b` released together by a barrier on separate tasks.
async fn race<A, B, X, Y>(a: A, b: B) -> (X, Y)
where
    A: std::future::Future<Output = X> + Send + 'static,
    B: std::future::Future<Output = Y> + Send + 'static,
    X: Send + 'static,
    Y: Send + 'static,
{
    let barrier = Arc::new(Barrier::new(2));
    let (ba, bb) = (barrier.clone(), barrier);
    let ta = tokio::spawn(async move {
        ba.wait().await;
        a.await
    });
    let tb = tokio::spawn(async move {
        bb.wait().await;
        b.await
    });
    (ta.await.expect("task"), tb.await.expect("task"))
}

/// S01: two coordinators commit against one revision, on two store instances.
pub async fn s01_concurrent_commit(factory: &dyn StoreFactory) {
    for n in 0..RACES {
        let w = World::new(factory, &format!("s01-{n}")).await;
        let other = w.fixture.second(w.clock.clone());
        let turn = w.turn().await;
        let same = n % 2 == 0;
        let first = commit(&turn, "c-a", 0, vec![schedule(&w.key, "a", 0).0]);
        let second = if same {
            first.clone()
        } else {
            commit(&turn, "c-b", 1, vec![schedule(&w.key, "a", 0).0])
        };
        let (sa, sb) = (w.fixture.store.clone(), other);
        let (ra, rb) = race(async move { sa.commit_turn(first).await }, async move {
            sb.commit_turn(second).await
        })
        .await;
        let results = [ra, rb];
        let applied = results
            .iter()
            .filter(|r| matches!(r, Ok(x) if x.disposition == Disposition::Applied))
            .count();
        assert_eq!(applied, 1, "exactly one commit applies: {results:?}");
        for r in &results {
            match r {
                Ok(receipt) if receipt.disposition == Disposition::Duplicate => assert!(same),
                Ok(_) => {}
                Err(e) => {
                    assert!(!same, "identical request must not fail: {e:?}");
                    assert_eq!(
                        (e.kind, e.code.as_str()),
                        (StoreFailureKind::Stale, "revision.mismatch")
                    );
                }
            }
        }
        assert_eq!(w.run().await.revision, 1);
        assert_eq!(w.invocations().await.len(), 1);
        assert_eq!(w.due().await.len(), 1);
    }
}

/// S02: every mutation replayed after the state moved on returns its receipt unchanged.
pub async fn s02_replay_after_progress(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s02").await;
    let s = w.store();
    let invocation = w.scheduled(0).await;
    let app = append(&w.key, "append-1", "sig:1", json!({"x": 1}));
    let appended = s.append_event(app.clone()).await.unwrap();
    let cl = claim(&w.key, "claim-1", &invocation, "w1", 500);
    let claimed = s.claim_attempt(cl.clone()).await.unwrap();
    let hb = heartbeat(&claimed.value.attempt, "hb-1", 500);
    let beat = s.heartbeat(hb.clone()).await.unwrap();
    let fin = finish(&claimed.value.attempt, "finish-1", 1);
    let finished = s.finish_attempt(fin.clone()).await.unwrap();
    let qr = quarantine(&w.key, "q-1", 1);
    let quarantined = s.quarantine_run(qr.clone()).await.unwrap();
    let rl = release(&w.key, "r-1");
    let released = s.release_run(rl.clone()).await.unwrap();
    w.commit("commit-2", vec![]).await;
    w.clock.set(5_000);
    let (events, run) = (w.events().await, w.run().await);

    let replay = s.create_run(create_run(&w.key, "create", 0)).await.unwrap();
    assert_eq!(
        (replay.disposition, replay.value.started_sequence),
        (Disposition::Duplicate, 1)
    );
    let replay = s.append_event(app).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &appended.value)
    );
    let replay = s.claim_attempt(cl).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &claimed.value)
    );
    let replay = s.heartbeat(hb).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &beat.value)
    );
    let replay = s.finish_attempt(fin).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &finished.value)
    );
    let replay = s.quarantine_run(qr).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &quarantined.value)
    );
    let replay = s.release_run(rl).await.unwrap();
    assert_eq!(
        (replay.disposition, &replay.value),
        (Disposition::Duplicate, &released.value)
    );
    let stored = s
        .get_receipt(&w.key, "claim-1")
        .await
        .unwrap()
        .expect("receipt");
    assert_eq!(stored.decode::<Claim>().unwrap(), claimed.value);
    assert_eq!(stored.value, canonical::encode(&claimed.value).unwrap());
    assert_eq!(w.events().await, events);
    assert_eq!(w.run().await, run);
    assert!(w.run().await.quarantine.is_none());
}

/// S03: request-digest mismatch, and identical requests submitted concurrently.
pub async fn s03_request_identity(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s03").await;
    w.store()
        .append_event(append(&w.key, "req", "sig:a", json!(1)))
        .await
        .unwrap();
    let r = w
        .store()
        .append_event(append(&w.key, "req", "sig:b", json!(2)))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Conflict, "request.digest-mismatch".into())
    );
    assert_eq!(w.events().await.len(), 2);
    for n in 0..RACES {
        let w = World::new(factory, &format!("s03-{n}")).await;
        let (sa, sb) = (w.fixture.store.clone(), w.fixture.second(w.clock.clone()));
        let request = append(&w.key, "same", "sig:x", json!({"n": n}));
        let copy = request.clone();
        let (ra, rb) = race(async move { sa.append_event(request).await }, async move {
            sb.append_event(copy).await
        })
        .await;
        let (ra, rb) = (ra.unwrap(), rb.unwrap());
        let mut dispositions = [ra.disposition, rb.disposition];
        dispositions.sort_by_key(|d| *d == Disposition::Duplicate);
        assert_eq!(dispositions, [Disposition::Applied, Disposition::Duplicate]);
        assert_eq!(ra.value, rb.value);
        assert_eq!(w.events().await.len(), 2);
    }
}

/// S04: a result arriving after expiry loses; only the expired result enters the inbox.
pub async fn s04_result_after_expiry(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s04").await;
    let invocation = w.scheduled(0).await;
    let attempt = w
        .store()
        .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
        .await
        .unwrap()
        .value
        .attempt;
    w.clock.set(1_100);
    assert_eq!(
        w.due().await,
        vec![DueItem::Lease {
            attempt: attempt.clone()
        }]
    );
    w.store()
        .expire_attempt(expire(&attempt, "expire"))
        .await
        .unwrap();
    let late = w
        .store()
        .finish_attempt(finish(&attempt, "finish", 1))
        .await;
    assert_eq!(
        failure(late),
        (StoreFailureKind::Stale, "lease.lost".into())
    );
    let audit = RecordLateEvidence {
        mutation: mutation("late", "late", json!([attempt])),
        attempt: attempt.clone(),
        evidence: success(&attempt, 1),
    };
    w.store().record_late_evidence(audit).await.unwrap();
    let results: Vec<_> = w
        .events()
        .await
        .into_iter()
        .filter(|e| e.kind == "activity.result")
        .collect();
    assert_eq!(results.len(), 1);
    let result: ActivityResult = canonical::decode(&results[0].body).unwrap();
    assert_eq!(result, ActivityResult::expired(&invocation, 1));
    assert_eq!(w.invocations().await[0].status, InvocationStatus::Settled);
    assert!(w
        .store()
        .get_receipt(&w.key, "late")
        .await
        .unwrap()
        .is_some());
    assert!(w
        .store()
        .get_receipt(&w.key, "finish")
        .await
        .unwrap()
        .is_none());
}

/// S05: heartbeat extends only strictly before the deadline; no lease is resurrected.
pub async fn s05_heartbeat_deadline(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s05").await;
    let invocation = w.scheduled(0).await;
    let attempt = w
        .store()
        .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
        .await
        .unwrap()
        .value
        .attempt;
    w.clock.set(1_099);
    let first = w
        .store()
        .heartbeat(heartbeat(&attempt, "hb-1", 100))
        .await
        .unwrap();
    assert_eq!(first.value.lease_until_ms, 1_199);
    w.clock.set(1_199);
    let at = w.store().heartbeat(heartbeat(&attempt, "hb-2", 100)).await;
    assert_eq!(failure(at), (StoreFailureKind::Stale, "lease.lost".into()));
    w.clock.set(1_500);
    let after = w.store().heartbeat(heartbeat(&attempt, "hb-3", 100)).await;
    assert_eq!(
        failure(after),
        (StoreFailureKind::Stale, "lease.lost".into())
    );
    let replay = w
        .store()
        .heartbeat(heartbeat(&attempt, "hb-1", 100))
        .await
        .unwrap();
    assert_eq!(
        (replay.disposition, replay.value.lease_until_ms),
        (Disposition::Duplicate, 1_199)
    );
    assert_eq!(w.invocations().await[0].lease_until_ms, Some(1_199));
    w.store()
        .expire_attempt(expire(&attempt, "expire"))
        .await
        .unwrap();
    let dead = w.store().heartbeat(heartbeat(&attempt, "hb-4", 100)).await;
    assert_eq!(
        failure(dead),
        (StoreFailureKind::Stale, "lease.lost".into())
    );
}

/// S06: the same event ID with a different body is refused; the original stays.
pub async fn s06_event_id_conflict(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s06").await;
    let original = w
        .store()
        .append_event(append(&w.key, "a-1", "sig:1", json!("first")))
        .await
        .unwrap();
    let r = w
        .store()
        .append_event(append(&w.key, "a-2", "sig:1", json!("second")))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Conflict, "event.id-conflict".into())
    );
    let same = w
        .store()
        .append_event(append(&w.key, "a-3", "sig:1", json!("first")))
        .await
        .unwrap();
    assert_eq!(
        (same.disposition, &same.value),
        (Disposition::Duplicate, &original.value)
    );
    let events = w.events().await;
    assert_eq!(events.len(), 2);
    assert_eq!(
        (events[1].sequence, events[1].body.as_slice()),
        (2, b"\"first\"".as_slice())
    );
}

/// S07: concurrent appends get gapless sequences and non-decreasing times while the clock steps back.
pub async fn s07_concurrent_appends(factory: &dyn StoreFactory) {
    for n in 0..10 {
        let w = World::new(factory, &format!("s07-{n}")).await;
        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();
        for t in 0..8u64 {
            let (store, barrier, clock, key) = (
                w.fixture.second(w.clock.clone()),
                barrier.clone(),
                w.clock.clone(),
                w.key.clone(),
            );
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                for k in 0..5u64 {
                    clock.set(2_000 + ((t * 7 + k * 13) % 11) * 100);
                    let id = format!("sig:{t}-{k}");
                    store
                        .append_event(append(&key, &id, &id, json!([t, k])))
                        .await
                        .unwrap();
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let events = w.events().await;
        assert_eq!(events.len(), 41);
        for (index, pair) in events.windows(2).enumerate() {
            assert_eq!(pair[0].sequence, index as u64 + 1);
            assert_eq!(pair[1].sequence, pair[0].sequence + 1);
            assert!(pair[1].accepted_at_ms >= pair[0].accepted_at_ms);
        }
        assert_eq!(w.run().await.next_sequence, 42);
    }
}

/// S08: a commit whose second command is invalid writes nothing.
pub async fn s08_atomic_commit(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s08").await;
    let turn = w.turn().await;
    let request = commit(
        &turn,
        "bad",
        0,
        vec![schedule(&w.key, "a", 0).0, retry(&w.key, "b", 2, 0)],
    );
    let r = w.store().commit_turn(request).await;
    assert_eq!(
        failure(r),
        (
            StoreFailureKind::Conflict,
            "invocation.retry-invalid".into()
        )
    );
    let run = w.run().await;
    assert_eq!(
        (run.revision, run.applied_sequence, run.status.as_str()),
        (0, 0, "created")
    );
    assert!(w.invocations().await.is_empty());
    assert!(w
        .store()
        .get_receipt(&w.key, "bad")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        w.store().list_events(&w.key, 0, 10).await.unwrap().items[0].consumed_revision,
        None
    );
    assert_eq!(w.ready().await, vec![w.key.clone()]);
    w.commit("good", vec![schedule(&w.key, "a", 0).0]).await;
    assert_eq!(w.invocations().await.len(), 1);
}

/// S09: an invocation with a future `notBeforeMs` is neither due nor claimable until then.
pub async fn s09_not_before(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s09").await;
    let invocation = w.scheduled(5_000).await;
    assert!(w.due().await.is_empty());
    let r = w
        .store()
        .claim_attempt(claim(&w.key, "early", &invocation, "w1", 100))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Conflict, "invocation.not-due".into())
    );
    assert!(w
        .store()
        .get_receipt(&w.key, "early")
        .await
        .unwrap()
        .is_none());
    w.clock.set(5_000);
    assert_eq!(
        w.due().await,
        vec![DueItem::Outbox {
            key: w.key.clone(),
            invocation_id: invocation.clone()
        }]
    );
    let claimed = w
        .store()
        .claim_attempt(claim(&w.key, "due", &invocation, "w1", 100))
        .await
        .unwrap();
    assert_eq!(claimed.value.lease_until_ms, 5_100);
    assert_eq!(w.invocations().await[0].status, InvocationStatus::Running);
}

/// S10: fences strictly increase across attempts; an old fence can neither heartbeat nor finish.
pub async fn s10_fences(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s10").await;
    let invocation = w.scheduled(0).await;
    let first = w
        .store()
        .claim_attempt(claim(&w.key, "claim-1", &invocation, "w1", 100))
        .await
        .unwrap()
        .value;
    w.clock.set(1_100);
    w.store()
        .expire_attempt(expire(&first.attempt, "expire-1"))
        .await
        .unwrap();
    w.commit("commit-2", vec![retry(&w.key, "a", 2, 1_100)])
        .await;
    let second = w
        .store()
        .claim_attempt(claim(&w.key, "claim-2", &invocation, "w1", 100))
        .await
        .unwrap()
        .value;
    assert_eq!(second.attempt_number, 2);
    assert!(second.attempt.fence > first.attempt.fence);
    let mut stale_ref = second.attempt.clone();
    stale_ref.fence = first.attempt.fence;
    let r = w
        .store()
        .heartbeat(heartbeat(&stale_ref, "hb-old", 100))
        .await;
    assert_eq!(failure(r), (StoreFailureKind::Stale, "lease.lost".into()));
    let r = w
        .store()
        .finish_attempt(finish(&stale_ref, "fin-old", 2))
        .await;
    assert_eq!(failure(r), (StoreFailureKind::Stale, "lease.lost".into()));
    let r = w
        .store()
        .finish_attempt(finish(&first.attempt, "fin-first", 1))
        .await;
    assert_eq!(failure(r), (StoreFailureKind::Stale, "lease.lost".into()));
    w.store()
        .finish_attempt(finish(&second.attempt, "fin-new", 2))
        .await
        .unwrap();
    assert_eq!(w.invocations().await[0].status, InvocationStatus::Settled);
}

/// S11: a quarantined run is hidden and refused but still accepts events.
pub async fn s11_quarantine(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s11").await;
    let invocation = w.scheduled(0).await;
    w.store()
        .append_event(append(&w.key, "a-1", "sig:1", json!(1)))
        .await
        .unwrap();
    let r = w.store().quarantine_run(quarantine(&w.key, "q-0", 0)).await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Stale, "revision.mismatch".into())
    );
    assert!(
        w.store()
            .quarantine_run(quarantine(&w.key, "q-1", 1))
            .await
            .unwrap()
            .value
            .quarantined
    );
    assert!(
        w.store()
            .quarantine_run(quarantine(&w.key, "q-2", 1))
            .await
            .unwrap()
            .value
            .quarantined
    );
    assert_eq!(
        w.run().await.quarantine.expect("quarantine").code,
        "kernel.trap"
    );
    assert!(w.store().load_turn(&w.key).await.unwrap().is_none());
    assert!(w.ready().await.is_empty());
    assert!(w.due().await.is_empty());
    let r = w
        .store()
        .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Stale, "run.quarantined".into())
    );
    let pending = w.store().list_events(&w.key, 1, 1).await.unwrap().items[0]
        .event
        .clone();
    let turn = Turn {
        key: w.key.clone(),
        revision: 1,
        applied_sequence: 1,
        package_digest: String::new(),
        snapshot: None,
        snapshot_digest: None,
        next_event: pending,
    };
    let r = w
        .store()
        .commit_turn(commit(&turn, "commit-2", 0, vec![]))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Stale, "run.quarantined".into())
    );
    w.store()
        .append_event(append(&w.key, "a-2", "sig:2", json!(2)))
        .await
        .unwrap();
    assert_eq!(w.events().await.len(), 3);
    assert!(
        !w.store()
            .release_run(release(&w.key, "r-1"))
            .await
            .unwrap()
            .value
            .quarantined
    );
    assert!(
        !w.store()
            .release_run(release(&w.key, "r-2"))
            .await
            .unwrap()
            .value
            .quarantined
    );
    assert!(w.run().await.quarantine.is_none());
    assert_eq!(w.ready().await, vec![w.key.clone()]);
    assert_eq!(w.due().await.len(), 1);
    w.store()
        .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
        .await
        .unwrap();
    w.commit("commit-2", vec![]).await;
}

/// S12: finish and expire race across the deadline; exactly one wins.
pub async fn s12_finish_expire_race(factory: &dyn StoreFactory) {
    for n in 0..RACES {
        let w = World::new(factory, &format!("s12-{n}")).await;
        let invocation = w.scheduled(0).await;
        let attempt = w
            .store()
            .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
            .await
            .unwrap()
            .value
            .attempt;
        w.clock.set(1_099);
        let (sa, sb, clock) = (
            w.fixture.store.clone(),
            w.fixture.second(w.clock.clone()),
            w.clock.clone(),
        );
        let (fa, ea) = (attempt.clone(), attempt.clone());
        let ticker = tokio::spawn(async move {
            tokio::task::yield_now().await;
            clock.set(1_100);
        });
        let (finished, expired) = race(
            async move { sa.finish_attempt(finish(&fa, "finish", 1)).await },
            async move {
                loop {
                    match sb.expire_attempt(expire(&ea, "expire")).await {
                        Err(e) if e.code == "lease.not-due" => tokio::task::yield_now().await,
                        other => return other,
                    }
                }
            },
        )
        .await;
        ticker.await.unwrap();
        assert_eq!(
            finished.is_ok() as u32 + expired.is_ok() as u32,
            1,
            "{finished:?} {expired:?}"
        );
        for loser in [finished.err(), expired.err()].into_iter().flatten() {
            assert_eq!(
                (loser.kind, loser.code.as_str()),
                (StoreFailureKind::Stale, "lease.lost")
            );
        }
        assert_eq!(
            w.events()
                .await
                .iter()
                .filter(|e| e.kind == "activity.result")
                .count(),
            1
        );
    }
}

/// S13: `create_run` repeated under another request ID, and with a changed package.
pub async fn s13_create_run_repeat(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s13").await;
    let again = w
        .store()
        .create_run(create_run(&w.key, "create-again", 0))
        .await
        .unwrap();
    assert_eq!(
        (again.disposition, again.value.started_sequence),
        (Disposition::Duplicate, 1)
    );
    let r = w
        .store()
        .create_run(create_run(&w.key, "create-changed", 1))
        .await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Conflict, "run.exists".into())
    );
    let mut other = create_run(&RunKey::new(DEFAULT_TENANT, "s13-other"), "create-other", 0);
    other.start_key = format!("start-{}", w.key.run);
    let r = w.store().create_run(other).await;
    assert_eq!(
        failure(r),
        (StoreFailureKind::Conflict, "run.start-key-mismatch".into())
    );
    assert_eq!(w.events().await.len(), 1);
    let runs = w
        .store()
        .list_runs(DEFAULT_TENANT, None, 10)
        .await
        .unwrap()
        .items;
    assert_eq!(runs.len(), 1);
    assert!(w
        .store()
        .get_package(&runs[0].package_digest)
        .await
        .unwrap()
        .is_some());
}

/// S14: reserved kinds and oversize bodies are refused by `append_event`.
pub async fn s14_append_limits(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s14").await;
    for kind in ["run.started", "activity.result"] {
        let mut request = append(&w.key, kind, "e", json!(1));
        request.kind = kind.to_string();
        assert_eq!(
            failure(w.store().append_event(request).await),
            (StoreFailureKind::Conflict, "event.reserved-kind".into())
        );
    }
    let big = append(&w.key, "big", "sig:big", json!("x".repeat(64 * 1024)));
    assert_eq!(
        failure(w.store().append_event(big).await),
        (StoreFailureKind::Quota, "event.too-large".into())
    );
    let fits = append(&w.key, "fits", "sig:fits", json!("x".repeat(64 * 1024 - 2)));
    w.store().append_event(fits).await.unwrap();
    assert_eq!(w.events().await.len(), 2);
    assert!(w
        .store()
        .get_receipt(&w.key, "big")
        .await
        .unwrap()
        .is_none());
}

/// Expands to one `#[tokio::test]` per scenario, run against `$factory`.
#[macro_export]
macro_rules! store_conformance_tests {
    ($factory:expr) => {
        $crate::store_conformance_tests!(@each $factory;
            s01_concurrent_commit s02_replay_after_progress s03_request_identity
            s04_result_after_expiry s05_heartbeat_deadline s06_event_id_conflict
            s07_concurrent_appends s08_atomic_commit s09_not_before s10_fences
            s11_quarantine s12_finish_expire_race s13_create_run_repeat s14_append_limits);
    };
    (@each $factory:expr; $($name:ident)*) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            async fn $name() {
                $crate::$name(&$factory).await;
            }
        )*
    };
}
