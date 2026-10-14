use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::File;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::fcntl::Flock;
use runspore_types::artifact::{Artifact, Manifest};
use runspore_types::canonical;
use runspore_types::digest::{self, ids};
use runspore_types::failpoint;
use runspore_types::model::{
    event_kind, ActivityResult, AttemptStatus, InvocationResolved, Resolution, RunStarted,
    RunStatus, SignalReceived, State, Workflow, WorkflowLimits, OUTCOME_RECEIVED,
};
use runspore_types::reducer::{Envelope, Identity, Limits, Reducer, TransitionRequest};
use runspore_types::store::*;
use runspore_wasmtime::ArtifactReader;
use serde_json::{json, Map, Value};
use tokio::sync::{watch, Notify};
use tokio::task::JoinSet;

use crate::activity::{ActivityContext, ActivityOutput, ActivityRegistry, DEFAULT_STOP_GRACE_MS};
use crate::arrival::{Arrival, Trail};
use crate::error::HostError;
use crate::external;
use crate::files::{self, FilesDir, StepFiles, Unmade};
use crate::guard;
use crate::process::{self, Leftover};

mod intervention;
mod steps;

pub use steps::{ExternalStep, StepClaim, StepRenewed, StepReport, StepState, RESULT_CHANNEL};

/// Quarantine code of a run whose stored artifact cannot be read.
pub const ARTIFACT_UNREADABLE: &str = "artifact.unreadable";
/// Refusal and quarantine code of an artifact with files on an engine without `files_dir`.
pub const FILES_DIR_MISSING: &str = "host.files-dir-missing";
/// Quarantine code of a run whose step files or work folder cannot be written.
pub const FILES_UNWRITABLE: &str = "host.files-unwritable";
/// How often the sweep looks for step files no unfinished run needs.
const FILES_SWEEP_EVERY: Duration = Duration::from_secs(30);

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
    /// Recorded as the base folder of every run this engine starts. Dispatch uses the
    /// run's recorded folder; this one serves only runs that have none.
    pub base_dir: PathBuf,
    /// The database's absolute path, given to every program a step starts as
    /// `RUNSPORE_DB`.
    pub db_path: Option<PathBuf>,
    pub run_ids: RunIds,
    /// SIGTERM to SIGKILL when a command's process group is stopped.
    pub stop_grace_ms: u64,
    /// The executable whose guard mode runs in front of every command; by default this
    /// process's own. Its `main` must call `guard::main_if_requested` first.
    pub guard: PathBuf,
    /// Where step files and work folders go. Required for a workflow artifact that
    /// declares files; without it, steps get neither.
    pub files_dir: Option<PathBuf>,
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
            db_path: None,
            run_ids: RunIds::random(),
            stop_grace_ms: DEFAULT_STOP_GRACE_MS,
            guard: guard::own_executable(),
            files_dir: None,
        }
    }
}

type NextRunId = dyn Fn() -> Result<String, HostError> + Send + Sync;

/// Where the ID of each new run comes from. Run IDs are never derived from a start
/// key, so two stores never share a run ID or anything built from it.
#[derive(Clone)]
pub struct RunIds(Arc<NextRunId>);

impl RunIds {
    /// `run_<32 lowercase hex>`: 128 bits from the operating system's random source.
    pub fn random() -> Self {
        Self(Arc::new(|| {
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).map_err(|e| {
                HostError::invalid("run.id-unavailable", format!("random source: {e}"))
            })?;
            Ok(bytes.iter().fold(String::from("run_"), |mut id, b| {
                id.push_str(&format!("{b:02x}"));
                id
            }))
        }))
    }

    /// Hands out `ids` in order, for tests. Once they run out, starting a new run
    /// fails with `run.ids-exhausted`.
    pub fn fixed<I>(ids: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let ids = Mutex::new(ids.into_iter().map(Into::into).collect::<VecDeque<_>>());
        Self(Arc::new(move || {
            ids.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .ok_or_else(|| HostError::invalid("run.ids-exhausted", "no run IDs left"))
        }))
    }

    fn next(&self) -> Result<String, HostError> {
        (self.0)()
    }
}

impl fmt::Debug for RunIds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunIds")
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

/// What a claimed attempt runs, and where.
struct Runnable {
    action: Value,
    runner: Arc<dyn crate::ActivityRunner>,
    input: Value,
    /// The node's params, `None` when it has none.
    params: Option<Value>,
    visit: u32,
    base_dir: PathBuf,
    package: Arc<Package>,
}

/// What `dispatch` did with a due invocation.
enum Dispatched {
    Started,
    Quarantined,
    Not,
}

/// What a due invocation is given before it is claimed: its step files, its work
/// folder, and the artifact's lock, held shared until the try ends.
#[derive(Default)]
struct Delivery {
    files: Option<StepFiles>,
    work_dir: Option<PathBuf>,
    _lock: Option<Flock<File>>,
}

/// Why a due invocation is not claimed.
enum Undeliverable {
    /// The run was quarantined instead.
    Quarantined,
    /// Nothing to do now: the run or its invocation is gone.
    Skip,
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
    nodes: Map<String, Value>,
    /// `limits.maxVisitsPerNode`: no activation has a higher visit.
    max_visits: u32,
    /// The workflow's `onIntervention`, if it has one.
    on_intervention: Option<Value>,
}

