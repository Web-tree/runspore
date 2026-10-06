use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::failpoint;
use runspore_types::model::{
    event_kind, ActivityResult, AttemptStatus, InvocationResolved, Resolution, RunStarted,
    RunStatus, SignalReceived,
};
use runspore_types::reducer::{Envelope, Identity, Limits, Reducer, TransitionRequest};
use runspore_types::store::*;
use serde_json::{json, Map, Value};
use tokio::sync::{watch, Notify};
use tokio::task::JoinSet;

use crate::activity::{ActivityContext, ActivityOutput, ActivityRegistry};
use crate::error::HostError;

/// Engine settings. Defaults are those of the host specification.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub tenant: String,
    pub worker_id: String,
    pub lease_ms: u64,
    pub heartbeat_ms: u64,
    pub poll_ms: u64,
    pub shutdown_grace_ms: u64,
    pub max_concurrent_activities: usize,
    pub turns_per_run: u32,
    pub limits: Limits,
    pub base_dir: PathBuf,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            tenant: DEFAULT_TENANT.to_string(),
            worker_id: format!("worker-{}", process_nonce()),
            lease_ms: 10_000,
            heartbeat_ms: 3_000,
            poll_ms: 200,
            shutdown_grace_ms: 10_000,
            max_concurrent_activities: 4,
            turns_per_run: 32,
            limits: Limits::default(),
            base_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

/// Unique per call: engines created in one process within one clock tick differ by
/// the counter.
fn process_nonce() -> String {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{nanos:x}-{n:x}", std::process::id())
}

/// The result of `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartOutcome {
    pub key: RunKey,
    /// False when the run already existed: the call was a client retry.
    pub created: bool,
}

/// What one `tick` did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickReport {
    pub expired: usize,
    /// Commits whose receipt was `applied`.
    pub turns: usize,
    pub quarantined: usize,
    /// Decisions refused because another coordinator committed a different decision
    /// for the same event.
    pub diverged: usize,
    pub dispatched: usize,
}

impl TickReport {
    pub fn did_work(&self) -> bool {
        self.expired + self.turns + self.quarantined + self.diverged + self.dispatched > 0
    }
}

/// Why a claimed attempt cannot start.
enum Unrunnable {
    /// The run or its package could not be read. Nothing is reported: the lease
    /// expires and the kernel decides, as after a crash.
    Abandon,
    /// The workflow cannot be run as stored: a non-retryable failure is reported.
    Fail(ActivityOutput),
}

struct Package {
    bytes: Vec<u8>,
    actions: Map<String, Value>,
}

struct Inner {
    store: Arc<dyn Store>,
    reducer: Arc<dyn Reducer>,
    activities: ActivityRegistry,
    config: EngineConfig,
    nonce: String,
    counter: AtomicU64,
    packages: Mutex<HashMap<String, Arc<Package>>>,
    inflight: Mutex<HashSet<(RunKey, String)>>,
    tasks: Mutex<JoinSet<()>>,
    settled: Notify,
    shutdown: watch::Sender<bool>,
}

/// The engine. Cheap to clone; clones share in-flight attempts.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

const PAGE: u32 = 256;
const TRANSIENT_RETRIES: u32 = 100;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn mutation(operation: &str, request_id: String, body: Value) -> Result<Mutation, HostError> {
    Mutation::new(operation, request_id, &body)
        .map_err(|e| HostError::invalid(e.code(), e.to_string()))
}

fn evidence<T: serde::Serialize>(value: &T) -> Result<Evidence, HostError> {
    Evidence::encode(value).map_err(|e| HostError::invalid(e.code(), e.to_string()))
}

fn is(result: &StoreFailure, kind: StoreFailureKind, code: &str) -> bool {
    result.kind == kind && result.code == code
}

