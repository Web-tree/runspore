//! Held async attempts (spec/host.md §3, Async commands): handing an attempt to the
//! holder an async start command names, probing a held attempt, and watching it.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use runspore_types::canonical;
use runspore_types::digest::ids;
use runspore_types::store::*;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::{
    activity_result, checked, evidence, is, lock, mutation, retry, visit_of, Engine, Inner,
    Package, PAGE,
};
use crate::activity::{ActivityContext, ActivityOutput};
use crate::command::Program;
use crate::error::HostError;
use crate::external::{self, Async, StepToken};
use crate::holder::{self, Answer, Watched};

/// How often the sweep looks after the held async attempts of the tenant.
const ASYNC_SWEEP_EVERY: Duration = Duration::from_secs(30);
/// How much of a stop's stdout its late evidence keeps: the end.
const STOP_STDOUT: usize = 16 * 1024;

/// A held async attempt, with what its probe and watch need.
pub(super) struct HeldAttempt {
    attempt: AttemptRef,
    number: u32,
    node_id: String,
    action_id: String,
    input: Value,
    params: Option<Value>,
    /// Canonical JSON of the holder handle; `null` when the claim gave none.
    holder: Vec<u8>,
    deadline_ms: u64,
    pub(super) bounds: Async,
    outcomes: Vec<String>,
    visit: u32,
    base_dir: PathBuf,
    /// The run ID of the run's parent, `None` when it has none.
    parent_run: Option<String>,
    package: Arc<Package>,
}