impl Package {
    /// The schema node `node_id` declares for `outcome`, if any.
    fn schema(&self, node_id: &str, outcome: &str) -> Option<&Value> {
        self.nodes.get(node_id)?.get("outputSchemas")?.get(outcome)
    }

    /// The schema the await-signal nodes waiting for `signal` declare for `outcome`.
    /// W13 makes them equal, so the first one found is the answer.
    fn signal_schema(&self, signal: &str, outcome: &str) -> Option<&Value> {
        self.nodes
            .values()
            .filter(|node| {
                node.get("kind").and_then(Value::as_str) == Some("await-signal")
                    && node.get("signal").and_then(Value::as_str) == Some(signal)
            })
            .find_map(|node| node.get("outputSchemas")?.get(outcome))
    }
}

/// A runner's output, with a success whose output does not fit its outcome's schema
/// turned into a non-retryable `output.invalid` failure (host §2, One attempt).
fn checked(output: ActivityOutput, schema: impl FnOnce(&str) -> Option<Value>) -> ActivityOutput {
    let ActivityOutput::Success { outcome, output } = output else {
        return output;
    };
    match schema(&outcome).map(|schema| runspore_schema::fit(&outcome, &schema, &output)) {
        Some(Err(misfit)) => ActivityOutput::Failure {
            error: ActivityOutput::error(
                runspore_schema::OUTPUT_INVALID,
                misfit.message(),
                misfit.details(),
            ),
            retryable: false,
        },
        _ => ActivityOutput::Success { outcome, output },
    }
}