/// Sends the identical request again while the store answers `unavailable`, or
/// `unknown-commit` when `ambiguous` is set. Every request is idempotent by its ID.
async fn retry<T, F, Fut>(ambiguous: bool, mut call: F) -> StoreResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = StoreResult<T>>,
{
    let mut tries = 0;
    loop {
        match call().await {
            Err(f)
                if tries < TRANSIENT_RETRIES
                    && (f.kind == StoreFailureKind::Unavailable
                        || (ambiguous && f.kind == StoreFailureKind::UnknownCommit)) =>
            {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(10 * u64::from(tries.min(20)))).await;
            }
            other => return other,
        }
    }
}

/// Reads the `status` wire string of a canonical `model::State` snapshot.
fn snapshot_status(snapshot: &[u8]) -> Option<&'static str> {
    let state: Value = serde_json::from_slice(snapshot).ok()?;
    let status: RunStatus = serde_json::from_value(state.get("status")?.clone()).ok()?;
    Some(status.as_str())
}

fn parked(view: &RunView) -> bool {
    if view.quarantine.is_some() {
        return true;
    }
    let pending = view.applied_sequence + 1 < view.next_sequence;
    match serde_json::from_value::<RunStatus>(json!(view.status)) {
        Ok(status) if status.is_terminal() => true,
        Ok(RunStatus::Waiting | RunStatus::NeedsIntervention) => !pending,
        _ => false,
    }
}