/// What a watch of a held attempt tells while it runs: the message of its latest
/// `needs` line, if any.
pub(super) type Waits = Arc<Mutex<Option<String>>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The outcomes an action declares; none declared is `["ok"]` (kernel §2).
pub(super) fn outcomes(action: &Value) -> Vec<String> {
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

impl Inner {
    /// The held async attempt of `invocation` in `view`'s run: running, with the
    /// deadline a claim outside the worker or `hold_attempt` set, at an action held
    /// outside the worker. `None` for any other invocation.
    pub(super) async fn held_attempt(
        &self,
        view: &RunView,
        invocation: &InvocationView,
    ) -> Option<HeldAttempt> {
        if invocation.status != InvocationStatus::Running {
            return None;
        }
        self.attempt_held(view, invocation).await
    }

    /// The latest attempt of `invocation` if it was held outside the worker, whether it
    /// still runs or has ended: it has the deadline a claim outside the worker or
    /// `hold_attempt` set, at an action held outside the worker.
    pub(super) async fn attempt_held(
        &self,
        view: &RunView,
        invocation: &InvocationView,
    ) -> Option<HeldAttempt> {
        let deadline_ms = invocation.deadline_ms?;
        let attempt = invocation.attempt.clone()?;
        let package = self.package(&view.package_digest).await.ok()?;
        let action = package.actions.get(&invocation.action_id)?;
        let bounds = external::async_of(action).ok()??;
        let input = canonical::parse(&invocation.input).ok()?;
        let params = match invocation.params.as_deref() {
            Some(bytes) => Some(canonical::parse(bytes).ok()?),
            None => None,
        };
        let visit = visit_of(
            &view.key,
            &invocation.node_id,
            &invocation.invocation_id,
            package.max_visits,
        )?;
        Some(HeldAttempt {
            attempt,
            number: invocation.attempt_number,
            node_id: invocation.node_id.clone(),
            action_id: invocation.action_id.clone(),
            input,
            params,
            holder: invocation
                .holder
                .clone()
                .unwrap_or_else(|| b"null".to_vec()),
            deadline_ms,
            outcomes: outcomes(action),
            bounds,
            visit,
            base_dir: view
                .base_dir
                .as_ref()
                .map_or_else(|| self.config.base_dir.clone(), PathBuf::from),
            parent_run: view.parent.as_ref().map(|p| p.run_id.clone()),
            package,
        })
    }

    /// The context a probe or watch of `held` runs in: the try it looks after, its step
    /// files and work folder, and `stop`.
    fn held_context(
        &self,
        held: &HeldAttempt,
        files: &super::Delivery,
        stop: watch::Receiver<bool>,
    ) -> ActivityContext {
        let mut ctx = ActivityContext::new(
            held.attempt.key.clone(),
            held.node_id.clone(),
            held.action_id.clone(),
            held.attempt.invocation_id.clone(),
            held.number,
            held.visit,
            ids::effect_key(&held.attempt.invocation_id),
            held.base_dir.clone(),
            stop,
        );
        if let Some(params) = &held.params {
            ctx.params = params.clone();
        }
        ctx.db = self.config.db_path.clone();
        ctx.parent_run = held.parent_run.clone();
        ctx.files = files.files.clone();
        ctx.work_dir = files.work_dir.clone();
        ctx.guard = self.config.guard.clone();
        ctx.stop_grace = Duration::from_millis(self.config.stop_grace_ms);
        ctx
    }

    /// Records the holder handle an async start command printed (store §2
    /// `hold_attempt`) and starts its watch. Refused when the attempt has already
    /// ended: the work reported, or the attempt expired.
    pub(super) async fn hold(
        this: &Arc<Self>,
        claim: &Claim,
        action: &Value,
        holder: &Value,
    ) -> Result<(), HostError> {
        let Ok(Some(bounds)) = external::async_of(action) else {
            return Ok(());
        };
        let holder =
            canonical::to_vec(holder).map_err(|e| HostError::invalid(e.code(), e.to_string()))?;
        let attempt = &claim.attempt;
        let request = HoldAttempt {
            mutation: mutation(
                "hold_attempt",
                format!("hold/{}", attempt.attempt_id),
                json!([attempt, holder, bounds.lease_ms(), bounds.timeout_ms]),
            )?,
            attempt: attempt.clone(),
            holder,
            lease_duration_ms: bounds.lease_ms(),
            deadline_duration_ms: bounds.timeout_ms,
        };
        match retry(true, || this.store.hold_attempt(request.clone())).await {
            Ok(_) => {}
            Err(f) if f.kind == StoreFailureKind::Stale => return Ok(()),
            Err(f) => return Err(f.into()),
        }
        if bounds.watch.is_none() {
            return Ok(());
        }
        let Some(view) = retry(false, || this.store.get_run(&attempt.key)).await? else {
            return Ok(());
        };
        let invocations = retry(false, || this.store.list_invocations(&attempt.key)).await?;
        let invocation = invocations
            .iter()
            .find(|i| i.invocation_id == attempt.invocation_id);
        if let Some(held) = match invocation {
            Some(invocation) => this.held_attempt(&view, invocation).await,
            None => None,
        } {
            Inner::start_watch(this, held);
        }
        Ok(())
    }

    /// Runs the probe of `held`, when its action declares one; a `dead` answer expires
    /// the attempt at once with reason `probe-dead`.
    pub(super) async fn probe(&self, held: &HeldAttempt) -> Result<Option<Answer>, HostError> {
        let Some(program) = &held.bounds.probe else {
            return Ok(None);
        };
        let Ok(delivery) = self
            .deliver(&held.attempt.key, &held.attempt.invocation_id)
            .await?
        else {
            return Ok(None);
        };
        let ctx = self.held_context(held, &delivery, self.shutdown.subscribe());
        let answer = holder::probe(&ctx, program, &held.input, &held.holder).await;
        drop(delivery);
        if answer == Answer::Dead {
            let reason = ExpireReason::ProbeDead;
            let attempt = &held.attempt;
            let request = ExpireAttempt {
                mutation: mutation(
                    "expire_attempt",
                    format!("probe-dead/{}", attempt.attempt_id),
                    json!([attempt, false, reason]),
                )?,
                attempt: attempt.clone(),
                process_unchecked: false,
                reason,
            };
            match retry(true, || self.store.expire_attempt(request.clone())).await {
                Ok(_) => {}
                // It ended meanwhile: reported, expired or abandoned.
                Err(f) if f.kind == StoreFailureKind::Stale => {}
                Err(f) => return Err(f.into()),
            }
        }
        Ok(Some(answer))
    }

    /// Runs the stop of `held`, an attempt a cancel has expired, once, and keeps its exit
    /// code and stdout as late evidence (spec/host.md §3, Async commands, Stop). Returns
    /// its exit code; whatever it does is otherwise ignored.
    pub(super) async fn stop(&self, held: &HeldAttempt, program: &Program) -> Option<i32> {
        // Without its step files the stop still runs; a `./` program then fails.
        let delivery = match self
            .deliver(&held.attempt.key, &held.attempt.invocation_id)
            .await
        {
            Ok(Ok(delivery)) => delivery,
            _ => super::Delivery::default(),
        };
        let ctx = self.held_context(held, &delivery, self.shutdown.subscribe());
        let (exit, stdout) = holder::stop(&ctx, program, &held.input, &held.holder).await;
        drop(delivery);
        let tail = &stdout[stdout.len().saturating_sub(STOP_STDOUT)..];
        let said = json!({"stop": {"exit": exit, "stdout": String::from_utf8_lossy(tail)}});
        let attempt = &held.attempt;
        if let (Ok(evidence), Ok(mutation)) = (
            evidence(&said),
            mutation(
                "record_late_evidence",
                format!("stop/{}", attempt.attempt_id),
                json!([attempt]),
            ),
        ) {
            let late = RecordLateEvidence {
                mutation,
                attempt: attempt.clone(),
                evidence,
            };
            let _ = retry(true, || self.store.record_late_evidence(late.clone())).await;
        }
        exit
    }

    /// Starts a watch of `held` in the background, unless this engine watches it
    /// already, runs `max_watches`, or is shutting down. Returns whether it started.
    pub(super) fn start_watch(this: &Arc<Self>, held: HeldAttempt) -> bool {
        if held.bounds.watch.is_none() || this.closing.load(Ordering::SeqCst) {
            return false;
        }
        let slot = (held.attempt.key.clone(), held.attempt.attempt_id.clone());
        let waits: Waits = Arc::new(Mutex::new(None));
        {
            let mut watching = lock(&this.watching);
            if watching.contains_key(&slot) || watching.len() >= this.config.max_watches {
                return false;
            }
            watching.insert(slot.clone(), waits.clone());
        }
        let task = this.clone();
        lock(&this.tasks).spawn(async move {
            task.watch(held, waits).await;
            lock(&task.watching).remove(&slot);
            task.settled.notify_waiters();
        });
        true
    }

    /// Runs the watch of `held` until it ends, the attempt stops being held, or the
    /// engine shuts down; records the result it delivers.
    async fn watch(self: &Arc<Self>, held: HeldAttempt, waits: Waits) {
        let Some(program) = held.bounds.watch.clone() else {
            return;
        };
        let Ok(Ok(delivery)) = self
            .deliver(&held.attempt.key, &held.attempt.invocation_id)
            .await
        else {
            return;
        };
        let (stop, stop_rx) = watch::channel(false);
        let ctx = self.held_context(&held, &delivery, stop_rx);
        // The time left before the deadline, by this machine's clock when it agrees
        // with the store's; the check below stops the watch once the attempt expires.
        let whole = Duration::from_millis(held.bounds.timeout_ms);
        let left = held
            .deadline_ms
            .checked_sub(now_ms())
            .map(Duration::from_millis);
        let timeout = left.map_or(whole, |left| left.min(whole));
        let watched = holder::watch(
            &ctx,
            &program,
            &held.input,
            &held.holder,
            timeout,
            &held.outcomes,
            waits,
        );
        tokio::pin!(watched);
        let period = Duration::from_millis(self.config.heartbeat_ms.max(1));
        let mut check = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        let mut shutdown = self.shutdown.subscribe();
        let mut stopping = false;
        let watched = loop {
            tokio::select! {
                watched = &mut watched => break watched,
                _ = check.tick(), if !stopping => {
                    if !self.still_held(&held.attempt).await {
                        stopping = true;
                        stop.send_replace(true);
                    }
                }
                () = async { let _ = shutdown.wait_for(|s| *s).await; }, if !stopping => {
                    stopping = true;
                    stop.send_replace(true);
                }
            }
        };
        drop(delivery);
        let Watched::Result(output) = watched else {
            return;
        };
        let output = checked(output, |outcome| {
            held.package.schema(&held.node_id, outcome).cloned()
        });
        let _ = self.report_held(&held, output).await;
    }

    /// Whether `attempt` is still the running attempt of its invocation.
    async fn still_held(&self, attempt: &AttemptRef) -> bool {
        let Ok(invocations) = retry(false, || self.store.list_invocations(&attempt.key)).await
        else {
            return true;
        };
        invocations.iter().any(|i| {
            i.invocation_id == attempt.invocation_id
                && i.status == InvocationStatus::Running
                && i.attempt.as_ref() == Some(attempt)
        })
    }

    /// `finish_attempt` with a watch's result and the attempt's reference read from the
    /// store; a result that lost to another channel is kept as late evidence.
    async fn report_held(
        &self,
        held: &HeldAttempt,
        output: ActivityOutput,
    ) -> Result<(), HostError> {
        let Some(result) = activity_result(output, &held.attempt, held.number) else {
            return Ok(());
        };
        let evidence = evidence(&result)?;
        let attempt = &held.attempt;
        let request = FinishAttempt {
            mutation: mutation(
                "finish_attempt",
                format!("watch/{}/{}", attempt.attempt_id, evidence.digest),
                json!([attempt, evidence.digest]),
            )?,
            attempt: attempt.clone(),
            result: evidence.clone(),
        };
        match retry(true, || self.store.finish_attempt(request.clone())).await {
            Ok(_) => return Ok(()),
            Err(f) if is(&f, StoreFailureKind::Stale, "lease.lost") => {}
            Err(f) => return Err(f.into()),
        }
        let late = RecordLateEvidence {
            mutation: mutation(
                "record_late_evidence",
                format!("late/{}/{}", attempt.attempt_id, evidence.digest),
                json!([attempt, evidence.digest]),
            )?,
            attempt: attempt.clone(),
            evidence,
        };
        retry(true, || self.store.record_late_evidence(late.clone())).await?;
        Ok(())
    }

    /// Looks after every held async attempt of the tenant, at most once every
    /// `ASYNC_SWEEP_EVERY`, starting with the first call: probes each whose action
    /// declares a probe, and starts a watch for each whose action declares a watch.
    /// Its failures wait for the next sweep.
    pub(super) async fn sweep_async(this: &Arc<Self>) {
        {
            let mut swept = lock(&this.async_swept);
            if swept.is_some_and(|at| at.elapsed() < ASYNC_SWEEP_EVERY) {
                return;
            }
            *swept = Some(Instant::now());
        }
        let _ = Inner::look_after_held(this).await;
    }

    async fn look_after_held(this: &Arc<Self>) -> Result<(), HostError> {
        let tenant = &this.config.tenant;
        let mut cursor = None;
        loop {
            let page = retry(false, || this.store.list_runs(tenant, cursor.clone(), PAGE)).await?;
            for view in &page.items {
                if view.ended() || view.quarantine.is_some() {
                    continue;
                }
                let invocations = retry(false, || this.store.list_invocations(&view.key)).await?;
                for invocation in &invocations {
                    let Some(held) = this.held_attempt(view, invocation).await else {
                        continue;
                    };
                    if this.probe(&held).await? == Some(Answer::Dead) {
                        continue;
                    }
                    Inner::start_watch(this, held);
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}

impl Engine {
    /// Probes the run's held async step, when its action declares a probe (spec/host.md
    /// §2, Client operations); a `dead` answer expires the attempt with reason
    /// `probe-dead`. `None` when nothing was probed.
    pub async fn probe_step(&self, key: &RunKey) -> Result<Option<Answer>, HostError> {
        let Some(view) = self.get_run(key).await? else {
            return Ok(None);
        };
        for invocation in self.list_invocations(key).await? {
            if let Some(held) = self.inner.held_attempt(&view, &invocation).await {
                return self.inner.probe(&held).await;
            }
        }
        Ok(None)
    }

    /// The message of the latest `needs` line a running watch of the run printed: its
    /// holder waits for a person. `None` when no watch of the run says so.
    pub fn holder_waits(&self, key: &RunKey) -> Option<String> {
        lock(&self.inner.watching)
            .iter()
            .filter(|((k, _), _)| k == key)
            .find_map(|(_, waits)| lock(waits).clone())
    }

    /// Whether this engine watches the attempt `invocation_id`/`attempt` of the run and
    /// its watch has not said that the holder waits for a person.
    pub(super) fn watches(&self, key: &RunKey, invocation_id: &str, attempt: u32) -> bool {
        let slot = (key.clone(), ids::attempt(invocation_id, attempt));
        lock(&self.inner.watching)
            .get(&slot)
            .is_some_and(|waits| lock(waits).is_none())
    }
}

/// The token of a claimed attempt, as `RUNSPORE_TOKEN` gives it.
pub(super) fn token(claim: &Claim) -> String {
    StepToken::of(&claim.attempt, claim.attempt_number).to_string()
}