struct Inner {
    store: Arc<dyn Store>,
    reducer: Arc<dyn Reducer>,
    activities: ActivityRegistry,
    config: EngineConfig,
    nonce: String,
    counter: AtomicU64,
    packages: Mutex<HashMap<String, Arc<Package>>>,
    /// Manifests of stored artifacts, by artifact digest.
    manifests: Mutex<HashMap<String, Arc<Manifest>>>,
    files: Option<FilesDir>,
    /// When the sweep last looked at the step files.
    files_swept: Mutex<Option<Instant>>,
    inflight: Mutex<HashSet<(RunKey, String)>>,
    /// Invocations of `external` actions, which dispatch leaves to actors.
    external: Mutex<HashSet<(RunKey, String)>>,
    tasks: Mutex<JoinSet<()>>,
    settled: Notify,
    /// Set by `shutdown`: no attempt is claimed from then on.
    closing: AtomicBool,
    /// Fires every in-flight attempt's stop signal.
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

/// The visit `v` of the claimed node, at most `max_visits`, whose activation
/// `<node>/<v>` derives the claimed invocation ID.
fn invocation_visit(key: &RunKey, claim: &Claim, max_visits: u32) -> Option<u32> {
    (1..=max_visits).find(|visit| {
        let activation = ids::activation(&claim.node_id, *visit);
        ids::invocation(&key.tenant, &key.run, &activation) == claim.attempt.invocation_id
    })
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

fn identity(reducer: &dyn Reducer, package_digest: &str) -> Identity {
    let descriptor = reducer.describe();
    Identity {
        package_digest: package_digest.to_string(),
        kernel_digest: reducer.kernel_digest(),
        semantics_version: descriptor.semantics_version,
        codec_version: descriptor.codec_version,
    }
}

/// The identity of a turn on `snapshot`: the semantics the run recorded, so a newer
/// kernel whose default moved on still continues it (spec/host.md §2). A run's first
/// turn, with no snapshot, takes the kernel's default.
fn turn_identity(reducer: &dyn Reducer, package_digest: &str, snapshot: Option<&[u8]>) -> Identity {
    let recorded = snapshot
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|state| state.get("semantics")?.as_str().map(str::to_string));
    let identity = identity(reducer, package_digest);
    match recorded {
        Some(semantics_version) => Identity {
            semantics_version,
            ..identity
        },
        None => identity,
    }
}

/// Checks a workflow without a store: canonicalizes it, validates every action with
/// the runner its `kind` names, and runs a dry `run.started` transition with a null
/// input. Returns the canonical package bytes. `start` performs the same checks.
pub fn validate_workflow(
    reducer: &dyn Reducer,
    activities: &ActivityRegistry,
    limits: Limits,
    workflow_json: &[u8],
) -> Result<Vec<u8>, HostError> {
    let started = evidence(&RunStarted {
        tenant: DEFAULT_TENANT.to_string(),
        run_id: "validate".to_string(),
        input: Value::Null,
    })?;
    Ok(check_workflow(reducer, activities, limits, workflow_json, &started)?.package)
}

/// A workflow that passed the checks of `validate_workflow`.
struct Checked {
    /// The canonical package bytes.
    package: Vec<u8>,
    /// The package as JSON, for the checks `start` adds.
    workflow: Value,
}

/// The checks of `validate_workflow`, with the dry transition fed `started`.
fn check_workflow(
    reducer: &dyn Reducer,
    activities: &ActivityRegistry,
    limits: Limits,
    workflow_json: &[u8],
    started: &Evidence,
) -> Result<Checked, HostError> {
    let workflow =
        canonical::parse(workflow_json).map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
    let package =
        canonical::to_vec(&workflow).map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
    let actions = workflow.get("actions").and_then(Value::as_object);
    for (action_id, action) in actions.into_iter().flatten() {
        let kind = action.get("kind").and_then(Value::as_str).ok_or_else(|| {
            HostError::invalid("action.kind-missing", format!("action `{action_id}`"))
        })?;
        // An external action has no runner: an actor outside the worker does it.
        if kind == external::KIND {
            external::parse(action).map_err(|e| {
                HostError::invalid("action.invalid", format!("action `{action_id}`: {e}"))
            })?;
            continue;
        }
        let runner = activities.get(kind).ok_or_else(|| {
            HostError::invalid(
                "action.kind-unregistered",
                format!("action `{action_id}` has kind `{kind}` with no runner"),
            )
        })?;
        runner
            .validate(action_id, action)
            .map_err(|e| HostError::invalid("action.invalid", e))?;
    }
    reducer.transition(&TransitionRequest {
        identity: identity(reducer, &digest::package(&package)),
        graph: package.clone(),
        snapshot: None,
        input_event: Envelope {
            event_id: ids::EVENT_STARTED.to_string(),
            sequence: 1,
            accepted_at_ms: 0,
            kind: event_kind::RUN_STARTED.to_string(),
            payload: started.body.clone(),
        },
        frozen_limits: limits,
    })?;
    check_schemas(&workflow)?;
    intervention::check(&workflow)?;
    Ok(Checked { package, workflow })
}

/// Every schema in a node's `outputSchemas` against the subset (spec/schema.md §1),
/// and the package's `inputSchema` against the subset and for its object root (§4).
/// The kernel has already checked the fields' shapes (W13; an object).
fn check_schemas(workflow: &Value) -> Result<(), HostError> {
    if let Some(schema) = workflow.get("inputSchema") {
        runspore_schema::check_input(schema).map_err(|e| {
            HostError::invalid(runspore_schema::INVALID_SCHEMA, format!("inputSchema: {e}"))
        })?;
    }
    let nodes = workflow.get("nodes").and_then(Value::as_object);
    for (id, node) in nodes.into_iter().flatten() {
        let schemas = node.get("outputSchemas").and_then(Value::as_object);
        for (outcome, schema) in schemas.into_iter().flatten() {
            runspore_schema::check(schema, runspore_schema::ROOT_LEVEL).map_err(|e| {
                HostError::invalid(
                    runspore_schema::INVALID_SCHEMA,
                    format!("nodes.{id}.outputSchemas.{outcome}: {e}"),
                )
            })?;
        }
    }
    Ok(())
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
                files: config.files_dir.as_deref().map(FilesDir::new),
                config,
                nonce: process_nonce(),
                counter: AtomicU64::new(0),
                packages: Mutex::new(HashMap::new()),
                manifests: Mutex::new(HashMap::new()),
                files_swept: Mutex::new(None),
                inflight: Mutex::new(HashSet::new()),
                external: Mutex::new(HashSet::new()),
                tasks: Mutex::new(JoinSet::new()),
                settled: Notify::new(),
                closing: AtomicBool::new(false),
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

    /// Validates and starts a workflow. Retrying with the same `start_key` is a
    /// duplicate; the key names a run only within this store.
    pub async fn start(
        &self,
        workflow_json: &[u8],
        input: Value,
        start_key: &str,
    ) -> Result<StartOutcome, HostError> {
        self.start_from(workflow_json, None, input, start_key).await
    }

    /// `start` with the package of a loaded workflow artifact. The store keeps the
    /// artifact and the run records its digest; a run found by its start key keeps
    /// the artifact it was first started from.
    pub async fn start_artifact(
        &self,
        artifact: &Artifact,
        input: Value,
        start_key: &str,
    ) -> Result<StartOutcome, HostError> {
        if self.inner.files.is_none() && files::declares_files(&artifact.manifest) {
            return Err(HostError::invalid(
                FILES_DIR_MISSING,
                "the workflow declares instructions or assets, and the engine has no files_dir",
            ));
        }
        let stored = StoredArtifact {
            digest: artifact.digest.clone(),
            bytes: artifact.bytes.clone(),
        };
        self.start_from(&artifact.package, Some(stored), input, start_key)
            .await
    }

    async fn start_from(
        &self,
        workflow_json: &[u8],
        artifact: Option<StoredArtifact>,
        input: Value,
        start_key: &str,
    ) -> Result<StartOutcome, HostError> {
        match self
            .try_start(workflow_json, artifact.clone(), &input, start_key)
            .await
        {
            // Another start with this key created its run after our lookup; the next
            // lookup finds it.
            Err(HostError::Store(f)) if f.code == "run.start-key-mismatch" => {
                self.try_start(workflow_json, artifact, &input, start_key)
                    .await
            }
            other => other,
        }
    }

    async fn try_start(
        &self,
        workflow_json: &[u8],
        artifact: Option<StoredArtifact>,
        input: &Value,
        start_key: &str,
    ) -> Result<StartOutcome, HostError> {
        let inner = &self.inner;
        let tenant = &inner.config.tenant;
        let found = retry(false, || inner.store.find_run(tenant, start_key)).await?;
        let key = match found {
            Some(key) => key,
            None => self.key(&inner.config.run_ids.next()?),
        };
        let started = evidence(&RunStarted {
            tenant: tenant.clone(),
            run_id: key.run.clone(),
            input: input.clone(),
        })?;
        let Checked { package, workflow } = check_workflow(
            &*inner.reducer,
            &inner.activities,
            inner.config.limits,
            workflow_json,
            &started,
        )?;
        // The input against the workflow's inputSchema, before anything is created
        // (spec/host.md §2, spec/schema.md §4).
        if let Some(schema) = workflow.get("inputSchema") {
            runspore_schema::fit_input(schema, input).map_err(HostError::InputInvalid)?;
        }
        let package_digest = digest::package(&package);
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
            base_dir: Some(inner.config.base_dir.to_string_lossy().into_owned()),
            artifact,
        };
        let receipt = retry(true, || inner.store.create_run(request.clone())).await?;
        Ok(StartOutcome {
            key: receipt.value.key,
            created: receipt.disposition == Disposition::Applied,
        })
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
        if let Some(package) = self.run_package(key).await? {
            let routed = outcome.as_deref().unwrap_or(OUTCOME_RECEIVED);
            if let Some(schema) = package.signal_schema(name, routed) {
                runspore_schema::fit(routed, schema, &data).map_err(HostError::OutputInvalid)?;
            }
        }
        let body = evidence(&SignalReceived {
            name: name.to_string(),
            outcome,
            data,
        })?;
        let event_id = ids::event_for_signal(message_id);
        let request_id = format!("signal/{message_id}");
        self.inner
            .append(key, request_id, event_id, event_kind::SIGNAL_RECEIVED, body)
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
        if let Resolution::Complete { outcome, output } = &resolution {
            self.check_completion(key, invocation_id, outcome, output)
                .await?;
        }
        let body = evidence(&InvocationResolved {
            invocation_id: invocation_id.to_string(),
            resolution,
        })?;
        let event_id = ids::event_for_resolution(request_id);
        let request_id = format!("resolve/{request_id}");
        self.inner
            .append(
                key,
                request_id,
                event_id,
                event_kind::INVOCATION_RESOLVED,
                body,
            )
            .await
    }