impl Engine {
    pub fn new(
        store: Arc<dyn Store>,
        reducer: Arc<dyn Reducer>,
        activities: ActivityRegistry,
        config: EngineConfig,
    ) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            inner: Arc::new(Inner {
                store,
                reducer,
                activities,
                config,
                nonce: process_nonce(),
                counter: AtomicU64::new(0),
                packages: Mutex::new(HashMap::new()),
                inflight: Mutex::new(HashSet::new()),
                tasks: Mutex::new(JoinSet::new()),
                settled: Notify::new(),
                shutdown,
            }),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.inner.config
    }

    fn key(&self, run: &str) -> RunKey {
        RunKey::new(&self.inner.config.tenant, run)
    }

    /// Validates and starts a workflow. Retrying with the same `start_key` is a duplicate.
    pub async fn start(
        &self,
        workflow_json: &[u8],
        input: Value,
        start_key: &str,
    ) -> Result<StartOutcome, HostError> {
        let inner = &self.inner;
        let workflow = canonical::parse(workflow_json)
            .map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
        let package = canonical::to_vec(&workflow)
            .map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
        let actions = workflow.get("actions").and_then(Value::as_object);
        for (action_id, action) in actions.into_iter().flatten() {
            let kind = action.get("kind").and_then(Value::as_str).ok_or_else(|| {
                HostError::invalid("action.kind-missing", format!("action `{action_id}`"))
            })?;
            let runner = inner.activities.get(kind).ok_or_else(|| {
                HostError::invalid(
                    "action.kind-unregistered",
                    format!("action `{action_id}` has kind `{kind}` with no runner"),
                )
            })?;
            runner
                .validate(action_id, action)
                .map_err(|e| HostError::invalid("action.invalid", e))?;
        }
        let tenant = &inner.config.tenant;
        let key = self.key(&ids::run_from_start_key(tenant, start_key));
        let started = evidence(&RunStarted {
            tenant: tenant.clone(),
            run_id: key.run.clone(),
            input,
        })?;
        let package_digest = digest::package(&package);
        inner.reducer.transition(&TransitionRequest {
            identity: inner.identity(&package_digest),
            graph: package.clone(),
            snapshot: None,
            input_event: Envelope {
                event_id: "start".to_string(),
                sequence: 1,
                accepted_at_ms: 0,
                kind: event_kind::RUN_STARTED.to_string(),
                payload: started.body.clone(),
            },
            frozen_limits: inner.config.limits,
        })?;
        let request = CreateRun {
            mutation: mutation(
                "create_run",
                format!("start/{start_key}"),
                json!([key, start_key, package_digest, started.digest]),
            )?,
            key: key.clone(),
            start_key: start_key.to_string(),
            package_digest,
            package,
            started,
        };
        let receipt = retry(true, || inner.store.create_run(request.clone())).await?;
        Ok(StartOutcome {
            key: receipt.value.key,
            created: receipt.disposition == Disposition::Applied,
        })
    }

    async fn append(
        &self,
        key: &RunKey,
        request_id: String,
        event_id: String,
        kind: &str,
        body: Evidence,
    ) -> Result<Receipt<EventAccepted>, HostError> {
        let request = AppendEvent {
            mutation: mutation(
                "append_event",
                request_id,
                json!([key, event_id, kind, body.digest]),
            )?,
            key: key.clone(),
            event_id,
            kind: kind.to_string(),
            body,
        };
        Ok(retry(true, || self.inner.store.append_event(request.clone())).await?)
    }

    /// Delivers a signal. Retrying with the same `message_id` is a duplicate.
    pub async fn signal(
        &self,
        key: &RunKey,
        message_id: &str,
        name: &str,
        outcome: Option<String>,
        data: Value,
    ) -> Result<Receipt<EventAccepted>, HostError> {
        let body = evidence(&SignalReceived {
            name: name.to_string(),
            outcome,
            data,
        })?;
        let event_id = ids::event_for_signal(message_id);
        let request_id = format!("signal/{message_id}");
        self.append(key, request_id, event_id, event_kind::SIGNAL_RECEIVED, body)
            .await
    }

    /// Resolves an invocation in `needs-intervention`. Retrying with the same
    /// `request_id` is a duplicate.
    pub async fn resolve(
        &self,
        key: &RunKey,
        request_id: &str,
        invocation_id: &str,
        resolution: Resolution,
    ) -> Result<Receipt<EventAccepted>, HostError> {
        let body = evidence(&InvocationResolved {
            invocation_id: invocation_id.to_string(),
            resolution,
        })?;
        let event_id = ids::event_for_resolution(request_id);
        let request_id = format!("resolve/{request_id}");
        self.append(
            key,
            request_id,
            event_id,
            event_kind::INVOCATION_RESOLVED,
            body,
        )
        .await
    }

    /// Lifts a quarantine. The request ID names the quarantine being lifted, so a
    /// retry is a duplicate and a later quarantine needs a new release.
    pub async fn release(&self, key: &RunKey) -> Result<Receipt<QuarantineChanged>, HostError> {
        let view = self.get_run(key).await?.ok_or_else(|| {
            HostError::Store(StoreFailure::new(
                StoreFailureKind::NotFound,
                "run.not-found",
                format!("run {} does not exist", key.run),
            ))
        })?;
        let at = view
            .quarantine
            .map(|q| q.at_ms.to_string())
            .unwrap_or_default();
        let request = ReleaseRun {
            mutation: mutation(
                "release_run",
                format!("release/{}/{at}", view.revision),
                json!([key]),
            )?,
            key: key.clone(),
        };
        Ok(retry(true, || self.inner.store.release_run(request.clone())).await?)
    }

    pub async fn get_run(&self, key: &RunKey) -> Result<Option<RunView>, HostError> {
        Ok(self.inner.store.get_run(key).await?)
    }

    pub async fn list_runs(
        &self,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<Page<RunView>, HostError> {
        let tenant = &self.inner.config.tenant;
        Ok(self.inner.store.list_runs(tenant, cursor, limit).await?)
    }

    pub async fn list_events(
        &self,
        key: &RunKey,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Page<EventRecord>, HostError> {
        Ok(self
            .inner
            .store
            .list_events(key, after_sequence, limit)
            .await?)
    }

    pub async fn list_invocations(&self, key: &RunKey) -> Result<Vec<InvocationView>, HostError> {
        Ok(self.inner.store.list_invocations(key).await?)
    }

    /// Number of attempts this engine is running.
    pub fn in_flight(&self) -> usize {
        lock(&self.inner.inflight).len()
    }

    /// One bounded pass: sweep expired leases, coordinate ready runs, dispatch due
    /// invocations. Never waits for an activity.
    pub async fn tick(&self) -> Result<TickReport, HostError> {
        let inner = &self.inner;
        let mut report = TickReport::default();
        inner.reap();
        for item in inner.scan_due().await? {
            if let DueItem::Lease { attempt } = item {
                if inner.expire(attempt).await? {
                    report.expired += 1;
                }
            }
        }
        let mut cursor = None;
        loop {
            let page = retry(false, || inner.store.scan_ready(cursor.clone(), PAGE)).await?;
            for key in &page.items {
                inner.coordinate(key, &mut report).await?;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        if *inner.shutdown.borrow() {
            return Ok(report);
        }
        for item in inner.scan_due().await? {
            if lock(&inner.inflight).len() >= inner.config.max_concurrent_activities {
                break;
            }
            if let DueItem::Outbox { key, invocation_id } = item {
                if Inner::dispatch(inner, key, invocation_id).await? {
                    report.dispatched += 1;
                }
            }
        }
        Ok(report)
    }

    /// Ticks until `shutdown` resolves, sleeping `poll_ms` after a tick that did
    /// nothing. Then stops claiming, waits up to `shutdown_grace_ms` for in-flight
    /// attempts, signals the rest to stop, waits up to `shutdown_grace_ms` again for
    /// their runners to return, and returns without finishing them: their leases expire.
    pub async fn serve(&self, shutdown: impl Future<Output = ()>) -> Result<(), HostError> {
        let inner = &self.inner;
        tokio::pin!(shutdown);
        loop {
            let idle = match self.tick().await {
                Ok(report) => !report.did_work(),
                Err(HostError::Store(f)) if f.kind == StoreFailureKind::Unavailable => true,
                Err(e) => return Err(e),
            };
            let pause = if idle { inner.config.poll_ms } else { 0 };
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                () = tokio::time::sleep(Duration::from_millis(pause)) => {}
                () = inner.settled.notified(), if idle => {}
            }
        }
        let grace = Duration::from_millis(inner.config.shutdown_grace_ms);
        inner.drain(grace).await;
        inner.shutdown.send_replace(true);
        inner.drain(grace).await;
        let mut tasks = std::mem::take(&mut *lock(&inner.tasks));
        tasks.abort_all();
        Ok(())
    }

    /// Ticks until the run is terminal, `waiting`, `needs-intervention` or quarantined,
    /// with no pending event and nothing of it in flight.
    pub async fn run_until_parked(&self, key: &RunKey) -> Result<RunView, HostError> {
        let inner = &self.inner;
        loop {
            let report = self.tick().await?;
            let busy = lock(&inner.inflight).iter().any(|(k, _)| k == key);
            let view = self.get_run(key).await?.ok_or_else(|| {
                HostError::invalid("run.not-found", format!("run {} does not exist", key.run))
            })?;
            if !busy && parked(&view) {
                return Ok(view);
            }
            if !report.did_work() {
                let poll = Duration::from_millis(inner.config.poll_ms);
                let _ = tokio::time::timeout(poll, inner.settled.notified()).await;
            }
        }
    }
}

impl Inner {
    fn unique(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}/{}/{n}", self.nonce)
    }

    fn identity(&self, package_digest: &str) -> Identity {
        let descriptor = self.reducer.describe();
        Identity {
            package_digest: package_digest.to_string(),
            kernel_digest: self.reducer.kernel_digest(),
            semantics_version: descriptor.semantics_version,
            codec_version: descriptor.codec_version,
        }
    }

    fn reap(&self) {
        let mut tasks = lock(&self.tasks);
        while tasks.try_join_next().is_some() {}
    }

    async fn drain(&self, grace: Duration) {
        let deadline = tokio::time::Instant::now() + grace;
        while !lock(&self.inflight).is_empty() {
            if tokio::time::timeout_at(deadline, self.settled.notified())
                .await
                .is_err()
            {
                return;
            }
        }
    }

    async fn scan_due(&self) -> Result<Vec<DueItem>, HostError> {
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let page = retry(false, || self.store.scan_due(cursor.clone(), PAGE)).await?;
            items.extend(page.items);
            cursor = page.next;
            if cursor.is_none() {
                return Ok(items);
            }
        }
    }

    async fn package(&self, digest: &str) -> Result<Arc<Package>, HostError> {
        if let Some(package) = lock(&self.packages).get(digest) {
            return Ok(package.clone());
        }
        let bytes = retry(false, || self.store.get_package(digest))
            .await?
            .ok_or_else(|| {
                HostError::Store(StoreFailure::new(
                    StoreFailureKind::Corrupt,
                    "package.missing",
                    format!("package {digest} is not stored"),
                ))
            })?;
        let workflow: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let actions = workflow
            .get("actions")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let package = Arc::new(Package { bytes, actions });
        lock(&self.packages).insert(digest.to_string(), package.clone());
        Ok(package)
    }

    async fn expire(&self, attempt: AttemptRef) -> Result<bool, HostError> {
        let request = ExpireAttempt {
            mutation: mutation(
                "expire_attempt",
                format!("expire/{}", attempt.attempt_id),
                json!([attempt]),
            )?,
            attempt,
        };
        match retry(true, || self.store.expire_attempt(request.clone())).await {
            Ok(_) => Ok(true),
            Err(f)
                if is(&f, StoreFailureKind::Stale, "lease.lost")
                    || is(&f, StoreFailureKind::Conflict, "lease.not-due") =>
            {
                Ok(false)
            }
            Err(f) => Err(f.into()),
        }
    }

    async fn quarantine(&self, turn: &Turn, code: &str, details: &str) -> Result<(), HostError> {
        let request = QuarantineRun {
            mutation: mutation(
                "quarantine_run",
                self.unique(&format!("quarantine/{}", turn.revision)),
                json!([turn.key, turn.revision, code, details]),
            )?,
            key: turn.key.clone(),
            expected_revision: turn.revision,
            code: code.to_string(),
            details: details.to_string(),
        };
        match retry(true, || self.store.quarantine_run(request.clone())).await {
            Ok(_) => Ok(()),
            Err(f) if f.kind == StoreFailureKind::Stale => Ok(()),
            Err(f) => Err(f.into()),
        }
    }

    /// Runs up to `turns_per_run` transitions of one run.
    async fn coordinate(&self, key: &RunKey, report: &mut TickReport) -> Result<(), HostError> {
        for _ in 0..self.config.turns_per_run {
            let Some(turn) = retry(false, || self.store.load_turn(key)).await? else {
                return Ok(());
            };
            let package = self.package(&turn.package_digest).await?;
            let event = &turn.next_event;
            let request = TransitionRequest {
                identity: self.identity(&turn.package_digest),
                graph: package.bytes.clone(),
                snapshot: turn.snapshot.clone(),
                input_event: Envelope {
                    event_id: event.id.clone(),
                    sequence: event.sequence,
                    accepted_at_ms: event.accepted_at_ms,
                    kind: event.kind.clone(),
                    payload: event.body.clone(),
                },
                frozen_limits: self.config.limits,
            };
            let decision = match self.reducer.transition(&request) {
                Ok(decision) => decision,
                Err(failure) => {
                    self.quarantine(&turn, &failure.code, &failure.details)
                        .await?;
                    report.quarantined += 1;
                    return Ok(());
                }
            };
            let Some(status) = snapshot_status(&decision.snapshot) else {
                let details = "the decision's snapshot has no valid status";
                self.quarantine(&turn, "host.snapshot-invalid", details)
                    .await?;
                report.quarantined += 1;
                return Ok(());
            };
            failpoint::hit("host.turn.after-reduce");
            let decision_digest = digest::decision(&decision);
            let request = CommitTurn {
                mutation: mutation(
                    "commit_turn",
                    ids::commit_request(turn.revision + 1, &event.id),
                    json!([key, turn.revision, event.id, decision_digest]),
                )?,
                key: key.clone(),
                expected_revision: turn.revision,
                event_id: event.id.clone(),
                event_sequence: event.sequence,
                decision_digest,
                snapshot: decision.snapshot,
                snapshot_digest: decision.snapshot_digest,
                status: status.to_string(),
                commands: decision.commands,
                diagnostics: decision.diagnostics,
            };
            match retry(true, || self.store.commit_turn(request.clone())).await {
                Ok(receipt) if receipt.disposition == Disposition::Applied => report.turns += 1,
                Ok(_) => {}
                Err(f) if f.kind == StoreFailureKind::Stale => return Ok(()),
                Err(f) if is(&f, StoreFailureKind::Conflict, "request.digest-mismatch") => {
                    report.diverged += 1;
                    return Ok(());
                }
                Err(f) => return Err(f.into()),
            }
            failpoint::hit("host.turn.after-commit");
        }
        Ok(())
    }

    async fn heartbeat(&self, attempt: &AttemptRef) -> StoreResult<Receipt<LeaseExtended>> {
        let request = Heartbeat {
            mutation: mutation(
                "heartbeat",
                self.unique(&format!("heartbeat/{}", attempt.attempt_id)),
                json!([attempt, self.config.lease_ms]),
            )
            .map_err(|e| StoreFailure::new(StoreFailureKind::Conflict, e.code(), e.to_string()))?,
            attempt: attempt.clone(),
            extend_ms: self.config.lease_ms,
        };
        retry(true, || self.store.heartbeat(request.clone())).await
    }

    /// Claims one due invocation and starts its attempt in the background.
    async fn dispatch(
        this: &Arc<Self>,
        key: RunKey,
        invocation_id: String,
    ) -> Result<bool, HostError> {
        let slot = (key.clone(), invocation_id.clone());
        if lock(&this.inflight).contains(&slot) {
            return Ok(false);
        }
        let request = ClaimAttempt {
            mutation: mutation(
                "claim_attempt",
                this.unique(&format!("claim/{invocation_id}")),
                json!([
                    key,
                    invocation_id,
                    this.config.worker_id,
                    this.config.lease_ms
                ]),
            )?,
            key,
            invocation_id,
            worker: this.config.worker_id.clone(),
            lease_duration_ms: this.config.lease_ms,
        };
        let receipt = match retry(true, || this.store.claim_attempt(request.clone())).await {
            Ok(receipt) => receipt,
            Err(f) if matches!(f.kind, StoreFailureKind::Conflict | StoreFailureKind::Stale) => {
                return Ok(false)
            }
            Err(f) => return Err(f.into()),
        };
        let claim = receipt.value;
        if receipt.disposition == Disposition::Duplicate {
            match this.heartbeat(&claim.attempt).await {
                Ok(hb) if hb.disposition == Disposition::Applied => {}
                _ => return Ok(false),
            }
        }
        lock(&this.inflight).insert(slot.clone());
        let task = this.clone();
        lock(&this.tasks).spawn(async move {
            task.attempt(claim).await;
            lock(&task.inflight).remove(&slot);
            task.settled.notify_waiters();
        });
        Ok(true)
    }

    /// Runs one claimed attempt to its reported result.
    async fn attempt(self: &Arc<Self>, claim: Claim) {
        failpoint::hit("host.attempt.after-claim");
        let attempt = claim.attempt.clone();
        let (stop, stop_rx) = watch::channel(false);
        let ctx = ActivityContext::new(
            attempt.key.clone(),
            claim.node_id.clone(),
            claim.action_id.clone(),
            attempt.invocation_id.clone(),
            claim.attempt_number,
            claim.effect_key.clone(),
            self.config.base_dir.clone(),
            stop_rx,
        );
        let mut lost = false;
        let mut abandoned = false;
        let output = match self.resolve_action(&attempt.key, &claim).await {
            Err(Unrunnable::Abandon) => return,
            Err(Unrunnable::Fail(output)) => output,
            Ok((action, runner, input)) => {
                let run = runner.run(&ctx, &action, &input);
                tokio::pin!(run);
                let period = Duration::from_millis(self.config.heartbeat_ms.max(1));
                let mut beat =
                    tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                let mut shutdown = self.shutdown.subscribe();
                loop {
                    tokio::select! {
                        output = &mut run => break output,
                        _ = beat.tick(), if !lost && !abandoned => {
                            if let Err(f) = self.heartbeat(&attempt).await {
                                if f.kind == StoreFailureKind::Stale {
                                    lost = true;
                                    stop.send_replace(true);
                                }
                            }
                        }
                        () = async { let _ = shutdown.wait_for(|s| *s).await; }, if !abandoned => {
                            abandoned = true;
                            stop.send_replace(true);
                        }
                    }
                }
            }
        };
        failpoint::hit("host.attempt.after-effect");
        let (status, outcome, output, error, retryable) = match output {
            ActivityOutput::Success { outcome, output } => {
                (AttemptStatus::Success, Some(outcome), output, None, true)
            }
            ActivityOutput::Failure { error, retryable } => (
                AttemptStatus::Failure,
                None,
                Value::Null,
                Some(error),
                retryable,
            ),
            ActivityOutput::Unknown { error } => {
                (AttemptStatus::Unknown, None, Value::Null, Some(error), true)
            }
        };
        let result = ActivityResult {
            invocation_id: attempt.invocation_id.clone(),
            attempt_id: attempt.attempt_id.clone(),
            attempt: claim.attempt_number,
            status,
            outcome,
            output,
            error,
            retryable,
        };
        let Ok(evidence) = evidence(&result) else {
            return;
        };
        if abandoned && !lost {
            return;
        }
        if !lost {
            let Ok(mutation) = mutation(
                "finish_attempt",
                format!("finish/{}", attempt.attempt_id),
                json!([attempt, evidence.digest]),
            ) else {
                return;
            };
            let request = FinishAttempt {
                mutation,
                attempt: attempt.clone(),
                result: evidence.clone(),
            };
            let finished = retry(true, || self.store.finish_attempt(request.clone())).await;
            failpoint::hit("host.attempt.after-finish");
            match finished {
                Err(f) if is(&f, StoreFailureKind::Stale, "lease.lost") => {}
                _ => return,
            }
        }
        let Ok(mutation) = mutation(
            "record_late_evidence",
            format!("late/{}/{}", attempt.attempt_id, self.nonce),
            json!([attempt, evidence.digest]),
        ) else {
            return;
        };
        let request = RecordLateEvidence {
            mutation,
            attempt,
            evidence,
        };
        let _ = retry(true, || self.store.record_late_evidence(request.clone())).await;
    }

    /// Finds the action, its runner and the input of a claimed attempt.
    #[allow(clippy::type_complexity)]
    async fn resolve_action(
        &self,
        key: &RunKey,
        claim: &Claim,
    ) -> Result<(Value, Arc<dyn crate::ActivityRunner>, Value), Unrunnable> {
        let fail = |code: &str, message: String| {
            Unrunnable::Fail(ActivityOutput::Failure {
                error: ActivityOutput::error(code, message, Value::Null),
                retryable: false,
            })
        };
        let Ok(Some(view)) = retry(false, || self.store.get_run(key)).await else {
            return Err(Unrunnable::Abandon);
        };
        let package = self
            .package(&view.package_digest)
            .await
            .map_err(|_| Unrunnable::Abandon)?;
        let action = package
            .actions
            .get(&claim.action_id)
            .cloned()
            .ok_or_else(|| fail("host.action-missing", claim.action_id.clone()))?;
        let kind = action
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let runner = self
            .activities
            .get(kind)
            .cloned()
            .ok_or_else(|| fail("host.runner-missing", kind.to_string()))?;
        let input = canonical::parse(&claim.input)
            .map_err(|e| fail("host.input-invalid", e.to_string()))?;
        Ok((action, runner, input))
    }
}
