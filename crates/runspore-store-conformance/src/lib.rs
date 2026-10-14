//! Adapter-independent conformance suite for `runspore_types::store::Store`.
//!
//! Scenarios S01 to S22 of `spec/store.md` section 4. An adapter implements
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
        base_dir: None,
        artifact: None,
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
    schedule_with(key, node, not_before_ms, None)
}

/// The same, for a node with `params` (or none).
pub fn schedule_with(
    key: &RunKey,
    node: &str,
    not_before_ms: u64,
    params: Option<Value>,
) -> (Command, String) {
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
        params,
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
        holder: None,
        deadline_duration_ms: None,
    }
}

/// A claim held outside the worker: with a holder handle and a deadline.
pub fn claim_held(
    key: &RunKey,
    request_id: &str,
    invocation_id: &str,
    lease_ms: u64,
    holder: Option<Value>,
    deadline_ms: u64,
) -> ClaimAttempt {
    let holder = holder.map(|h| canonical::encode(&h).expect("holder"));
    ClaimAttempt {
        mutation: mutation(
            "claim_attempt",
            request_id,
            json!([key, invocation_id, "actor", lease_ms, holder, deadline_ms]),
        ),
        holder,
        deadline_duration_ms: Some(deadline_ms),
        ..claim(key, request_id, invocation_id, "actor", lease_ms)
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
    expire_for(attempt, request_id, ExpireReason::Lease)
}

/// An `expire_attempt` with the given reason.
pub fn expire_for(attempt: &AttemptRef, request_id: &str, reason: ExpireReason) -> ExpireAttempt {
    ExpireAttempt {
        mutation: mutation(
            "expire_attempt",
            request_id,
            json!([attempt, false, reason]),
        ),
        attempt: attempt.clone(),
        process_unchecked: false,
        reason,
    }
}

/// A process record on host `host` for group `pgid`.
pub fn process_record(host: &str, pgid: i32) -> ProcessRecord {
    ProcessRecord {
        host: host.to_string(),
        boot: "boot-1".to_string(),
        pgid,
        started: "1700000000000000".to_string(),
    }
}

pub fn set_process(
    attempt: &AttemptRef,
    request_id: &str,
    process: Option<ProcessRecord>,
) -> SetProcess {
    SetProcess {
        mutation: mutation("set_process", request_id, json!([attempt, process])),
        attempt: attempt.clone(),
        process,
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
            attempt: attempt.clone(),
            process: None,
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

/// S15: two starts race with one start key and two run IDs; `find_run` names the winner.
pub async fn s15_start_key_race(factory: &dyn StoreFactory) {
    for n in 0..RACES {
        let clock = ManualClock::new(1_000);
        let fixture = factory.fresh(clock.clone());
        let start_key = format!("s15-{n}");
        let request = |run: &str| {
            let mut request = create_run(&RunKey::new(DEFAULT_TENANT, run), "start", 0);
            request.start_key = start_key.clone();
            request
        };
        let (a, b) = (request("s15-a"), request("s15-b"));
        let (sa, sb) = (fixture.store.clone(), fixture.second(clock));
        let (ra, rb) = race(async move { sa.create_run(a).await }, async move {
            sb.create_run(b).await
        })
        .await;
        let (winner, loser) = match (ra, rb) {
            (Ok(won), Err(lost)) | (Err(lost), Ok(won)) => (won, lost),
            other => panic!("expected one winner, got {other:?}"),
        };
        assert_eq!(winner.disposition, Disposition::Applied);
        assert_eq!(
            (loser.kind, loser.code),
            (StoreFailureKind::Conflict, "run.start-key-mismatch".into())
        );
        let store = &fixture.store;
        assert_eq!(
            store.find_run(DEFAULT_TENANT, &start_key).await.unwrap(),
            Some(winner.value.key)
        );
        assert_eq!(
            store
                .list_runs(DEFAULT_TENANT, None, 10)
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            store.find_run(DEFAULT_TENANT, "s15-unknown").await.unwrap(),
            None
        );
        assert_eq!(store.find_run("other", &start_key).await.unwrap(), None);
    }
}

/// S16: `set_process` holds `heartbeat`'s predicates, extends nothing, and its record
/// shows in `list_invocations` and in the `Lease` item of `scan_due`.
pub async fn s16_process_record(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s16").await;
    let invocation = w.scheduled(0).await;
    let attempt = w
        .store()
        .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 100))
        .await
        .unwrap()
        .value
        .attempt;
    assert_eq!(w.invocations().await[0].process, None);
    let record = process_record("here", 4242);
    let set = w
        .store()
        .set_process(set_process(&attempt, "process-1", Some(record.clone())))
        .await
        .unwrap();
    assert_eq!(
        (set.disposition, &set.value.process),
        (Disposition::Applied, &Some(record.clone()))
    );
    let shown = &w.invocations().await[0];
    assert_eq!(
        (&shown.process, shown.lease_until_ms),
        (&Some(record.clone()), Some(1_100))
    );

    let cleared = w
        .store()
        .set_process(set_process(&attempt, "process-2", None))
        .await
        .unwrap();
    assert_eq!(cleared.value.process, None);
    assert_eq!(w.invocations().await[0].process, None);
    let replay = w
        .store()
        .set_process(set_process(&attempt, "process-1", Some(record.clone())))
        .await
        .unwrap();
    assert_eq!(
        (replay.disposition, &replay.value.process),
        (Disposition::Duplicate, &Some(record.clone()))
    );
    assert_eq!(w.invocations().await[0].process, None, "a replay wrote");
    w.store()
        .set_process(set_process(&attempt, "process-3", Some(record.clone())))
        .await
        .unwrap();

    let other = process_record("other", 7);
    let mut wrong_owner = attempt.clone();
    wrong_owner.owner = "w2".to_string();
    let mut wrong_fence = attempt.clone();
    wrong_fence.fence += 1;
    for (n, stranger) in [wrong_owner, wrong_fence].iter().enumerate() {
        let refused = w
            .store()
            .set_process(set_process(
                stranger,
                &format!("stranger-{n}"),
                Some(other.clone()),
            ))
            .await;
        assert_eq!(
            failure(refused),
            (StoreFailureKind::Stale, "lease.lost".into())
        );
    }
    w.clock.set(1_100);
    let at = w
        .store()
        .set_process(set_process(&attempt, "process-at", Some(other.clone())))
        .await;
    assert_eq!(failure(at), (StoreFailureKind::Stale, "lease.lost".into()));
    assert_eq!(w.invocations().await[0].process, Some(record.clone()));
    assert_eq!(
        w.due().await,
        vec![DueItem::Lease {
            attempt: attempt.clone(),
            process: Some(record.clone()),
        }]
    );

    let mut unchecked = expire(&attempt, "expire");
    unchecked.process_unchecked = true;
    unchecked.mutation = mutation("expire_attempt", "expire", json!([attempt, true]));
    let expired = w.store().expire_attempt(unchecked).await.unwrap();
    assert_eq!(expired.disposition, Disposition::Applied);
    let after = w
        .store()
        .set_process(set_process(&attempt, "process-after", None))
        .await;
    assert_eq!(
        failure(after),
        (StoreFailureKind::Stale, "lease.lost".into())
    );
    assert_eq!(
        w.invocations().await[0].process,
        Some(record),
        "an ended attempt keeps its record"
    );
}

/// S17: the run keeps the base folder it was created with; a repeat with another is a
/// duplicate, and a run created without one shows none.
pub async fn s17_base_dir(factory: &dyn StoreFactory) {
    let fixture = factory.fresh(ManualClock::new(1_000));
    let store = fixture.store.as_ref();
    let key = RunKey::new(DEFAULT_TENANT, "s17");
    let mut first = create_run(&key, "create", 0);
    first.base_dir = Some("/repo".into());
    store.create_run(first.clone()).await.unwrap();
    let mut moved = first.clone();
    moved.base_dir = Some("/repo/sub".into());
    let again = store.create_run(moved.clone()).await.unwrap();
    assert_eq!(again.disposition, Disposition::Duplicate);
    moved.mutation = mutation(
        "create_run",
        "create-elsewhere",
        json!([key, "start", first.package_digest, first.started.digest]),
    );
    let elsewhere = store.create_run(moved).await.unwrap();
    assert_eq!(elsewhere.disposition, Disposition::Duplicate);
    let run = store.get_run(&key).await.unwrap().expect("run");
    assert_eq!(run.base_dir.as_deref(), Some("/repo"));
    let listed = store
        .list_runs(DEFAULT_TENANT, None, 10)
        .await
        .unwrap()
        .items;
    assert_eq!(listed[0].base_dir.as_deref(), Some("/repo"));

    let bare = RunKey::new(DEFAULT_TENANT, "s17-none");
    store
        .create_run(create_run(&bare, "create-none", 0))
        .await
        .unwrap();
    let run = store.get_run(&bare).await.unwrap().expect("run");
    assert_eq!(run.base_dir, None);
}

/// S18: a run keeps the artifact it was first started from; a mismatched digest
/// writes nothing.
pub async fn s18_artifact(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s18").await;
    assert_eq!(w.run().await.artifact_digest, None, "started without one");
    let artifact = |bytes: &[u8]| StoredArtifact {
        digest: digest::artifact(bytes),
        bytes: bytes.to_vec(),
    };
    let (first, rebuilt) = (artifact(b"first build"), artifact(b"second build"));

    let key = RunKey::new(DEFAULT_TENANT, "s18-artifact");
    let mut request = create_run(&key, "create-artifact", 0);
    request.artifact = Some(first.clone());
    let created = w.store().create_run(request).await.unwrap();
    assert_eq!(created.disposition, Disposition::Applied);
    let view = || async { w.store().get_run(&key).await.unwrap().unwrap() };
    assert_eq!(view().await.artifact_digest, Some(first.digest.clone()));
    assert_eq!(
        w.store().get_artifact(&first.digest).await.unwrap(),
        Some(first.bytes.clone())
    );

    let mut again = create_run(&key, "create-artifact-again", 0);
    again.artifact = Some(rebuilt.clone());
    let again = w.store().create_run(again).await.unwrap();
    assert_eq!(again.disposition, Disposition::Duplicate);
    assert_eq!(view().await.artifact_digest, Some(first.digest.clone()));
    assert_eq!(w.store().get_artifact(&rebuilt.digest).await.unwrap(), None);

    let bad_key = RunKey::new(DEFAULT_TENANT, "s18-bad");
    let mut bad = create_run(&bad_key, "create-bad", 0);
    bad.artifact = Some(StoredArtifact {
        digest: first.digest.clone(),
        bytes: b"tampered".to_vec(),
    });
    assert_eq!(
        failure(w.store().create_run(bad).await),
        (
            StoreFailureKind::Conflict,
            "artifact.digest-mismatch".into()
        )
    );
    assert_eq!(w.store().get_run(&bad_key).await.unwrap(), None);
    assert_eq!(
        w.store().get_receipt(&bad_key, "create-bad").await.unwrap(),
        None
    );
}

/// S19: a claim held outside the worker keeps its holder handle and a deadline that no
/// renewal passes.
pub async fn s19_holder_and_deadline(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s19").await;
    let invocation = w.scheduled(0).await;
    let large = json!("x".repeat(MAX_HOLDER_BYTES));
    let refused = w
        .store()
        .claim_attempt(claim_held(
            &w.key,
            "claim-large",
            &invocation,
            100,
            Some(large),
            250,
        ))
        .await;
    assert_eq!(
        failure(refused),
        (StoreFailureKind::Quota, "holder.too-large".into())
    );
    assert_eq!(w.invocations().await[0].status, InvocationStatus::Pending);
    assert_eq!(
        w.store().get_receipt(&w.key, "claim-large").await.unwrap(),
        None
    );

    let holder = json!({"pid": 7});
    let claimed = w
        .store()
        .claim_attempt(claim_held(
            &w.key,
            "claim",
            &invocation,
            100,
            Some(holder.clone()),
            250,
        ))
        .await
        .unwrap()
        .value;
    let encoded = canonical::encode(&holder).unwrap();
    assert_eq!(
        (&claimed.holder, claimed.deadline_ms, claimed.lease_until_ms),
        (&Some(encoded.clone()), Some(1_250), 1_100)
    );
    let shown = &w.invocations().await[0];
    assert_eq!(shown.attempt.as_ref(), Some(&claimed.attempt));
    assert_eq!(
        (&shown.holder, shown.deadline_ms, shown.lease_until_ms),
        (&Some(encoded), Some(1_250), Some(1_100))
    );

    w.clock.set(1_050);
    let renewed = w
        .store()
        .heartbeat(heartbeat(&claimed.attempt, "renew-1", 100))
        .await
        .unwrap();
    assert_eq!(renewed.value.lease_until_ms, 1_150);
    w.clock.set(1_140);
    let capped = w
        .store()
        .heartbeat(heartbeat(&claimed.attempt, "renew-2", 200))
        .await
        .unwrap();
    assert_eq!(
        capped.value.lease_until_ms, 1_250,
        "never past the deadline"
    );
    w.clock.set(1_250);
    let at = w
        .store()
        .heartbeat(heartbeat(&claimed.attempt, "renew-3", 100))
        .await;
    assert_eq!(failure(at), (StoreFailureKind::Stale, "lease.lost".into()));
}

/// S20: every reason but `lease` expires a running attempt before its lease is due, and
/// fences its holder out.
pub async fn s20_expire_reasons(factory: &dyn StoreFactory) {
    let lease = World::new(factory, "s20-lease").await;
    let invocation = lease.scheduled(0).await;
    let attempt = lease
        .store()
        .claim_attempt(claim(&lease.key, "claim", &invocation, "w1", 1_000))
        .await
        .unwrap()
        .value
        .attempt;
    let early = lease
        .store()
        .expire_attempt(expire_for(&attempt, "expire", ExpireReason::Lease))
        .await;
    assert_eq!(
        failure(early),
        (StoreFailureKind::Conflict, "lease.not-due".into())
    );

    for reason in [
        ExpireReason::ProbeDead,
        ExpireReason::Abandoned,
        ExpireReason::Cancelled,
    ] {
        let w = World::new(factory, &format!("s20-{}", reason.as_str())).await;
        let invocation = w.scheduled(0).await;
        let attempt = w
            .store()
            .claim_attempt(claim(&w.key, "claim", &invocation, "w1", 1_000))
            .await
            .unwrap()
            .value
            .attempt;
        let expired = w
            .store()
            .expire_attempt(expire_for(&attempt, "expire", reason))
            .await
            .unwrap();
        assert_eq!(expired.disposition, Disposition::Applied, "{reason:?}");
        let events = w.events().await;
        let results: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "activity.result")
            .collect();
        assert_eq!(results.len(), 1, "{reason:?}");
        let result: ActivityResult = canonical::decode(&results[0].body).unwrap();
        assert_eq!(result.status, AttemptStatus::Expired);
        assert_eq!(w.invocations().await[0].status, InvocationStatus::Settled);

        let renewed = w.store().heartbeat(heartbeat(&attempt, "renew", 100)).await;
        assert_eq!(
            failure(renewed),
            (StoreFailureKind::Stale, "lease.lost".into())
        );
        let late = w
            .store()
            .finish_attempt(finish(&attempt, "finish", 1))
            .await;
        assert_eq!(
            failure(late),
            (StoreFailureKind::Stale, "lease.lost".into())
        );
        let kept = w
            .store()
            .record_late_evidence(RecordLateEvidence {
                mutation: mutation("record_late_evidence", "late", json!([attempt])),
                attempt: attempt.clone(),
                evidence: success(&attempt, 1),
            })
            .await
            .unwrap();
        assert_eq!(kept.disposition, Disposition::Applied);
        assert_eq!(
            w.events().await.len(),
            events.len(),
            "late evidence appends nothing"
        );
    }
}

