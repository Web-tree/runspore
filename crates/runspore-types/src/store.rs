//! The semantic store protocol: a native async contract derived from
//! `contracts/store.ts`. Operation semantics, atomicity, and failure codes are
//! specified in `spec/store.md`; passing the type checker is not conformance.
//!
//! Rules that hold for every mutating operation:
//! - The caller chooses `request_id` before the call and reuses it on retry.
//! - The store looks up `(run, request_id)` first. A stored receipt with the same
//!   digest is returned with `Disposition::Duplicate` before any other predicate is
//!   evaluated; the same ID with a different digest is `Conflict`/`request.digest-mismatch`.
//! - Receipt lookup, the mutation, and receipt insertion share one transaction.
//! - Time is the store's authority clock, sampled after the write lock is held.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::canonical::{self, CanonError};
use crate::digest::{self, Digest};
use crate::model::u64_str;
use crate::reducer::{Command, Diagnostic};

pub const STORE_PROTOCOL: &str = "0.1";
pub const DEFAULT_TENANT: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunKey {
    pub tenant: String,
    pub run: String,
}

impl RunKey {
    pub fn new(tenant: impl Into<String>, run: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            run: run.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mutation {
    pub request_id: String,
    pub request_digest: Digest,
}

impl Mutation {
    /// Builds the identity of a request from its operation name and body.
    /// `body` must contain every field that defines the request's meaning.
    pub fn new<T: Serialize>(
        operation: &str,
        request_id: impl Into<String>,
        body: &T,
    ) -> Result<Self, CanonError> {
        let bytes = canonical::encode(body)?;
        Ok(Self {
            request_id: request_id.into(),
            request_digest: digest::request(operation, &bytes),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    Applied,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt<T> {
    pub request_id: String,
    pub request_digest: Digest,
    pub disposition: Disposition,
    pub value: T,
}

/// A receipt read back by request ID; `value` is the canonical JSON of the typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReceipt {
    pub request_id: String,
    pub request_digest: Digest,
    pub operation: String,
    pub value: Vec<u8>,
}

impl StoredReceipt {
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, CanonError> {
        canonical::decode(&self.value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoreFailureKind {
    /// The request contradicts stored state and will never succeed as written.
    Conflict,
    /// The caller's revision, fence, or lease is no longer current.
    Stale,
    /// Transient; retry the identical request.
    Unavailable,
    /// The outcome is not known; query the receipt or retry the identical request.
    UnknownCommit,
    Quota,
    Incompatible,
    Corrupt,
    Unauthorized,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?} {code}: {message}")]
pub struct StoreFailure {
    pub kind: StoreFailureKind,
    /// Stable code from `spec/store.md`.
    pub code: String,
    pub message: String,
}

impl StoreFailure {
    pub fn new(
        kind: StoreFailureKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
        }
    }
}

pub type StoreResult<T> = Result<T, StoreFailure>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub protocol: String,
    pub atomic_run_mutation: bool,
    pub conditional_writes: bool,
    pub unique_insert: bool,
    pub authoritative_readback: bool,
    pub ordered_recovery_scan: bool,
    pub multiworker_claims: bool,
    pub persistent_wakeup: bool,
    pub failure_model: String,
    pub transaction_scope: String,
    pub max_record_bytes: u64,
}

/// Canonical JSON plus its `digest::body`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub digest: Digest,
    pub body: Vec<u8>,
}

impl Evidence {
    pub fn from_canonical(body: Vec<u8>) -> Self {
        Self {
            digest: digest::body(&body),
            body,
        }
    }

    pub fn encode<T: Serialize>(value: &T) -> Result<Self, CanonError> {
        Ok(Self::from_canonical(canonical::encode(value)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

// --- create_run -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRun {
    pub mutation: Mutation,
    pub key: RunKey,
    /// Unique per tenant. The same start key always names the same run.
    pub start_key: String,
    pub package_digest: Digest,
    /// Canonical workflow document; stored content-addressed.
    pub package: Vec<u8>,
    /// Canonical `RunStarted` payload; becomes event sequence 1 with ID `start`.
    pub started: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCreated {
    pub key: RunKey,
    #[serde(with = "u64_str")]
    pub started_sequence: u64,
}

// --- append_event -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEvent {
    pub mutation: Mutation,
    pub key: RunKey,
    pub event_id: String,
    pub kind: String,
    pub body: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAccepted {
    #[serde(with = "u64_str")]
    pub sequence: u64,
    #[serde(with = "u64_str")]
    pub accepted_at_ms: u64,
}

// --- load_turn / commit_turn --------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: String,
    pub sequence: u64,
    pub accepted_at_ms: u64,
    pub kind: String,
    pub body: Vec<u8>,
    pub body_digest: Digest,
}

/// A coherent read of one run and its next pending event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub key: RunKey,
    /// Committed transitions so far.
    pub revision: u64,
    /// Sequence of the last consumed event; `next_event.sequence` is this plus one.
    pub applied_sequence: u64,
    pub package_digest: Digest,
    pub snapshot: Option<Vec<u8>>,
    pub snapshot_digest: Option<Digest>,
    pub next_event: StoredEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitTurn {
    pub mutation: Mutation,
    pub key: RunKey,
    pub expected_revision: u64,
    pub event_id: String,
    pub event_sequence: u64,
    pub decision_digest: Digest,
    pub snapshot: Vec<u8>,
    pub snapshot_digest: Digest,
    /// `State.status` of the new snapshot, as its wire string. Indexed for scans and listings.
    pub status: String,
    pub commands: Vec<Command>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitReceipt {
    pub key: RunKey,
    #[serde(with = "u64_str")]
    pub revision: u64,
    #[serde(with = "u64_str")]
    pub applied_sequence: u64,
    pub decision_digest: Digest,
}

// --- attempts ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRef {
    pub key: RunKey,
    pub invocation_id: String,
    pub attempt_id: String,
    pub owner: String,
    #[serde(with = "u64_str")]
    pub fence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimAttempt {
    pub mutation: Mutation,
    pub key: RunKey,
    pub invocation_id: String,
    pub worker: String,
    pub lease_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub attempt: AttemptRef,
    pub attempt_number: u32,
    #[serde(with = "u64_str")]
    pub lease_until_ms: u64,
    pub node_id: String,
    pub action_id: String,
    pub effect_key: String,
    pub input_digest: Digest,
    /// Canonical JSON.
    pub input: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub mutation: Mutation,
    pub attempt: AttemptRef,
    pub extend_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseExtended {
    #[serde(with = "u64_str")]
    pub lease_until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishAttempt {
    pub mutation: Mutation,
    pub attempt: AttemptRef,
    /// Canonical `ActivityResult` payload; appended as an `activity.result` event.
    pub result: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpireAttempt {
    pub mutation: Mutation,
    pub attempt: AttemptRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultAccepted {
    #[serde(with = "u64_str")]
    pub result_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLateEvidence {
    pub mutation: Mutation,
    pub attempt: AttemptRef,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecorded {
    pub audit_id: String,
}

// --- quarantine -------------------------------------------------------------

/// Stops coordination of a run after a kernel failure. The pending event stays pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRun {
    pub mutation: Mutation,
    pub key: RunKey,
    pub expected_revision: u64,
    pub code: String,
    pub details: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRun {
    pub mutation: Mutation,
    pub key: RunKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineChanged {
    pub quarantined: bool,
}

// --- scans and views --------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DueItem {
    /// A running attempt whose lease deadline has passed; expire it.
    Lease { attempt: AttemptRef },
    /// An invocation that may be claimed now.
    Outbox { key: RunKey, invocation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quarantine {
    pub code: String,
    pub details: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    pub key: RunKey,
    pub start_key: String,
    pub package_digest: Digest,
    /// `State.status` wire string, or `created` before the first commit.
    pub status: String,
    pub revision: u64,
    pub applied_sequence: u64,
    /// Sequence the next accepted event will receive.
    pub next_sequence: u64,
    pub snapshot: Option<Vec<u8>>,
    pub snapshot_digest: Option<Digest>,
    pub quarantine: Option<Quarantine>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub event: StoredEvent,
    /// The revision whose commit consumed this event; `None` while pending.
    pub consumed_revision: Option<u64>,
    /// Diagnostics of that commit.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationStatus {
    /// Authorized and not yet claimed.
    Pending,
    /// Claimed; an attempt holds a live or not-yet-expired lease.
    Running,
    /// The latest attempt ended; the kernel has not issued another.
    Settled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationView {
    pub invocation_id: String,
    pub node_id: String,
    pub action_id: String,
    pub status: InvocationStatus,
    /// The attempt number most recently authorized by the kernel.
    pub attempt_number: u32,
    pub not_before_ms: u64,
    pub lease_until_ms: Option<u64>,
    pub owner: Option<String>,
}

#[async_trait]
pub trait Store: Send + Sync {
    async fn capabilities(&self) -> StoreResult<Capabilities>;

    async fn create_run(&self, request: CreateRun) -> StoreResult<Receipt<RunCreated>>;
    async fn append_event(&self, request: AppendEvent) -> StoreResult<Receipt<EventAccepted>>;

    /// `None` when the run has no pending event or is quarantined.
    async fn load_turn(&self, key: &RunKey) -> StoreResult<Option<Turn>>;
    async fn get_package(&self, digest: &str) -> StoreResult<Option<Vec<u8>>>;
    async fn commit_turn(&self, request: CommitTurn) -> StoreResult<Receipt<CommitReceipt>>;

    /// Absent is not proof that an in-flight original request aborted.
    async fn get_receipt(
        &self,
        key: &RunKey,
        request_id: &str,
    ) -> StoreResult<Option<StoredReceipt>>;

    async fn claim_attempt(&self, request: ClaimAttempt) -> StoreResult<Receipt<Claim>>;
    async fn heartbeat(&self, request: Heartbeat) -> StoreResult<Receipt<LeaseExtended>>;
    async fn finish_attempt(&self, request: FinishAttempt) -> StoreResult<Receipt<ResultAccepted>>;
    async fn expire_attempt(&self, request: ExpireAttempt) -> StoreResult<Receipt<ResultAccepted>>;
    async fn record_late_evidence(
        &self,
        request: RecordLateEvidence,
    ) -> StoreResult<Receipt<AuditRecorded>>;

    async fn quarantine_run(
        &self,
        request: QuarantineRun,
    ) -> StoreResult<Receipt<QuarantineChanged>>;
    async fn release_run(&self, request: ReleaseRun) -> StoreResult<Receipt<QuarantineChanged>>;

    /// Runs with a pending event that are not quarantined, in a stable order.
    async fn scan_ready(&self, cursor: Option<String>, limit: u32) -> StoreResult<Page<RunKey>>;
    /// Expired leases and claimable invocations, in a stable order.
    async fn scan_due(&self, cursor: Option<String>, limit: u32) -> StoreResult<Page<DueItem>>;

    async fn get_run(&self, key: &RunKey) -> StoreResult<Option<RunView>>;
    async fn list_runs(
        &self,
        tenant: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> StoreResult<Page<RunView>>;
    /// Events with sequence greater than `after_sequence`, ascending.
    async fn list_events(
        &self,
        key: &RunKey,
        after_sequence: u64,
        limit: u32,
    ) -> StoreResult<Page<EventRecord>>;
    async fn list_invocations(&self, key: &RunKey) -> StoreResult<Vec<InvocationView>>;
}

/// The store's authority for "now". Injected so lease and backoff rules are testable.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}
