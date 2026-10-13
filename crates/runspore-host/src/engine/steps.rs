//! Steps outside the worker (spec/host.md §2): an actor claims an `external` step's
//! attempt, renews it, and reports its result; a person may abandon it.

use std::path::PathBuf;

use runspore_types::canonical;
use runspore_types::model::{ActivityResult, AttemptStatus, ErrorInfo, InvocationPhase, State};
use runspore_types::store::*;
use serde_json::{json, Value};

use super::{evidence, mutation, retry, Engine, Undeliverable};
use crate::error::HostError;
use crate::external::{self, External, StepToken};

/// The result channel a claim reports through: the store, by `step complete` or
/// `step fail`.
pub const RESULT_CHANNEL: &str = "store";
/// The largest result body an actor may report.
const MAX_RESULT_BYTES: usize = 64 * 1024;
/// The code of a reported failure that names none.
const STEP_FAILED: &str = "step.failed";

/// Whether an external step waits for an actor or is held by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Ready,
    Held,
}

impl StepState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Held => "held",
        }
    }
}

/// The external step a run is at, as `status` shows it: never its input, owner or
/// fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStep {
    pub kind: &'static str,
    pub actor: Option<String>,
    pub state: StepState,
    /// The attempt the kernel last authorized.
    pub attempt: u32,
    pub invocation_id: String,
    pub node_id: String,
    pub action_id: String,
    /// While held: the holder handle given at the claim.
    pub holder: Option<Value>,
    pub deadline_ms: Option<u64>,
    pub lease_until_ms: Option<u64>,
}

/// What an actor gets from a claim: everything needed to do the step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepClaim {
    pub token: StepToken,
    pub key: RunKey,
    pub node_id: String,
    pub action_id: String,
    pub invocation_id: String,
    pub attempt: u32,
    pub input: Value,
    pub outcomes: Vec<String>,
    /// The node's `outputSchemas`, or an empty object.
    pub output_schemas: Value,
    pub deadline_ms: u64,
    pub lease_until_ms: u64,
    pub result_channel: &'static str,
    /// The step files folder, its instructions file and the work folder, when the
    /// engine has a `files_dir`.
    pub files: Option<PathBuf>,
    pub instructions: Option<PathBuf>,
    pub work_dir: Option<PathBuf>,
}

/// A report or an abandon that the store took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub key: RunKey,
    pub invocation_id: String,
    pub attempt: u32,
    pub disposition: Disposition,
}

/// A renewal: the new lease end and the deadline it never passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRenewed {
    pub key: RunKey,
    pub lease_until_ms: u64,
    pub deadline_ms: Option<u64>,
}

/// What a claim hands the actor from the package.
struct Wanted {
    outcomes: Vec<String>,
    schemas: Value,
    external: External,
}

/// A held attempt a token names, with what the host needs to check a report.
struct Held {
    attempt: AttemptRef,
    number: u32,
    node_id: String,
    external: External,
    outcomes: Vec<String>,
    schemas: Value,
    deadline_ms: Option<u64>,
}

fn token_invalid(message: impl Into<String>) -> HostError {
    HostError::invalid("step.token-invalid", message)
}

/// The outcomes an action declares; none declared is `["ok"]` (kernel §2).
fn outcomes(action: &Value) -> Vec<String> {
    action
        .get("outcomes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_else(|| vec!["ok".to_string()])
}

/// A fresh owner for one claim, so its token names that claim alone.
fn fresh_owner() -> Result<String, HostError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| HostError::invalid("step.owner-unavailable", format!("random source: {e}")))?;
    Ok(bytes
        .iter()
        .fold(String::from("actor-"), |mut id, b| {
            id.push_str(&format!("{b:02x}"));
            id
        }))
}

