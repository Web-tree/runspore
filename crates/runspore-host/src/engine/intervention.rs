//! Telling a person (spec/host.md §2): the workflow's `onIntervention` action, run once
//! each time a run enters `needs-intervention` or ends `failed`. Its result never moves
//! the run; its failure is recorded as an `on-intervention.failed` event.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use runspore_kernel::mapping;
use runspore_types::canonical;
use runspore_types::digest::ids;
use runspore_types::model::{
    event_kind, ActivityResult, InvocationPhase, OnInterventionFailed, RunResult, RunStatus, State,
};
use runspore_types::store::*;
use serde_json::{json, Map, Value};
use tokio::sync::watch;

use super::{evidence, lock, retry, Inner, Package, CAPABILITY_MISSING};
use crate::activity::{ActivityContext, ActivityOutput};
use crate::error::HostError;
use crate::external;

/// The roots of `onIntervention.params`: the run's context, without `from`.
const PARAMS_ROOTS: [&str; 3] = ["input", "nodes", "run"];

/// Checks a workflow's `onIntervention`, whose shape the kernel has decoded: it names
/// an action of the workflow that the worker runs, and its params are a mapping over
/// [`PARAMS_ROOTS`].
pub(super) fn check(workflow: &Value) -> Result<(), HostError> {
    let Some(hook) = workflow.get("onIntervention") else {
        return Ok(());
    };
    let invalid = |message: String| HostError::invalid("workflow.invalid", message);
    let id = hook
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(action) = workflow.get("actions").and_then(|a| a.get(id)) else {
        return Err(invalid(format!(
            "onIntervention names action `{id}`, which `actions` does not define"
        )));
    };
    if external::is_external(action) {
        return Err(invalid(format!(
            "onIntervention names action `{id}`, an external action: the worker runs \
             onIntervention, and nobody would claim it"
        )));
    }
    if matches!(external::async_of(action), Ok(Some(_))) {
        return Err(invalid(format!(
            "onIntervention names action `{id}`, an async command: its result would \
             come back to no attempt"
        )));
    }
    if let Some(params) = hook.get("params") {
        mapping::check(params, &PARAMS_ROOTS)
            .map_err(|e| invalid(format!("onIntervention.params: {e}")))?;
    }
    Ok(())
}

/// What the action is told, and where it runs from.
struct Notice {
    key: RunKey,
    /// The revision whose turn moved the run; names the notice.
    revision: u64,
    action_id: String,
    params: Option<Value>,
    status: RunStatus,
    /// The action's input: `{run, node, status, reason, error}`.
    input: Value,
    node: String,
    /// The roots of its params.
    scope: Map<String, Value>,
}

impl Notice {
    /// The notice for a turn that left the run in `state`, after consuming `event`;
    /// `None` unless the run now needs intervention or has failed.
    fn new(
        key: &RunKey,
        revision: u64,
        hook: &Value,
        state: &State,
        event: &StoredEvent,
    ) -> Option<Self> {
        let (node, reason, error) = match state.status {
            RunStatus::NeedsIntervention => {
                let inv = state.invocation.as_ref()?;
                match inv.state {
                    InvocationPhase::Stuck => {
                        (Some(inv.node_id.clone()), "stuck", inv.error.clone())
                    }
                    // The kernel keeps no error for an unknown outcome: the result
                    // that made it unknown holds it, when it has one.
                    _ => {
                        let error = (event.kind == event_kind::ACTIVITY_RESULT)
                            .then(|| canonical::decode::<ActivityResult>(&event.body).ok())
                            .flatten()
                            .and_then(|r| r.error);
                        (Some(inv.node_id.clone()), "unknown", error)
                    }
                }
            }
            RunStatus::Failed => match &state.result {
                Some(RunResult::Error(error)) => {
                    (error.node_id.clone(), "failed", Some(error.clone()))
                }
                _ => (None, "failed", None),
            },
            _ => return None,
        };
        let run = json!({"id": key.run, "tenant": key.tenant});
        let mut scope = Map::new();
        scope.insert("input".into(), state.input.clone());
        scope.insert("nodes".into(), json!(state.nodes));
        scope.insert("run".into(), run.clone());
        Some(Self {
            key: key.clone(),
            revision,
            action_id: hook.get("action")?.as_str()?.to_string(),
            params: hook.get("params").cloned(),
            status: state.status,
            input: json!({
                "run": run,
                "node": node,
                "status": state.status.as_str(),
                "reason": reason,
                "error": error,
            }),
            node: node.unwrap_or_default(),
            scope,
        })
    }