    /// The package of a run, or `None` when there is no such run (the append that
    /// follows reports that).
    async fn run_package(&self, key: &RunKey) -> Result<Option<Arc<Package>>, HostError> {
        match self.get_run(key).await? {
            Some(view) => Ok(Some(self.inner.package(&view.package_digest).await?)),
            None => Ok(None),
        }
    }

    /// Checks a `complete` resolution's output against the schema the invocation's
    /// node declares for its outcome. An unknown invocation is left to the kernel.
    async fn check_completion(
        &self,
        key: &RunKey,
        invocation_id: &str,
        outcome: &str,
        output: &Value,
    ) -> Result<(), HostError> {
        let Some(package) = self.run_package(key).await? else {
            return Ok(());
        };
        let invocations = self.list_invocations(key).await?;
        let node = invocations
            .iter()
            .find(|i| i.invocation_id == invocation_id)
            .map(|i| i.node_id.as_str());
        match node.and_then(|node| package.schema(node, outcome)) {
            Some(schema) => {
                runspore_schema::fit(outcome, schema, output).map_err(HostError::OutputInvalid)
            }
            None => Ok(()),
        }
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

    /// The canonical package bytes stored under `digest`.
    pub async fn get_package(&self, digest: &str) -> Result<Option<Vec<u8>>, HostError> {
        Ok(self.inner.store.get_package(digest).await?)
    }

    /// The workflow artifact bytes stored under `digest`.
    pub async fn get_artifact(&self, digest: &str) -> Result<Option<Vec<u8>>, HostError> {
        Ok(self.inner.store.get_artifact(digest).await?)
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

    /// The arrow into the run's position and, once it is `completed` or `failed`, the
    /// node it ended at. Replays the run's consumed events through the reducer, so it
    /// costs one transition per event; the kernel's state holds no arrow.
    pub async fn arrival(&self, view: &RunView) -> Result<Arrival, HostError> {
        let Some(current) = view.snapshot.as_deref() else {
            return Ok(Arrival::default());
        };
        let undecodable = |e: canonical::CanonError| HostError::invalid(e.code(), e.to_string());
        let state: State = canonical::decode(current).map_err(undecodable)?;
        let package = self.inner.package(&view.package_digest).await?;
        let workflow: Workflow = canonical::decode(&package.bytes).map_err(undecodable)?;
        let identity = Identity {
            semantics_version: state.semantics,
            ..identity(&*self.inner.reducer, &view.package_digest)
        };
        // Every replayed decision was committed once, so no budget can refuse it.
        let limits = Limits {
            microsteps: u32::MAX,
            expression_operations: u32::MAX,
            max_state_bytes: u32::MAX,
            max_command_count: u32::MAX,
        };
        let mut trail = Trail::new(&workflow);
        let mut snapshot = None;
        let mut after = 0;
        while after < view.applied_sequence {
            let page = self.list_events(&view.key, after, 100).await?;
            if page.items.is_empty() {
                break;
            }
            for record in page.items {
                let event = record.event;
                after = event.sequence;
                if after > view.applied_sequence {
                    break;
                }
                let decision = self.inner.reducer.transition(&TransitionRequest {
                    identity: identity.clone(),
                    graph: package.bytes.clone(),
                    snapshot: snapshot.take(),
                    input_event: Envelope {
                        event_id: event.id,
                        sequence: event.sequence,
                        accepted_at_ms: event.accepted_at_ms,
                        kind: event.kind,
                        payload: event.body,
                    },
                    frozen_limits: limits,
                })?;
                trail.push(canonical::decode(&decision.snapshot).map_err(undecodable)?);
                snapshot = Some(decision.snapshot);
            }
        }
        Ok(trail.finish())
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
            if let DueItem::Lease { attempt, process } = item {
                if inner.expire(attempt, process.as_ref()).await? {
                    report.expired += 1;
                }
            }
        }
        inner.sweep_files().await;
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
        if inner.closing.load(Ordering::SeqCst) {
            return Ok(report);
        }
        for item in inner.scan_due().await? {
            if lock(&inner.inflight).len() >= inner.config.max_concurrent_activities {
                break;
            }
            if let DueItem::Outbox { key, invocation_id } = item {
                match Inner::dispatch(inner, key, invocation_id).await? {
                    Dispatched::Started => report.dispatched += 1,
                    Dispatched::Quarantined => report.quarantined += 1,
                    Dispatched::Not => {}
                }
            }
        }
        Ok(report)
    }

    /// Ticks until `shutdown` resolves, sleeping `poll_ms` after a tick that did
    /// nothing, then calls `Engine::shutdown`.
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
        self.shutdown().await;
        Ok(())
    }