impl Engine {
    /// The external step the run is at, if any (spec/host.md §2, Waiting on a step).
    pub async fn external_step(&self, view: &RunView) -> Result<Option<ExternalStep>, HostError> {
        let state: Option<State> = view
            .snapshot
            .as_deref()
            .and_then(|bytes| canonical::decode(bytes).ok());
        let Some(scheduled) = state
            .and_then(|s| s.invocation)
            .filter(|i| i.state == InvocationPhase::Scheduled)
        else {
            return Ok(None);
        };
        let package = self.inner.package(&view.package_digest).await?;
        let Some(action) = package.actions.get(&scheduled.action_id) else {
            return Ok(None);
        };
        if !external::is_external(action) {
            return Ok(None);
        }
        let actor = external::parse(action).ok().and_then(|e| e.actor);
        let invocations = self.list_invocations(&view.key).await?;
        let Some(invocation) = invocations
            .into_iter()
            .find(|i| i.invocation_id == scheduled.invocation_id)
        else {
            return Ok(None);
        };
        let mut step = ExternalStep {
            kind: external::KIND,
            actor,
            state: StepState::Ready,
            attempt: invocation.attempt_number,
            invocation_id: invocation.invocation_id,
            node_id: invocation.node_id,
            action_id: invocation.action_id,
            holder: None,
            deadline_ms: None,
            lease_until_ms: None,
        };
        match invocation.status {
            InvocationStatus::Pending => {}
            InvocationStatus::Running => {
                step.state = StepState::Held;
                step.holder = invocation
                    .holder
                    .as_deref()
                    .and_then(|bytes| canonical::parse(bytes).ok());
                step.deadline_ms = invocation.deadline_ms;
                step.lease_until_ms = invocation.lease_until_ms;
            }
            // Its result is in and not yet applied: the run is about to move on.
            InvocationStatus::Settled => return Ok(None),
        }
        Ok(Some(step))
    }

    /// Whether the run waits for an actor: it is `running` at an external step with no
    /// pending event.
    pub(super) async fn waits_on_actor(&self, view: &RunView) -> Result<bool, HostError> {
        let pending = view.applied_sequence + 1 < view.next_sequence;
        if pending || view.quarantine.is_some() || view.status != "running" {
            return Ok(false);
        }
        Ok(self.external_step(view).await?.is_some())
    }

    /// Claims a ready external step whose action's label is `actor` (both absent counts
    /// as equal): that run's, or with no run the oldest of the tenant. `None` when there
    /// is nothing to claim.
    pub async fn claim_step(
        &self,
        run: Option<&str>,
        actor: Option<&str>,
        holder: Option<Value>,
    ) -> Result<Option<StepClaim>, HostError> {
        let holder = match holder {
            None => None,
            Some(holder) => {
                let bytes = canonical::to_vec(&holder)
                    .map_err(|e| HostError::invalid("holder.invalid", e.to_string()))?;
                if bytes.len() > MAX_HOLDER_BYTES {
                    return Err(HostError::invalid(
                        "holder.invalid",
                        format!(
                            "the holder handle is {} bytes; at most {MAX_HOLDER_BYTES}",
                            bytes.len()
                        ),
                    ));
                }
                Some(bytes)
            }
        };
        for (key, invocation_id) in self.ready_steps(run).await? {
            let Some((invocation, package)) =
                self.inner.invocation_package(&key, &invocation_id).await?
            else {
                continue;
            };
            let Some(action) = package
                .actions
                .get(&invocation.action_id)
                .filter(|a| external::is_external(a))
            else {
                continue;
            };
            let Ok(external) = external::parse(action) else {
                continue;
            };
            if external.actor.as_deref() != actor {
                continue;
            }
            let wanted = Wanted {
                outcomes: outcomes(action),
                schemas: package
                    .nodes
                    .get(&invocation.node_id)
                    .and_then(|n| n.get("outputSchemas"))
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                external,
            };
            if let Some(claim) = self
                .claim_one(&key, &invocation_id, wanted, holder.clone())
                .await?
            {
                return Ok(Some(claim));
            }
        }
        Ok(None)
    }