    /// The notice's own invocation ID, also its effect key: the same for every engine
    /// that ever tells about this turn.
    fn invocation_id(&self) -> String {
        let activation = format!("onIntervention/{}", self.revision);
        ids::invocation(&self.key.tenant, &self.key.run, &activation)
    }
}

fn failure(code: &str, message: impl Into<String>, details: Value) -> ActivityOutput {
    ActivityOutput::Failure {
        error: ActivityOutput::error(code, message, details),
        retryable: false,
    }
}

impl Inner {
    /// Starts the workflow's `onIntervention` action in the background when the turn
    /// just committed at `revision` moved the run from `before` into
    /// `needs-intervention` or `failed`. An engine that is stopping starts nothing.
    pub(super) fn tell(
        self: &Arc<Self>,
        turn: &Turn,
        package: &Arc<Package>,
        before: Option<&str>,
        snapshot: &[u8],
    ) {
        let Some(hook) = &package.on_intervention else {
            return;
        };
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        let Ok(state) = canonical::decode::<State>(snapshot) else {
            return;
        };
        if before == Some(state.status.as_str()) {
            return;
        }
        let revision = turn.revision + 1;
        let Some(notice) = Notice::new(&turn.key, revision, hook, &state, &turn.next_event) else {
            return;
        };
        let slot = (notice.key.clone(), notice.invocation_id());
        if !lock(&self.inflight).insert(slot.clone()) {
            return;
        }
        let (task, package) = (self.clone(), package.clone());
        lock(&self.tasks).spawn(async move {
            task.notify(notice, &package).await;
            lock(&task.inflight).remove(&slot);
            task.settled.notify_waiters();
        });
    }

    /// Runs the action once and records its failure.
    async fn notify(self: &Arc<Self>, notice: Notice, package: &Package) {
        let output = self.run_notice(&notice, package).await;
        let error = match output {
            // `check` refuses an async command, so nothing is ever held here.
            ActivityOutput::Success { .. } | ActivityOutput::Held { .. } => return,
            ActivityOutput::Failure { error, .. } | ActivityOutput::Unknown { error } => error,
        };
        let Ok(body) = evidence(&OnInterventionFailed {
            action: notice.action_id.clone(),
            status: notice.status,
            error,
        }) else {
            return;
        };
        let id = format!("on-intervention/{}", notice.revision);
        let _ = self
            .append(
                &notice.key,
                id.clone(),
                id,
                event_kind::ON_INTERVENTION_FAILED,
                body,
            )
            .await;
    }

    async fn run_notice(&self, notice: &Notice, package: &Package) -> ActivityOutput {
        let Some(action) = package.actions.get(&notice.action_id) else {
            return failure("host.action-missing", notice.action_id.clone(), Value::Null);
        };
        let kind = action
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(runner) = self.activities.get(kind).cloned() else {
            let shortfall = self.capabilities.recheck(action, None);
            let message = shortfall.map_or_else(|| kind.to_string(), |s| s.to_string());
            return failure(CAPABILITY_MISSING, message, Value::Null);
        };
        let params = match &notice.params {
            None => Value::Object(Map::new()),
            Some(params) => match mapping::evaluate(params, &notice.scope, &mut 0) {
                Ok(params) => params,
                Err(e) => {
                    let details = e.details(&[json!("params")]);
                    return failure(e.code(), "onIntervention.params", details);
                }
            },
        };
        let (base_dir, parent_run) = match retry(false, || self.store.get_run(&notice.key)).await {
            Ok(Some(view)) => (
                view.base_dir.map(PathBuf::from),
                view.parent.map(|p| p.run_id),
            ),
            _ => (None, None),
        };
        let base_dir = base_dir.unwrap_or_else(|| self.config.base_dir.clone());
        let invocation_id = notice.invocation_id();
        let (stop, stop_rx) = watch::channel(false);
        let mut ctx = ActivityContext::new(
            notice.key.clone(),
            notice.node.clone(),
            notice.action_id.clone(),
            invocation_id.clone(),
            1,
            1,
            ids::effect_key(&invocation_id),
            base_dir,
            stop_rx,
        );
        ctx.params = params;
        ctx.db = self.config.db_path.clone();
        ctx.parent_run = parent_run;
        ctx.guard = self.config.guard.clone();
        ctx.stop_grace = Duration::from_millis(self.config.stop_grace_ms);
        let run = runner.run(&ctx, action, &notice.input);
        tokio::pin!(run);
        let mut shutdown = self.shutdown.subscribe();
        let mut stopping = false;
        loop {
            tokio::select! {
                output = &mut run => return output,
                () = async { let _ = shutdown.wait_for(|s| *s).await; }, if !stopping => {
                    stopping = true;
                    stop.send_replace(true);
                }
            }
        }
    }
}
