//! Fixtures shared by the engine tests: a scripted reducer, a store wrapper that
//! injects faults, and readers for the run's history.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use runspore_host::{ActivityOutput, ActivityRegistry, Engine, EngineConfig, NativeRunner};
use runspore_store_conformance::schedule;
use runspore_store_sqlite::SqliteStore;
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::reducer::*;
use runspore_types::store::*;
use serde_json::{json, Value};

type Step = dyn Fn(&TransitionRequest) -> Result<Decision, Failure> + Send + Sync;

/// A reducer that answers with whatever its closure decides.
pub struct Scripted(pub Box<Step>);

impl Reducer for Scripted {
    fn describe(&self) -> Descriptor {
        Descriptor {
            abi_version: "test".into(),
            semantics_version: "test".into(),
            graph_format_version: "test".into(),
            codec_version: "test".into(),
        }
    }
    fn kernel_digest(&self) -> String {
        "scripted".into()
    }
    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        (self.0)(request)
    }
}

/// A decision whose snapshot is `{"status": status, "seq": <event sequence>}`.
pub fn decision(request: &TransitionRequest, status: &str, commands: Vec<Command>) -> Decision {
    let seq = request.input_event.sequence;
    let snapshot = canonical::encode(&json!({"status": status, "seq": seq})).unwrap();
    Decision {
        snapshot_digest: digest::state(&snapshot),
        snapshot,
        commands,
        diagnostics: Vec::new(),
    }
}

fn run_key(request: &TransitionRequest) -> RunKey {
    let started: Value = serde_json::from_slice(&request.input_event.payload).unwrap();
    RunKey::new("default", started["runId"].as_str().unwrap_or_default())
}

/// `run.started` schedules node `a`; anything else completes the run.
pub fn one_step(request: &TransitionRequest) -> Result<Decision, Failure> {
    if request.input_event.kind == "run.started" {
        let (command, _) = schedule(&run_key(request), "a", 0);
        Ok(decision(request, "running", vec![command]))
    } else {
        Ok(decision(request, "completed", Vec::new()))
    }
}

/// A workflow for the scripted reducer: one native action `echo`.
pub const WORKFLOW: &[u8] = br#"{"actions": {"echo": {"kind": "native", "function": "echo"}}}"#;

/// A fresh database path, unique per test binary and `name`.
pub fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runspore-host-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("store.db")
}

/// An engine with a scripted reducer and the built-in runners.
pub fn engine(
    store: Arc<dyn Store>,
    reducer: impl Fn(&TransitionRequest) -> Result<Decision, Failure> + Send + Sync + 'static,
    native: NativeRunner,
    config: EngineConfig,
) -> Engine {
    let reducer = Arc::new(Scripted(Box::new(reducer)));
    Engine::new(
        store,
        reducer,
        ActivityRegistry::with_builtins(native),
        config,
    )
}

/// A native function `echo` that returns its input and counts its calls.
pub fn counting_echo(count: Arc<AtomicUsize>) -> NativeRunner {
    NativeRunner::new().function("echo", move |_ctx, input| {
        count.fetch_add(1, Ordering::SeqCst);
        async move {
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: input,
            }
        }
    })
}

/// Every event of the run, in sequence order.
pub async fn events(engine: &Engine, key: &RunKey) -> Vec<EventRecord> {
    let mut items = Vec::new();
    let mut after = 0;
    loop {
        let page = engine.list_events(key, after, 100).await.unwrap().items;
        let Some(last) = page.last() else {
            return items;
        };
        after = last.event.sequence;
        items.extend(page);
    }
}

/// The payloads of the run's `activity.result` events, in sequence order.
pub async fn results(engine: &Engine, key: &RunKey) -> Vec<Value> {
    events(engine, key)
        .await
        .iter()
        .filter(|e| e.event.kind == "activity.result")
        .map(|e| serde_json::from_slice(&e.event.body).unwrap())
        .collect()
}

/// Rows of the store's late-evidence audit table.
pub fn audits(path: &std::path::Path) -> i64 {
    let db = rusqlite::Connection::open(path).unwrap();
    db.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))
        .unwrap()
}

/// Faults a test injects into a `Hooked` store.
#[async_trait]
pub trait Hook: Send + Sync + 'static {
    /// Called with the receipt of every claim the inner store applied.
    async fn claimed(
        &self,
        _inner: &SqliteStore,
        receipt: Receipt<Claim>,
    ) -> StoreResult<Receipt<Claim>> {
        Ok(receipt)
    }
    /// A failure to answer `get_run` with instead of reading.
    fn get_run(&self) -> Option<StoreFailure> {
        None
    }
}

/// A SQLite store whose answers a `Hook` may replace.
pub struct Hooked<H> {
    pub inner: Arc<SqliteStore>,
    pub hook: H,
}

#[async_trait]
impl<H: Hook> Store for Hooked<H> {
    async fn claim_attempt(&self, r: ClaimAttempt) -> StoreResult<Receipt<Claim>> {
        let receipt = self.inner.claim_attempt(r).await?;
        self.hook.claimed(&self.inner, receipt).await
    }
    async fn get_run(&self, k: &RunKey) -> StoreResult<Option<RunView>> {
        match self.hook.get_run() {
            Some(failure) => Err(failure),
            None => self.inner.get_run(k).await,
        }
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