    /// Pending invocations that may be external steps: the run's, or the tenant's in
    /// `scan_due` order, oldest `notBeforeMs` first.
    async fn ready_steps(&self, run: Option<&str>) -> Result<Vec<(RunKey, String)>, HostError> {
        if let Some(run) = run {
            let key = self.key(run);
            if self.get_run(&key).await?.is_none() {
                return Err(HostError::Store(StoreFailure::new(
                    StoreFailureKind::NotFound,
                    "run.not-found",
                    format!("run {run} does not exist"),
                )));
            }
            let invocations = self.list_invocations(&key).await?;
            return Ok(invocations
                .into_iter()
                .filter(|i| i.status == InvocationStatus::Pending)
                .map(|i| (key.clone(), i.invocation_id))
                .collect());
        }
        let tenant = &self.inner.config.tenant;
        Ok(self
            .inner
            .scan_due()
            .await?
            .into_iter()
            .filter_map(|item| match item {
                DueItem::Outbox { key, invocation_id } if &key.tenant == tenant => {
                    Some((key, invocation_id))
                }
                _ => None,
            })
            .collect())
    }

    /// Gives the step its files and claims it; `None` when another claimer got there
    /// first or the run was quarantined instead.
    async fn claim_one(
        &self,
        key: &RunKey,
        invocation_id: &str,
        wanted: Wanted,
        holder: Option<Vec<u8>>,
    ) -> Result<Option<StepClaim>, HostError> {
        let inner = &self.inner;
        let external = &wanted.external;
        // The files stay; the actor's work outlives this process, so the lock does not.
        let delivery = match inner.deliver(key, invocation_id).await? {
            Ok(delivery) => delivery,
            Err(Undeliverable::Quarantined | Undeliverable::Skip) => return Ok(None),
        };
        let owner = fresh_owner()?;
        let request = ClaimAttempt {
            mutation: mutation(
                "claim_attempt",
                format!("step-claim/{invocation_id}/{owner}"),
                json!([key, invocation_id, owner, external.lease_ms(), holder, external.timeout_ms]),
            )?,
            key: key.clone(),
            invocation_id: invocation_id.to_string(),
            worker: owner,
            lease_duration_ms: external.lease_ms(),
            holder,
            deadline_duration_ms: Some(external.timeout_ms),
        };
        let claim = match retry(true, || inner.store.claim_attempt(request.clone())).await {
            Ok(receipt) => receipt.value,
            Err(f) if matches!(f.kind, StoreFailureKind::Conflict | StoreFailureKind::Stale) => {
                return Ok(None)
            }
            Err(f) => return Err(f.into()),
        };
        let input = canonical::parse(&claim.input)
            .map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
        let files = delivery.files.as_ref();
        Ok(Some(StepClaim {
            token: StepToken::of(&claim.attempt, claim.attempt_number),
            key: key.clone(),
            node_id: claim.node_id,
            action_id: claim.action_id,
            invocation_id: invocation_id.to_string(),
            attempt: claim.attempt_number,
            input,
            outcomes: wanted.outcomes,
            output_schemas: wanted.schemas,
            deadline_ms: claim.deadline_ms.unwrap_or(claim.lease_until_ms),
            lease_until_ms: claim.lease_until_ms,
            result_channel: RESULT_CHANNEL,
            files: files.map(|f| f.dir.clone()),
            instructions: files.and_then(|f| f.instructions.clone()),
            work_dir: delivery.work_dir.clone(),
        }))
    }