/// S21: an attempt past its deadline is a `Lease` item of `scan_due`, even when its
/// lease alone would run longer.
pub async fn s21_deadline_sweep(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s21").await;
    let invocation = w.scheduled(0).await;
    let claimed = w
        .store()
        .claim_attempt(claim_held(&w.key, "claim", &invocation, 1_000, None, 200))
        .await
        .unwrap()
        .value;
    assert_eq!(
        (claimed.lease_until_ms, claimed.deadline_ms),
        (1_200, Some(1_200))
    );
    w.clock.set(1_199);
    assert_eq!(w.due().await, vec![]);
    w.clock.set(1_200);
    assert_eq!(
        w.due().await,
        vec![DueItem::Lease {
            attempt: claimed.attempt.clone(),
            process: None,
        }]
    );
    let expired = w
        .store()
        .expire_attempt(expire(&claimed.attempt, "expire"))
        .await
        .unwrap();
    assert_eq!(expired.disposition, Disposition::Applied);
}

/// S22: the claim of an invocation scheduled with `params` returns them, and a
/// retry's claim returns them again; one scheduled without returns none.
pub async fn s22_params(factory: &dyn StoreFactory) {
    let w = World::new(factory, "s22").await;
    let params = json!({"tools": ["Read", "Edit"], "schema": {"type": "object"}});
    let (with, invocation) = schedule_with(&w.key, "a", 0, Some(params.clone()));
    let (without, plain) = schedule(&w.key, "b", 0);
    w.commit("commit-1", vec![with, without]).await;
    let expected = Some(canonical::to_vec(&params).expect("params"));

    let first = w
        .store()
        .claim_attempt(claim(&w.key, "claim-1", &invocation, "w", 1_000))
        .await
        .expect("claim")
        .value;
    assert_eq!(first.params, expected);
    let bare = w
        .store()
        .claim_attempt(claim(&w.key, "claim-2", &plain, "w", 1_000))
        .await
        .expect("claim")
        .value;
    assert_eq!(bare.params, None);

    w.store()
        .finish_attempt(finish(&first.attempt, "finish-1", 1))
        .await
        .expect("finish");
    w.commit("commit-2", vec![retry(&w.key, "a", 2, 0)]).await;
    let second = w
        .store()
        .claim_attempt(claim(&w.key, "claim-3", &invocation, "w", 1_000))
        .await
        .expect("claim")
        .value;
    assert_eq!((second.attempt_number, second.params), (2, expected));
}

/// Expands to one `#[tokio::test]` per scenario, run against `$factory`.
#[macro_export]
macro_rules! store_conformance_tests {
    ($factory:expr) => {
        $crate::store_conformance_tests!(@each $factory;
            s01_concurrent_commit s02_replay_after_progress s03_request_identity
            s04_result_after_expiry s05_heartbeat_deadline s06_event_id_conflict
            s07_concurrent_appends s08_atomic_commit s09_not_before s10_fences
            s11_quarantine s12_finish_expire_race s13_create_run_repeat s14_append_limits
            s15_start_key_race s16_process_record s17_base_dir
            s18_artifact s19_holder_and_deadline s20_expire_reasons s21_deadline_sweep
            s22_params);
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