    /// Stops the engine for good: from now on no attempt is claimed, by this call or
    /// any later `tick`. Waits up to `shutdown_grace_ms` for in-flight attempts,
    /// signals the rest to stop and waits up to `shutdown_grace_ms` again; an attempt
    /// whose runner returns is finished with what it reported. Attempts still running
    /// are aborted and left to lease expiry. A last tick then sweeps and coordinates,
    /// so the results just recorded are applied; its errors are ignored. Repeating the
    /// call is harmless.
    pub async fn shutdown(&self) {
        let inner = &self.inner;
        inner.closing.store(true, Ordering::SeqCst);
        let grace = Duration::from_millis(inner.config.shutdown_grace_ms);
        inner.drain(grace).await;
        inner.shutdown.send_replace(true);
        inner.drain(grace).await;
        let mut tasks = std::mem::take(&mut *lock(&inner.tasks));
        tasks.abort_all();
        drop(tasks);
        lock(&inner.inflight).clear();
        let _ = self.tick().await;
    }

    /// Ticks until the run is terminal, `waiting`, `needs-intervention` or quarantined,
    /// or waits for an actor at an external step, with no pending event and nothing of
    /// it in flight.
    pub async fn run_until_parked(&self, key: &RunKey) -> Result<RunView, HostError> {
        let inner = &self.inner;
        loop {
            let report = self.tick().await?;
            let busy = lock(&inner.inflight).iter().any(|(k, _)| k == key);
            let view = self.get_run(key).await?.ok_or_else(|| {
                HostError::invalid("run.not-found", format!("run {} does not exist", key.run))
            })?;
            if !busy && (parked(&view) || self.waits_on_actor(&view).await?) {
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
        Ok(retry(true, || self.store.append_event(request.clone())).await?)
    }

    fn unique(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}/{}/{n}", self.nonce)
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
        let section = |name: &str| {
            workflow
                .get(name)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
        };
        let (actions, nodes) = (section("actions"), section("nodes"));
        let max_visits = workflow
            .pointer("/limits/maxVisitsPerNode")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(WorkflowLimits::default().max_visits_per_node);
        let on_intervention = workflow.get("onIntervention").cloned();
        let package = Arc::new(Package {
            bytes,
            actions,
            nodes,
            max_visits,
            on_intervention,
        });
        lock(&self.packages).insert(digest.to_string(), package.clone());
        Ok(package)
    }

    /// Expires an attempt whose lease ran out, once the process group its record names
    /// is gone from this machine or cannot be checked from here (host §3).
    async fn expire(
        &self,
        attempt: AttemptRef,
        process: Option<&ProcessRecord>,
    ) -> Result<bool, HostError> {
        let grace = Duration::from_millis(self.config.stop_grace_ms);
        let process_unchecked = match process {
            None => false,
            Some(record) => match process::settle(record, grace).await {
                Leftover::Gone => false,
                Leftover::Unchecked => true,
                Leftover::Remains => return Ok(false),
            },
        };
        let request = ExpireAttempt {
            mutation: mutation(
                "expire_attempt",
                format!("expire/{}", attempt.attempt_id),
                json!([attempt, process_unchecked]),
            )?,
            attempt,
            process_unchecked,
            reason: ExpireReason::Lease,
        };
        match retry(true, || self.store.expire_attempt(request.clone())).await {
            Ok(_) => Ok(true),
            // Another sweeper expired it first, or marked it the other way.
            Err(f)
                if is(&f, StoreFailureKind::Stale, "lease.lost")
                    || is(&f, StoreFailureKind::Conflict, "lease.not-due")
                    || is(&f, StoreFailureKind::Conflict, "request.digest-mismatch") =>
            {
                Ok(false)
            }
            Err(f) => Err(f.into()),
        }
    }

    async fn quarantine(&self, turn: &Turn, code: &str, details: &str) -> Result<(), HostError> {
        self.quarantine_at(&turn.key, turn.revision, code, details)
            .await
    }

    /// Quarantines the run at `revision`; a run that has moved on is left alone.
    async fn quarantine_at(
        &self,
        key: &RunKey,
        revision: u64,
        code: &str,
        details: &str,
    ) -> Result<(), HostError> {
        let request = QuarantineRun {
            mutation: mutation(
                "quarantine_run",
                self.unique(&format!("quarantine/{revision}")),
                json!([key, revision, code, details]),
            )?,
            key: key.clone(),
            expected_revision: revision,
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
    async fn coordinate(
        self: &Arc<Self>,
        key: &RunKey,
        report: &mut TickReport,
    ) -> Result<(), HostError> {
        for _ in 0..self.config.turns_per_run {
            let Some(turn) = retry(false, || self.store.load_turn(key)).await? else {
                return Ok(());
            };
            let package = self.package(&turn.package_digest).await?;
            let event = &turn.next_event;
            let request = TransitionRequest {
                identity: turn_identity(
                    &*self.reducer,
                    &turn.package_digest,
                    turn.snapshot.as_deref(),
                ),
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
                Ok(receipt) if receipt.disposition == Disposition::Applied => {
                    report.turns += 1;
                    let before = turn.snapshot.as_deref().and_then(snapshot_status);
                    self.tell(&turn, &package, before, &request.snapshot);
                }
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

    async fn set_process(
        &self,
        attempt: &AttemptRef,
        process: Option<ProcessRecord>,
    ) -> Result<(), HostError> {
        let request = SetProcess {
            mutation: mutation(
                "set_process",
                self.unique(&format!("process/{}", attempt.attempt_id)),
                json!([attempt, process]),
            )?,
            attempt: attempt.clone(),
            process,
        };
        retry(true, || self.store.set_process(request.clone())).await?;
        Ok(())
    }

    /// Gives one due invocation its files, claims it and starts its attempt in the
    /// background, unless the engine is shutting down.
    async fn dispatch(
        this: &Arc<Self>,
        key: RunKey,
        invocation_id: String,
    ) -> Result<Dispatched, HostError> {
        let slot = (key.clone(), invocation_id.clone());
        if this.closing.load(Ordering::SeqCst) || lock(&this.inflight).contains(&slot) {
            return Ok(Dispatched::Not);
        }
        // An actor claims an external step; the worker never does.
        if this.is_external(&key, &invocation_id).await {
            return Ok(Dispatched::Not);
        }
        let delivery = match this.deliver(&key, &invocation_id).await? {
            Ok(delivery) => delivery,
            Err(Undeliverable::Quarantined) => return Ok(Dispatched::Quarantined),
            Err(Undeliverable::Skip) => return Ok(Dispatched::Not),
        };
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
            holder: None,
            deadline_duration_ms: None,
        };
        let receipt = match retry(true, || this.store.claim_attempt(request.clone())).await {
            Ok(receipt) => receipt,
            Err(f) if matches!(f.kind, StoreFailureKind::Conflict | StoreFailureKind::Stale) => {
                return Ok(Dispatched::Not)
            }
            Err(f) => return Err(f.into()),
        };
        let claim = receipt.value;
        if receipt.disposition == Disposition::Duplicate {
            match this.heartbeat(&claim.attempt).await {
                Ok(hb) if hb.disposition == Disposition::Applied => {}
                _ => return Ok(Dispatched::Not),
            }
        }
        lock(&this.inflight).insert(slot.clone());
        let task = this.clone();
        lock(&this.tasks).spawn(async move {
            task.attempt(claim, delivery).await;
            lock(&task.inflight).remove(&slot);
            task.settled.notify_waiters();
        });
        Ok(Dispatched::Started)
    }

    /// The invocation's view and its run's package; `None` when either is gone.
    async fn invocation_package(
        &self,
        key: &RunKey,
        invocation_id: &str,
    ) -> Result<Option<(InvocationView, Arc<Package>)>, HostError> {
        let Some(view) = retry(false, || self.store.get_run(key)).await? else {
            return Ok(None);
        };
        let package = self.package(&view.package_digest).await?;
        let invocations = retry(false, || self.store.list_invocations(key)).await?;
        Ok(invocations
            .into_iter()
            .find(|i| i.invocation_id == invocation_id)
            .map(|invocation| (invocation, package)))
    }

    /// Whether a due invocation belongs to an `external` action. Remembered, since an
    /// invocation's action never changes; a run that cannot be read counts as not
    /// external and is left to the usual dispatch.
    async fn is_external(&self, key: &RunKey, invocation_id: &str) -> bool {
        let slot = (key.clone(), invocation_id.to_string());
        if lock(&self.external).contains(&slot) {
            return true;
        }
        let found = match self.invocation_package(key, invocation_id).await {
            Ok(Some((invocation, package))) => package
                .actions
                .get(&invocation.action_id)
                .is_some_and(external::is_external),
            _ => false,
        };
        if found {
            lock(&self.external).insert(slot);
        }
        found
    }

    /// The step files and work folder of a due invocation (host §3, Step files and
    /// work folders): checked, or made again, before every dispatch. A run whose files
    /// cannot be made is quarantined.
    async fn deliver(
        &self,
        key: &RunKey,
        invocation_id: &str,
    ) -> Result<Result<Delivery, Undeliverable>, HostError> {
        if self.files.is_none() {
            // Nothing to deliver; `resolve_action` catches an artifact with files.
            return Ok(Ok(Delivery::default()));
        }
        let Some(view) = retry(false, || self.store.get_run(key)).await? else {
            return Ok(Err(Undeliverable::Skip));
        };
        let invocations = retry(false, || self.store.list_invocations(key)).await?;
        let Some(invocation) = invocations
            .iter()
            .find(|i| i.invocation_id == invocation_id)
        else {
            return Ok(Err(Undeliverable::Skip));
        };
        match self.make_files(&view, invocation).await {
            Ok(delivery) => Ok(Ok(delivery)),
            Err(unmade) => self.undeliverable(&view, unmade).await.map(Err),
        }
    }

    async fn undeliverable(
        &self,
        view: &RunView,
        unmade: Unmade,
    ) -> Result<Undeliverable, HostError> {
        let (code, details) = match unmade {
            Unmade::Unreadable(details) => (ARTIFACT_UNREADABLE, details),
            Unmade::Unwritable(details) => (FILES_UNWRITABLE, details),
            Unmade::DirMissing => (
                FILES_DIR_MISSING,
                "the run's workflow declares files, and the engine has no files_dir".into(),
            ),
        };
        self.quarantine_at(&view.key, view.revision, code, &details)
            .await?;
        Ok(Undeliverable::Quarantined)
    }

    /// Checks the invocation's step files, making them again when the check fails, and
    /// creates its work folder.
    async fn make_files(
        &self,
        view: &RunView,
        invocation: &InvocationView,
    ) -> Result<Delivery, Unmade> {
        let dir = self.files.as_ref().expect("checked by the caller");
        let work_dir = Some(dir.work_dir(&view.key.run, &invocation.invocation_id)?);
        let Some(artifact) = view.artifact_digest.as_deref() else {
            return Ok(Delivery {
                work_dir,
                ..Delivery::default()
            });
        };
        let manifest = self.manifest(artifact).await?;
        let node = &invocation.node_id;
        let (wanted, instructions, assets) =
            files::wanted(&manifest, node, Some(&invocation.action_id));
        let step_dir = dir.step_dir(artifact, node)?;
        let lock = loop {
            // A sweep holds the lock exclusive only while it reads the runs.
            match dir.try_lock_shared(artifact)? {
                Some(lock) => break lock,
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        };
        if !files::intact(&step_dir, &wanted) {
            let bytes = self.artifact_bytes(artifact).await?;
            let mut reader = ArtifactReader::open(&bytes)?;
            files::make(&step_dir, &wanted, |digest| reader.entry(digest))?;
        }
        Ok(Delivery {
            files: Some(StepFiles {
                instructions: instructions.map(|path| step_dir.join(path)),
                dir: step_dir,
                assets,
            }),
            work_dir,
            _lock: Some(lock),
        })
    }

    /// The stored artifact's bytes, checked against its digest.
    async fn artifact_bytes(&self, artifact: &str) -> Result<Vec<u8>, Unmade> {
        let stored = retry(false, || self.store.get_artifact(artifact))
            .await
            .map_err(|f| Unmade::Unreadable(format!("{}: {}", f.code, f.message)))?;
        let bytes = stored
            .ok_or_else(|| Unmade::Unreadable(format!("the store holds no artifact {artifact}")))?;
        if digest::artifact(&bytes) != artifact {
            return Err(Unmade::Unreadable(format!(
                "the stored artifact {artifact} does not match its digest"
            )));
        }
        Ok(bytes)
    }

    /// The manifest of a stored artifact, read once per engine.
    async fn manifest(&self, artifact: &str) -> Result<Arc<Manifest>, Unmade> {
        if let Some(manifest) = lock(&self.manifests).get(artifact) {
            return Ok(manifest.clone());
        }
        let bytes = self.artifact_bytes(artifact).await?;
        let manifest = Arc::new(Manifest::decode(
            &ArtifactReader::open(&bytes)?.manifest()?,
        )?);
        lock(&self.manifests).insert(artifact.to_string(), manifest.clone());
        Ok(manifest)
    }

    /// Removes the step files of artifacts no unfinished run of this tenant pins, at
    /// most once every `FILES_SWEEP_EVERY`. Its failures wait for the next sweep.
    async fn sweep_files(&self) {
        let Some(dir) = &self.files else {
            return;
        };
        {
            let mut swept = lock(&self.files_swept);
            if swept.is_some_and(|at| at.elapsed() < FILES_SWEEP_EVERY) {
                return;
            }
            *swept = Some(Instant::now());
        }
        let _ = dir
            .sweep(|candidates| async move {
                let mut unpinned: BTreeSet<String> = candidates.into_iter().collect();
                let mut cursor = None;
                loop {
                    let tenant = &self.config.tenant;
                    let page = retry(false, || self.store.list_runs(tenant, cursor.clone(), PAGE))
                        .await
                        .ok()?;
                    for run in &page.items {
                        let finished = serde_json::from_value::<RunStatus>(json!(run.status))
                            .is_ok_and(RunStatus::is_terminal);
                        if let (false, Some(artifact)) = (finished, &run.artifact_digest) {
                            unpinned.remove(artifact);
                        }
                    }
                    cursor = page.next;
                    if cursor.is_none() {
                        return Some(unpinned);
                    }
                }
            })
            .await;
    }

    /// Runs one claimed attempt to its reported result.
    async fn attempt(self: &Arc<Self>, claim: Claim, delivery: Delivery) {
        failpoint::hit("host.attempt.after-claim");
        let attempt = claim.attempt.clone();
        let (stop, stop_rx) = watch::channel(false);
        let mut lost = false;
        let mut stopping = false;
        let output = match self.resolve_action(&attempt.key, &claim).await {
            Err(Unrunnable::Abandon) => return,
            Err(Unrunnable::Fail(output)) => output,
            Ok(Runnable {
                action,
                runner,
                input,
                params,
                visit,
                base_dir,
                package,
            }) => {
                let mut ctx = ActivityContext::new(
                    attempt.key.clone(),
                    claim.node_id.clone(),
                    claim.action_id.clone(),
                    attempt.invocation_id.clone(),
                    claim.attempt_number,
                    visit,
                    claim.effect_key.clone(),
                    base_dir,
                    stop_rx,
                );
                if let Some(params) = params {
                    ctx.params = params;
                }
                ctx.db = self.config.db_path.clone();
                ctx.files = delivery.files.clone();
                ctx.work_dir = delivery.work_dir.clone();
                ctx.guard = self.config.guard.clone();
                ctx.stop_grace = Duration::from_millis(self.config.stop_grace_ms);
                let (inner, held) = (self.clone(), attempt.clone());
                ctx.process = Arc::new(move |process| {
                    let (inner, held) = (inner.clone(), held.clone());
                    Box::pin(async move {
                        inner
                            .set_process(&held, process)
                            .await
                            .map_err(|e| e.to_string())
                    })
                });
                let run = runner.run(&ctx, &action, &input);
                tokio::pin!(run);
                let period = Duration::from_millis(self.config.heartbeat_ms.max(1));
                let mut beat =
                    tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                let mut shutdown = self.shutdown.subscribe();
                let output = loop {
                    tokio::select! {
                        output = &mut run => break output,
                        _ = beat.tick(), if !lost => {
                            if let Err(f) = self.heartbeat(&attempt).await {
                                if f.kind == StoreFailureKind::Stale {
                                    lost = true;
                                    stop.send_replace(true);
                                }
                            }
                        }
                        () = async { let _ = shutdown.wait_for(|s| *s).await; }, if !stopping => {
                            stopping = true;
                            stop.send_replace(true);
                        }
                    }
                };
                checked(output, |outcome| {
                    package.schema(&claim.node_id, outcome).cloned()
                })
            }
        };
        // The try is over: a sweep may take the artifact's step files from here on.
        drop(delivery);
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

    /// Finds the action, its runner, the input, the run's base folder and the package
    /// of a claimed attempt.
    async fn resolve_action(&self, key: &RunKey, claim: &Claim) -> Result<Runnable, Unrunnable> {
        let fail = |code: &str, message: String| {
            Unrunnable::Fail(ActivityOutput::Failure {
                error: ActivityOutput::error(code, message, Value::Null),
                retryable: false,
            })
        };
        let Ok(Some(view)) = retry(false, || self.store.get_run(key)).await else {
            return Err(Unrunnable::Abandon);
        };
        if let (None, Some(artifact)) = (&self.files, view.artifact_digest.as_deref()) {
            // A run started by an engine with a files folder: its steps cannot get
            // their files here. The lease runs out, as after a crash.
            if self
                .manifest(artifact)
                .await
                .is_ok_and(|m| files::declares_files(&m))
            {
                let _ = self.undeliverable(&view, Unmade::DirMissing).await;
                return Err(Unrunnable::Abandon);
            }
        }
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
        let params = claim
            .params
            .as_deref()
            .map(canonical::parse)
            .transpose()
            .map_err(|e| fail("host.params-invalid", e.to_string()))?;
        let visit = invocation_visit(key, claim, package.max_visits).ok_or_else(|| {
            fail(
                "host.invocation-unknown",
                claim.attempt.invocation_id.clone(),
            )
        })?;
        let base_dir = view
            .base_dir
            .map_or_else(|| self.config.base_dir.clone(), PathBuf::from);
        Ok(Runnable {
            action,
            runner,
            input,
            params,
            visit,
            base_dir,
            package,
        })
    }
}