    /// The attempt a token names, if it is an attempt of an external step of its run.
    async fn held(&self, token: &str) -> Result<Held, HostError> {
        let token: StepToken = token
            .parse()
            .map_err(|()| token_invalid("not a step token"))?;
        let attempt = token.attempt_ref(&self.inner.config.tenant);
        let unknown = || {
            token_invalid(format!(
                "the token names no attempt of an external step of run {}",
                token.run
            ))
        };
        let view = self.get_run(&attempt.key).await?.ok_or_else(unknown)?;
        let invocation = self
            .list_invocations(&attempt.key)
            .await?
            .into_iter()
            .find(|i| i.invocation_id == attempt.invocation_id && token.attempt <= i.attempt_number)
            .ok_or_else(unknown)?;
        let package = self.inner.package(&view.package_digest).await?;
        let action = package
            .actions
            .get(&invocation.action_id)
            .filter(|a| external::is_external(a))
            .ok_or_else(unknown)?;
        let external = external::parse(action).map_err(|_| unknown())?;
        let schemas = package
            .nodes
            .get(&invocation.node_id)
            .and_then(|n| n.get("outputSchemas"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(Held {
            attempt,
            number: token.attempt,
            outcomes: outcomes(action),
            node_id: invocation.node_id,
            external,
            schemas,
            deadline_ms: invocation.deadline_ms,
        })
    }

    /// Extends the held attempt's lease by `leaseMs` (or `timeoutMs`), never past its
    /// deadline.
    pub async fn renew_step(&self, token: &str) -> Result<StepRenewed, HostError> {
        let held = self.held(token).await?;
        let extend_ms = held.external.lease_ms();
        let request = Heartbeat {
            mutation: mutation(
                "heartbeat",
                self.inner
                    .unique(&format!("step-renew/{}", held.attempt.attempt_id)),
                json!([held.attempt, extend_ms]),
            )?,
            attempt: held.attempt.clone(),
            extend_ms,
        };
        let receipt = retry(true, || self.inner.store.heartbeat(request.clone())).await?;
        Ok(StepRenewed {
            key: held.attempt.key,
            lease_until_ms: receipt.value.lease_until_ms,
            deadline_ms: held.deadline_ms,
        })
    }

    /// Reports the step done with `outcome`. An undeclared outcome, output outside the
    /// canonical domain, a body over 64 KiB, or output that does not fit the outcome's
    /// schema is refused and nothing is recorded.
    pub async fn complete_step(
        &self,
        token: &str,
        outcome: &str,
        output: Value,
    ) -> Result<StepReport, HostError> {
        let held = self.held(token).await?;
        if !held.outcomes.iter().any(|o| o == outcome) {
            return Err(HostError::invalid(
                "step.undeclared-outcome",
                format!(
                    "step {} declares the outcomes {}, not \"{outcome}\"",
                    held.node_id,
                    held.outcomes.join(", ")
                ),
            ));
        }
        canonical::to_vec(&output)
            .map_err(|e| HostError::invalid("step.output-not-canonical", e.to_string()))?;
        if let Some(schema) = held.schemas.get(outcome) {
            runspore_schema::fit(outcome, schema, &output).map_err(HostError::OutputInvalid)?;
        }
        let result = ActivityResult {
            invocation_id: held.attempt.invocation_id.clone(),
            attempt_id: held.attempt.attempt_id.clone(),
            attempt: held.number,
            status: AttemptStatus::Success,
            outcome: Some(outcome.to_string()),
            output,
            error: None,
            retryable: true,
        };
        self.report(held, result).await
    }

    /// Reports the step failed. `retryable` lets the kernel try again up to the
    /// action's `maxAttempts`.
    pub async fn fail_step(
        &self,
        token: &str,
        message: &str,
        code: Option<&str>,
        retryable: bool,
    ) -> Result<StepReport, HostError> {
        let code = code.unwrap_or(STEP_FAILED);
        if !external::is_label(code) {
            return Err(HostError::invalid(
                "step.bad-code",
                format!("the error code \"{code}\" must match [a-z0-9][a-z0-9.-]{{0,63}}"),
            ));
        }
        let held = self.held(token).await?;
        let result = ActivityResult {
            invocation_id: held.attempt.invocation_id.clone(),
            attempt_id: held.attempt.attempt_id.clone(),
            attempt: held.number,
            status: AttemptStatus::Failure,
            outcome: None,
            output: Value::Null,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
                node_id: None,
                details: Value::Null,
            }),
            retryable,
        };
        self.report(held, result).await
    }

    /// `finish_attempt` with an actor's result; one that lost to an expiry, an abandon
    /// or an earlier report is kept as late evidence and refused with `lease.lost`.
    async fn report(&self, held: Held, result: ActivityResult) -> Result<StepReport, HostError> {
        let evidence = evidence(&result)?;
        if evidence.body.len() > MAX_RESULT_BYTES {
            return Err(HostError::invalid(
                "step.output-too-large",
                format!(
                    "the result is {} bytes; at most {MAX_RESULT_BYTES}",
                    evidence.body.len()
                ),
            ));
        }
        let attempt = held.attempt;
        let request = FinishAttempt {
            mutation: mutation(
                "finish_attempt",
                format!("step/{}/{}", attempt.attempt_id, evidence.digest),
                json!([attempt, evidence.digest]),
            )?,
            attempt: attempt.clone(),
            result: evidence.clone(),
        };
        let lost = match retry(true, || self.inner.store.finish_attempt(request.clone())).await {
            Ok(receipt) => {
                return Ok(StepReport {
                    key: attempt.key.clone(),
                    invocation_id: attempt.invocation_id.clone(),
                    attempt: held.number,
                    disposition: receipt.disposition,
                })
            }
            Err(f) if f.kind == StoreFailureKind::Stale => f,
            Err(f) => return Err(f.into()),
        };
        let late = RecordLateEvidence {
            mutation: mutation(
                "record_late_evidence",
                format!("late/{}/{}", attempt.attempt_id, evidence.digest),
                json!([attempt, evidence.digest]),
            )?,
            attempt,
            evidence,
        };
        retry(true, || self.inner.store.record_late_evidence(late.clone())).await?;
        Err(HostError::Store(StoreFailure::new(
            lost.kind,
            "lease.lost",
            "the attempt has already ended (abandoned, expired or reported); \
             the report is kept as late evidence",
        )))
    }

    /// Expires the run's held external attempt at once with reason `abandoned`; the
    /// action's effect then decides.
    pub async fn abandon_step(&self, key: &RunKey) -> Result<StepReport, HostError> {
        let not_held = || {
            HostError::invalid(
                "step.not-held",
                format!("run {} has no held external step", key.run),
            )
        };
        let view = self.get_run(key).await?.ok_or_else(|| {
            HostError::Store(StoreFailure::new(
                StoreFailureKind::NotFound,
                "run.not-found",
                format!("run {} does not exist", key.run),
            ))
        })?;
        let step = self.external_step(&view).await?.ok_or_else(not_held)?;
        if step.state != StepState::Held {
            return Err(not_held());
        }
        let attempt = self
            .list_invocations(key)
            .await?
            .into_iter()
            .find(|i| i.invocation_id == step.invocation_id)
            .and_then(|i| i.attempt)
            .ok_or_else(not_held)?;
        let reason = ExpireReason::Abandoned;
        let request = ExpireAttempt {
            mutation: mutation(
                "expire_attempt",
                format!("abandon/{}", attempt.attempt_id),
                json!([attempt, false, reason]),
            )?,
            attempt: attempt.clone(),
            process_unchecked: false,
            reason,
        };
        match retry(true, || self.inner.store.expire_attempt(request.clone())).await {
            Ok(receipt) => Ok(StepReport {
                key: key.clone(),
                invocation_id: step.invocation_id,
                attempt: step.attempt,
                disposition: receipt.disposition,
            }),
            // It ended meanwhile: reported, expired, or abandoned by another call.
            Err(f) if f.kind == StoreFailureKind::Stale => Err(not_held()),
            Err(f) => Err(f.into()),
        }
    }
}
