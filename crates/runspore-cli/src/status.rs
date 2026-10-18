//! The `status` of one run (spec/cli.md, `status`): read once into a typed view that
//! its JSON and its text both render, and from which `continue` says what comes next.

use std::collections::BTreeMap;

use runspore_host::disk::DiskFull;
use runspore_host::needs::{Need, Needs};
use runspore_host::{
    Arrival, Arrow, AsyncStep, Candidate, CutOff, CutOffReason, HeldSlot, StepState,
};
use runspore_types::model::{
    ActivityResult, AttemptStatus, ErrorInfo, InvocationPhase, InvocationState, Node, NodeResult,
    Position, RunResult, State, Workflow, OUTCOME_EXHAUSTED,
};
use runspore_types::store::{
    InvocationStatus, InvocationView, ProcessRecord, RunKey, RunView, STATUS_CANCELLED,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::view::{
    duration_text, parent_json, paused_json, paused_text, reason_text, release_command, tails_text,
    utc_text, CancelJson, Tree,
};

/// The exit code a parked run maps to: quarantine first, then the run status (7 for a
/// cancelled run). A running run parked at an async step waits on an actor or on the
/// work it started; one at a step this spore lacks a capability for waits on a host
/// that has it.
pub fn exit_for(view: &RunView, step: Option<&AsyncStep>) -> u8 {
    if view.quarantine.is_some() {
        return 5;
    }
    match view.status.as_str() {
        "completed" => 0,
        "failed" => 1,
        "waiting" => 3,
        "needs-intervention" => 4,
        "running" if step.is_some() => 6,
        STATUS_CANCELLED => 7,
        _ => 10,
    }
}

/// What `status` reads of a run beside its view, all at one time: the arrow into its
/// position and when it came to stand there, its state, workflow and invocations, what
/// it needs, the latest result of its outstanding invocation, the runs below it, the
/// steps that take workers' slots and how many slots a worker has, the steps that wait
/// for a slot in the order a worker takes them, and the worker's disk.
pub struct Shown {
    pub arrival: Arrival,
    pub state: Option<State>,
    pub workflow: Option<Workflow>,
    pub invocations: Vec<InvocationView>,
    /// What the run needs, with its async step and its cut-off try.
    pub needs: Needs,
    pub last_result: Option<ActivityResult>,
    pub children: Tree,
    pub slots: Vec<HeldSlot>,
    pub slot_limit: usize,
    /// The steps waiting for a worker's free slot, by run and invocation ID, in the
    /// order a worker takes them.
    pub queue: Vec<(RunKey, String)>,
    /// The worker's disk when it is under the floor: no step is taken.
    pub paused: Option<DiskFull>,
    pub now_ms: u64,
}

impl Shown {
    /// The async step the run is at, or the step this spore lacks a capability for.
    pub fn step(&self) -> Option<&AsyncStep> {
        self.needs.step.as_ref()
    }

    /// The runs the run started that have not ended.
    fn unfinished_children(&self) -> impl Iterator<Item = &RunView> {
        self.children.unfinished_below()
    }

    /// The pause, for a run that waits for a worker to take its step.
    fn paused_for(&self) -> Option<&DiskFull> {
        let working = matches!(self.needs.need, Some(Need::Working | Need::Driver));
        self.paused.as_ref().filter(|_| working)
    }

    /// The run's status as one typed view.
    pub fn status<'a>(&'a self, view: &'a RunView) -> Status<'a> {
        Status::new(view, self)
    }

    /// The `status` object of spec/cli.md.
    pub fn json(&self, view: &RunView) -> Value {
        serde_json::to_value(self.status(view)).unwrap_or(Value::Null)
    }

    pub fn text(&self, view: &RunView) -> String {
        self.status(view).text()
    }
}

/// `sinceMs` and `forMs` of a stand that began at `since`, as decimal strings.
fn since(since: Option<u64>, now_ms: u64) -> (Option<String>, Option<String>) {
    since.map_or((None, None), |since| {
        let lasted = now_ms.saturating_sub(since);
        (Some(since.to_string()), Some(lasted.to_string()))
    })
}

/// The `maxAttempts` of an action: its retry policy's (one try without one).
fn max_attempts(workflow: &Workflow, action_id: &str) -> u32 {
    workflow
        .policy(action_id)
        .map_or(1, |policy| policy.retry.max_attempts)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArrowJson<'a> {
    node_id: &'a str,
    outcome: &'a str,
}

impl<'a> ArrowJson<'a> {
    fn of(arrow: Option<&'a Arrow>) -> Option<Self> {
        arrow.map(|a| Self {
            node_id: &a.node_id,
            outcome: &a.outcome,
        })
    }

    /// `, from node on outcome`, the way an arrow reads in text.
    fn text(arrow: Option<&Self>) -> String {
        arrow.map_or_else(String::new, |a| {
            format!(", from {} on {}", a.node_id, a.outcome)
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionJson<'a> {
    #[serde(flatten)]
    at: &'a Position,
    from: Option<ArrowJson<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndJson<'a> {
    node_id: &'a str,
    from: Option<ArrowJson<'a>>,
}

/// The run's outstanding invocation, with its action's `maxAttempts` when the workflow
/// is at hand and the process record of its latest attempt.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InvocationJson<'a> {
    #[serde(flatten)]
    state: &'a InvocationState,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_attempts: Option<u32>,
    process: Option<&'a ProcessRecord>,
}

/// At an await-signal node: the signal, since when, its outcomes and their schemas,
/// and one `spore signal` per outcome.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Waiting<'a> {
    node_id: Option<&'a str>,
    signal: &'a str,
    since_ms: Option<String>,
    for_ms: Option<String>,
    outcomes: Vec<&'a str>,
    output_schemas: BTreeMap<String, Value>,
    commands: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InterventionReason {
    Stuck,
    Unknown,
}

/// At a stuck step, or an unsafe step whose outcome is unknown: why, the step's last
/// error, its attempts, since when, and the `spore resolve` commands.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Intervention<'a> {
    node_id: &'a str,
    reason: InterventionReason,
    invocation_id: &'a str,
    attempt: u32,
    max_attempts: Option<u32>,
    attempt_status: Option<AttemptStatus>,
    error: Option<&'a ErrorInfo>,
    since_ms: Option<String>,
    for_ms: Option<String>,
    outcomes: Vec<String>,
    output_schemas: BTreeMap<String, Value>,
    commands: Vec<String>,
}

/// How a try stands: its program runs, it was cut off, it waits for a slot, or it waits
/// to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum TryState {
    CutOff,
    Started,
    Backoff,
    Queued,
}

#[derive(Serialize)]
struct Place {
    place: usize,
    of: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CutOffJson {
    reason: &'static str,
    message: String,
    commands: Vec<String>,
}

/// At an activity: its attempt, whether a worker or an actor holds it, and since when.
/// A started try runs from its claim, with the worker and the pid of its program, or,
/// cut off, with why nothing works it and the command that runs it again; a queued one
/// waits, from when it was due, for a worker with a free slot, at its place in the
/// queue; one in backoff waits until it is due.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Running<'a> {
    node_id: &'a str,
    action_id: &'a str,
    invocation_id: &'a str,
    attempt: u32,
    max_attempts: Option<u32>,
    claimed: Option<bool>,
    state: TryState,
    worker: Option<&'a str>,
    pid: Option<i32>,
    queue: Option<Place>,
    cut_off: Option<CutOffJson>,
    since_ms: Option<String>,
    for_ms: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlotJson<'a> {
    worker: &'a str,
    run_id: &'a str,
    start_key: &'a str,
    node_id: &'a str,
    attempt: u32,
    pid: Option<i32>,
    since_ms: Option<String>,
    for_ms: Option<String>,
}

/// How many steps a worker runs at once, and each step a worker runs now.
#[derive(Serialize)]
struct Slots<'a> {
    limit: usize,
    held: Vec<SlotJson<'a>>,
}

/// A node's streak, with its node's loop limit for that outcome.
#[derive(Serialize)]
struct StreakJson<'a> {
    outcome: &'a str,
    count: u32,
    limit: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuarantineJson<'a> {
    code: &'a str,
    details: &'a str,
    at_ms: String,
    commands: Vec<String>,
}

/// The async step the run is at, or the step this spore lacks a capability for. Never
/// its input or token. `waits` is what its holder reported while it waits.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StepJson<'a> {
    kind: &'a str,
    actor: Option<&'a str>,
    state: &'static str,
    attempt: u32,
    holder: Option<&'a Value>,
    deadline_ms: Option<String>,
    lease_until_ms: Option<String>,
    missing: Option<&'a str>,
    candidate: Option<CandidateJson<'a>>,
    waits: Option<&'a str>,
}

/// A held attempt's candidate answer: its outcome, when it was stored and how long ago.
/// Never its output.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateJson<'a> {
    outcome: &'a str,
    since_ms: String,
    for_ms: String,
}

impl<'a> CandidateJson<'a> {
    pub fn of(candidate: &'a Candidate, now_ms: u64) -> Self {
        Self {
            outcome: &candidate.outcome,
            since_ms: candidate.at_ms.to_string(),
            for_ms: now_ms.saturating_sub(candidate.at_ms).to_string(),
        }
    }
}

/// The `candidate` line of a step's text, or nothing.
pub fn candidate_text(candidate: Option<&Candidate>, now_ms: u64) -> String {
    candidate.map_or_else(String::new, |c| {
        format!(
            "candidate  {} since {}, for {}\n",
            c.outcome,
            utc_text(c.at_ms),
            duration_text(now_ms.saturating_sub(c.at_ms))
        )
    })
}

impl<'a> StepJson<'a> {
    fn of(step: &'a AsyncStep, now_ms: u64) -> Self {
        Self {
            kind: &step.kind,
            actor: step.actor.as_deref(),
            state: step.state.as_str(),
            attempt: step.attempt,
            holder: step.holder.as_ref(),
            deadline_ms: step.deadline_ms.map(|d| d.to_string()),
            lease_until_ms: step.lease_until_ms.map(|l| l.to_string()),
            missing: step.missing.as_deref(),
            candidate: step
                .candidate
                .as_ref()
                .map(|c| CandidateJson::of(c, now_ms)),
            waits: step.waits.as_deref(),
        }
    }
}

/// The `status` object of spec/cli.md, typed: its JSON is this struct's, and its text
/// reads the same fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status<'a> {
    #[serde(skip)]
    view: &'a RunView,
    #[serde(skip)]
    shown: &'a Shown,
    run_id: &'a str,
    start_key: &'a str,
    status: &'a str,
    needs: Option<Need>,
    revision: String,
    package_digest: &'a str,
    artifact_digest: Option<&'a str>,
    parent: Value,
    position: Option<PositionJson<'a>>,
    invocation: Option<InvocationJson<'a>>,
    waiting: Option<Waiting<'a>>,
    intervention: Option<Intervention<'a>>,
    running: Option<Running<'a>>,
    slots: Slots<'a>,
    streaks: Option<BTreeMap<&'a str, StreakJson<'a>>>,
    result: Option<&'a RunResult>,
    end: Option<EndJson<'a>>,
    nodes: Option<&'a BTreeMap<String, NodeResult>>,
    quarantine: Option<QuarantineJson<'a>>,
    cancel: Option<CancelJson<'a>>,
    step: Option<StepJson<'a>>,
    children: Value,
    paused: Value,
}

impl<'a> Status<'a> {
    fn new(view: &'a RunView, shown: &'a Shown) -> Self {
        let state = shown.state.as_ref();
        let workflow = shown.workflow.as_ref();
        let arrival = &shown.arrival;
        let invocation = state.and_then(|s| s.invocation.as_ref()).map(|i| {
            let process = shown
                .invocations
                .iter()
                .find(|stored| stored.invocation_id == i.invocation_id)
                .and_then(|stored| stored.process.as_ref());
            InvocationJson {
                state: i,
                max_attempts: workflow.map(|w| max_attempts(w, &i.action_id)),
                process,
            }
        });
        let stands = state.map(|s| Stands::new(view, s, shown));
        let detail = |status: &str| stands.as_ref().filter(|_| view.status == status);
        let held = shown.slots.iter().map(|slot| {
            let (since_ms, for_ms) = since(slot.claimed_at_ms, shown.now_ms);
            SlotJson {
                worker: &slot.worker,
                run_id: &slot.key.run,
                start_key: &slot.start_key,
                node_id: &slot.node_id,
                attempt: slot.attempt,
                pid: slot.pid,
                since_ms,
                for_ms,
            }
        });
        Self {
            view,
            shown,
            run_id: &view.key.run,
            start_key: &view.start_key,
            status: &view.status,
            needs: shown.needs.need,
            revision: view.revision.to_string(),
            package_digest: &view.package_digest,
            artifact_digest: view.artifact_digest.as_deref(),
            parent: parent_json(view),
            position: state
                .and_then(|s| s.position.as_ref())
                .map(|at| PositionJson {
                    at,
                    from: ArrowJson::of(arrival.from.as_ref()),
                }),
            invocation,
            waiting: detail("waiting").map(Stands::waiting),
            intervention: detail("needs-intervention").and_then(Stands::intervention),
            running: detail("running").and_then(Stands::running),
            slots: Slots {
                limit: shown.slot_limit,
                held: held.collect(),
            },
            streaks: state.map(|s| streaks(s, workflow)),
            result: state.and_then(|s| s.result.as_ref()),
            end: arrival.end.as_ref().map(|end| EndJson {
                node_id: &end.node_id,
                from: ArrowJson::of(end.from.as_ref()),
            }),
            nodes: state.map(|s| &s.nodes),
            quarantine: view.quarantine.as_ref().map(|q| QuarantineJson {
                code: &q.code,
                details: &q.details,
                at_ms: q.at_ms.to_string(),
                commands: vec![release_command(view)],
            }),
            cancel: view.cancel.as_ref().map(CancelJson::of),
            step: shown.step().map(|s| StepJson::of(s, shown.now_ms)),
            children: shown.children.json(),
            paused: paused_json(shown.paused_for()),
        }
    }
}

/// The state's streaks, each with its node's loop limit for that outcome.
fn streaks<'a>(
    state: &'a State,
    workflow: Option<&'a Workflow>,
) -> BTreeMap<&'a str, StreakJson<'a>> {
    state
        .streaks
        .iter()
        .map(|(node_id, streak)| {
            let limit = workflow
                .and_then(|w| w.nodes.get(node_id)?.loop_limits()?.get(&streak.outcome))
                .copied();
            let entry = StreakJson {
                outcome: &streak.outcome,
                count: streak.count,
                limit,
            };
            (node_id.as_str(), entry)
        })
        .collect()
}

/// How a run that is not terminal stands, read from its state, its workflow and the
/// store: what it waits for, since when, and the commands that move it on.
struct Stands<'a> {
    run_id: &'a str,
    state: &'a State,
    shown: &'a Shown,
    /// The node the run is parked at, from the workflow.
    node: Option<&'a Node>,
}

impl<'a> Stands<'a> {
    fn new(view: &'a RunView, state: &'a State, shown: &'a Shown) -> Self {
        let node_id = state.position.as_ref().map_or("", |p| p.node_id.as_str());
        let node = shown.workflow.as_ref().and_then(|w| w.nodes.get(node_id));
        Self {
            run_id: &view.key.run,
            state,
            shown,
            node,
        }
    }

    fn since(&self, at: Option<u64>) -> (Option<String>, Option<String>) {
        since(at, self.shown.now_ms)
    }

    fn output_schemas(&self) -> BTreeMap<String, Value> {
        let schemas = match self.node {
            Some(
                Node::Activity { output_schemas, .. } | Node::AwaitSignal { output_schemas, .. },
            ) => output_schemas.as_ref(),
            _ => None,
        };
        schemas.cloned().unwrap_or_default()
    }

    /// ` <flag> <json>` when `outcome` has an output schema, which its data must fit.
    fn data_flag(&self, flag: &str, outcome: &str) -> String {
        match self.output_schemas().contains_key(outcome) {
            true => format!(" {flag} <json>"),
            false => String::new(),
        }
    }

    fn waiting(&self) -> Waiting<'a> {
        let (signal, routes) = match self.node {
            Some(Node::AwaitSignal {
                signal, outcomes, ..
            }) => (signal.as_str(), Some(outcomes)),
            _ => ("", None),
        };
        let outcomes: Vec<&str> = routes
            .into_iter()
            .flatten()
            .map(|(outcome, _)| outcome.as_str())
            .filter(|outcome| *outcome != OUTCOME_EXHAUSTED)
            .collect();
        let commands = outcomes
            .iter()
            .map(|outcome| {
                format!(
                    "spore signal {} {signal} --outcome {outcome}{}",
                    self.run_id,
                    self.data_flag("--data", outcome)
                )
            })
            .collect();
        let (since_ms, for_ms) = self.since(self.shown.arrival.since_ms);
        Waiting {
            node_id: self.state.position.as_ref().map(|p| p.node_id.as_str()),
            signal,
            since_ms,
            for_ms,
            outcomes,
            output_schemas: self.output_schemas(),
            commands,
        }
    }

    fn intervention(&self) -> Option<Intervention<'a>> {
        let invocation = self.state.invocation.as_ref()?;
        let reason = match invocation.state {
            InvocationPhase::Stuck => InterventionReason::Stuck,
            InvocationPhase::Unknown => InterventionReason::Unknown,
            InvocationPhase::Scheduled => return None,
        };
        let last = self.shown.last_result.as_ref();
        let workflow = self.shown.workflow.as_ref();
        let outcomes = workflow
            .and_then(|w| w.policy(&invocation.action_id))
            .map_or_else(
                || vec![runspore_types::model::OUTCOME_OK.to_string()],
                |p| p.outcomes,
            );
        let resolve = format!("spore resolve {}", self.run_id);
        let mut commands = vec![format!("{resolve} --retry")];
        for outcome in &outcomes {
            let output = self.data_flag("--output", outcome);
            commands.push(format!("{resolve} --complete {outcome}{output}"));
        }
        commands.push(format!("{resolve} --fail <message>"));
        let (since_ms, for_ms) = self.since(self.shown.arrival.since_ms);
        Some(Intervention {
            node_id: &invocation.node_id,
            reason,
            invocation_id: &invocation.invocation_id,
            attempt: invocation.attempt,
            max_attempts: workflow.map(|w| max_attempts(w, &invocation.action_id)),
            attempt_status: last.map(|r| r.status),
            error: invocation
                .error
                .as_ref()
                .or_else(|| last.and_then(|r| r.error.as_ref())),
            since_ms,
            for_ms,
            outcomes,
            output_schemas: self.output_schemas(),
            commands,
        })
    }

    fn running(&self) -> Option<Running<'a>> {
        let invocation = self.state.invocation.as_ref()?;
        let shown = self.shown;
        let stored = shown
            .invocations
            .iter()
            .find(|i| i.invocation_id == invocation.invocation_id);
        let due = shown.arrival.since_ms.max(stored.map(|i| i.not_before_ms));
        let claimed = stored.map(|i| i.status == InvocationStatus::Running);
        // A worker's try whose program, worker or lease is gone; an async step's hold
        // is the `step`'s to show.
        let cut = shown.needs.cut_off.as_ref();
        let state = match (claimed, due) {
            (Some(true), _) if cut.is_some() => TryState::CutOff,
            (Some(true), _) => TryState::Started,
            (_, Some(due)) if due > shown.now_ms => TryState::Backoff,
            _ => TryState::Queued,
        };
        let started = stored.filter(|_| matches!(state, TryState::Started | TryState::CutOff));
        let cut_off = cut.map(|cut| CutOffJson {
            reason: cut.reason.as_str(),
            message: cut_off_text(cut),
            commands: vec![format!("spore continue {}", self.run_id)],
        });
        // A try claimed before the store recorded claims runs from when it was due.
        let (since_ms, for_ms) = self.since(started.and_then(|i| i.claimed_at_ms).or(due));
        // An actor's hold takes no slot; `step` shows it.
        let slot = shown
            .slots
            .iter()
            .find(|s| s.key.run == self.run_id && s.invocation_id == invocation.invocation_id);
        let place = shown
            .queue
            .iter()
            .position(|(k, i)| k.run == self.run_id && i == &invocation.invocation_id)
            .filter(|_| state == TryState::Queued)
            .map(|n| Place {
                place: n + 1,
                of: shown.queue.len(),
            });
        Some(Running {
            node_id: &invocation.node_id,
            action_id: &invocation.action_id,
            invocation_id: &invocation.invocation_id,
            attempt: invocation.attempt,
            max_attempts: shown
                .workflow
                .as_ref()
                .map(|w| max_attempts(w, &invocation.action_id)),
            claimed,
            state,
            worker: slot.map(|s| s.worker.as_str()),
            pid: started.and_then(|i| i.process.as_ref()).map(|p| p.pgid),
            queue: place,
            cut_off,
            since_ms,
            for_ms,
        })
    }
}

/// Why a cut-off try's program does not run: `pid 31295 is gone`.
fn cut_off_text(cut: &CutOff) -> String {
    match cut.reason {
        CutOffReason::ProcessGone => match cut.pid {
            Some(pid) => format!("pid {pid} is gone"),
            None => "its process is gone".to_string(),
        },
        CutOffReason::WorkerGone => match &cut.worker {
            Some(worker) => format!("worker {worker} is gone"),
            None => "its worker is gone".to_string(),
        },
        CutOffReason::Rebooted => match &cut.host {
            Some(host) => format!("{host} has restarted since it started"),
            None => "its machine has restarted since it started".to_string(),
        },
        CutOffReason::LeaseExpired => match cut.lease_until_ms {
            Some(lease) => format!("its lease ran out at {} with no result", utc_text(lease)),
            None => "its lease ran out with no result".to_string(),
        },
    }
}

/// ` of 3` for a known `maxAttempts`, else nothing.
fn of_text(max_attempts: Option<u32>) -> String {
    max_attempts.map_or_else(String::new, |m| format!(" of {m}"))
}

impl Running<'_> {
    /// `implement attempt 1 of 3 was cut off: pid 31295 is gone`.
    fn cut_off_line(&self) -> String {
        format!(
            "{} attempt {}{} was cut off: {}",
            self.node_id,
            self.attempt,
            of_text(self.max_attempts),
            self.cut_off.as_ref().map_or("", |c| c.message.as_str())
        )
    }

    /// The try, and whether its program runs, it was cut off, it waits for a slot (an
    /// external step: for an actor; a capability-missing one: for a spore that can run
    /// it) or it waits to retry.
    fn text(&self, waits: Option<StepState>) -> String {
        if let (TryState::CutOff, Some(cut)) = (self.state, &self.cut_off) {
            let again = cut.commands.first().map_or("", String::as_str);
            return format!(
                "running    {}; {again} runs it again\n",
                self.cut_off_line()
            );
        }
        let since = since_text(self.since_ms.as_deref(), self.for_ms.as_deref());
        let stands = match self.state {
            TryState::Started => {
                let by = self.worker.map_or_else(String::new, |w| format!(" by {w}"));
                format!("started{by}{since}{}", pid_text(self.pid))
            }
            TryState::Backoff => match self.since_ms.as_deref().and_then(|v| v.parse().ok()) {
                Some(due) => format!("waiting to retry, due {}", utc_text(due)),
                None => "waiting to retry".to_string(),
            },
            _ if waits == Some(StepState::CapabilityMissing) => {
                format!("queued for a spore that can run it{since}")
            }
            _ if waits.is_some() => format!("queued for an actor to claim it{since}"),
            _ => format!(
                "queued for a free worker slot{}{since}",
                place_text(self.queue.as_ref())
            ),
        };
        format!(
            "running    {} attempt {}{}, {stands}\n",
            self.node_id,
            self.attempt,
            of_text(self.max_attempts)
        )
    }
}

/// ` since <time>, for <duration>` from a stand's `sinceMs` and `forMs`.
fn since_text(since_ms: Option<&str>, for_ms: Option<&str>) -> String {
    let ms = |v: Option<&str>| v.and_then(|v| v.parse::<u64>().ok());
    match (ms(since_ms), ms(for_ms)) {
        (Some(since), Some(lasted)) => {
            format!(" since {}, for {}", utc_text(since), duration_text(lasted))
        }
        _ => String::new(),
    }
}

/// One line each, `label` before the first and blanks before the rest.
fn lines_text(label: &str, lines: impl IntoIterator<Item = String>) -> String {
    let mut text = String::new();
    for (i, line) in lines.into_iter().enumerate() {
        let label = if i == 0 { label } else { "" };
        text += &format!("{label:<10} {line}\n");
    }
    text
}

/// An error's code and message, then the ends of its command's streams.
fn error_text(error: &ErrorInfo) -> String {
    let details = json!({"details": error.details});
    format!("error      {}: {}\n", error.code, error.message) + &tails_text(&details, "")
}

/// `, 3rd of 16 waiting`: the step's place in the queue for a worker's slot.
fn place_text(queue: Option<&Place>) -> String {
    queue.map_or_else(String::new, |q| {
        format!(", {} of {} waiting", ordinal(q.place), q.of)
    })
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn pid_text(pid: Option<i32>) -> String {
    pid.map_or_else(String::new, |pid| format!(", pid {pid}"))
}

/// A phase as `status` shows it.
fn phase_text(phase: InvocationPhase) -> &'static str {
    match phase {
        InvocationPhase::Scheduled => "scheduled",
        InvocationPhase::Unknown => "unknown",
        InvocationPhase::Stuck => "stuck",
    }
}

/// A watch's message after a colon, or nothing when it gave none.
fn told(message: &str) -> String {
    match message {
        "" => String::new(),
        message => format!(": {message}"),
    }
}

impl Status<'_> {
    pub fn text(&self) -> String {
        let (view, shown) = (self.view, self.shown);
        let mut text = paused_text(shown.paused_for());
        text += &format!(
            "run        {}\nstart key  {}\n",
            self.run_id, self.start_key
        );
        match &view.quarantine {
            Some(q) => {
                text += &format!(
                    "status     quarantined since {}, for {}: {}: {}\n",
                    utc_text(q.at_ms),
                    duration_text(shown.now_ms.saturating_sub(q.at_ms)),
                    q.code,
                    q.details
                );
                text += &format!("release    {}\n", release_command(view));
            }
            None => text += &format!("status     {}\n", self.status),
        }
        text += &format!(
            "revision   {}\npackage    {}\n",
            self.revision, self.package_digest
        );
        if let Some(artifact) = self.artifact_digest {
            text += &format!("artifact   {artifact}\n");
        }
        if let Some(parent) = &view.parent {
            text += &format!("parent     {} at {}\n", parent.run_id, parent.node_id);
        }
        if let Some(p) = &self.position {
            text += &format!(
                "position   {} ({}{})\n",
                p.at.node_id,
                p.at.activation_id,
                ArrowJson::text(p.from.as_ref())
            );
        }
        if let Some(end) = &self.end {
            text += &format!(
                "end        {}{}\n",
                end.node_id,
                ArrowJson::text(end.from.as_ref())
            );
        }
        if let Some(i) = &self.invocation {
            text += &format!(
                "invocation {} attempt {}{} {}\n",
                i.state.invocation_id,
                i.state.attempt,
                of_text(i.max_attempts),
                phase_text(i.state.state)
            );
            if let Some(error) = &i.state.error {
                text += &error_text(error);
            }
            if let Some(p) = i.process {
                let gone = self
                    .running
                    .as_ref()
                    .and_then(|r| r.cut_off.as_ref())
                    .filter(|c| c.reason == CutOffReason::ProcessGone.as_str())
                    .map_or("", |_| ", gone");
                text += &format!("process    group {} on {}{gone}\n", p.pgid, p.host);
            }
        }
        text += &self.waiting_text();
        text += &self.intervention_text();
        // An external step waits for an actor, and a capability-missing one for a spore
        // that can run it, not for a worker's slot; a quarantined run waits for a release.
        let waits = shown.step().map(|step| step.state);
        if view.quarantine.is_none() {
            if let Some(running) = &self.running {
                text += &running.text(waits);
                if waits.is_none() {
                    text += &self.slots_text();
                }
            }
        }
        for (node, streak) in self.streaks.iter().flatten() {
            let limit = streak
                .limit
                .map_or_else(|| "null".to_string(), |l| l.to_string());
            text += &format!(
                "streak     {node} {} {} of {limit} in a row\n",
                streak.outcome, streak.count
            );
        }
        if let Some(result) = self.result {
            let result = serde_json::to_value(result).unwrap_or(Value::Null);
            text += &format!("result     {result}\n");
            text += &tails_text(&result["error"], "");
        }
        if let Some(cancel) = &view.cancel {
            text += &format!(
                "cancelled  {}{}\n",
                utc_text(cancel.at_ms),
                reason_text(Some(cancel))
            );
        }
        text += &self.step_text();
        if !shown.children.rows.is_empty() {
            text += "children\n";
            text += &shown.children.text(1, shown.now_ms);
            text += "\n";
        }
        text
    }

    /// The signal and since when, each outcome with the schema its data must fit, and
    /// the `spore signal` commands.
    fn waiting_text(&self) -> String {
        let Some(waiting) = &self.waiting else {
            return String::new();
        };
        let header = format!(
            "waiting    for signal {}{}\n",
            waiting.signal,
            since_text(waiting.since_ms.as_deref(), waiting.for_ms.as_deref())
        );
        let outcomes =
            waiting
                .outcomes
                .iter()
                .map(|outcome| match waiting.output_schemas.get(*outcome) {
                    Some(schema) => format!("{outcome}, with data that fits {schema}"),
                    None => outcome.to_string(),
                });
        header
            + &lines_text("outcomes", outcomes)
            + &lines_text("answer", waiting.commands.iter().cloned())
    }

    /// Why the step needs resolving and since when, the last error when it is not the
    /// kept one shown with the invocation, and the `spore resolve` commands.
    fn intervention_text(&self) -> String {
        let Some(intervention) = &self.intervention else {
            return String::new();
        };
        let why = match (intervention.reason, intervention.attempt_status) {
            (InterventionReason::Unknown, Some(AttemptStatus::Expired)) => {
                "lost its last attempt's lease with no result"
            }
            (InterventionReason::Unknown, _) => "has a last attempt whose outcome is unknown",
            _ => "gave up and has no failed arrow",
        };
        let mut text = format!(
            "needs      a person: step {} {why}{}\n",
            intervention.node_id,
            since_text(
                intervention.since_ms.as_deref(),
                intervention.for_ms.as_deref()
            )
        );
        let kept = self
            .invocation
            .as_ref()
            .and_then(|i| i.state.error.as_ref());
        if let Some(error) = intervention.error.filter(|e| Some(*e) != kept) {
            text += &error_text(error);
        }
        text + &lines_text("resolve", intervention.commands.iter().cloned())
    }

    /// Per worker, how many of its slots are in use and the step in each; or that no
    /// worker runs a step.
    fn slots_text(&self) -> String {
        let (limit, held) = (self.slots.limit, &self.slots.held);
        if held.is_empty() {
            return format!("slots      none of {limit} in use: no worker runs a step\n");
        }
        let mut workers: Vec<&str> = Vec::new();
        for slot in held {
            if !workers.contains(&slot.worker) {
                workers.push(slot.worker);
            }
        }
        let mut lines = Vec::new();
        for worker in workers {
            let steps: Vec<&SlotJson> = held.iter().filter(|h| h.worker == worker).collect();
            lines.push(format!("{} of {limit} in use by {worker}", steps.len()));
            for step in steps {
                lines.push(format!(
                    "  {} at {} attempt {}{}{}",
                    step.start_key,
                    step.node_id,
                    step.attempt,
                    pid_text(step.pid),
                    since_text(step.since_ms.as_deref(), step.for_ms.as_deref())
                ));
            }
        }
        lines_text("slots", lines)
    }

    fn step_text(&self) -> String {
        let Some(s) = &self.step else {
            return String::new();
        };
        let actor = s.actor.map_or_else(String::new, |a| format!(" for {a}"));
        let mut text = format!(
            "step       {} {}{actor}, attempt {}",
            s.kind, s.state, s.attempt
        );
        if let Some(deadline) = &s.deadline_ms {
            text += &format!(", deadline {deadline} ms");
        }
        text += "\n";
        if let Some(missing) = s.missing {
            text += &format!("missing    {missing}\n");
        }
        if let Some(message) = s.waits {
            text += &format!("holder     waits{}\n", told(message));
        }
        text + &candidate_text(
            self.shown.step().and_then(|s| s.candidate.as_ref()),
            self.shown.now_ms,
        )
    }
}

/// What `continue` prints once it has worked a run: what the run needs, the commands a
/// person types next, and both as text (spec/cli.md, What a run needs).
pub struct Next {
    pub needs: Option<Need>,
    pub commands: Vec<String>,
    pub text: String,
}

impl Shown {
    pub fn next(&self, view: &RunView) -> Next {
        let s = self.status(view);
        let run = &view.key.run;
        let needs = self.needs.need;
        let node = s.position.as_ref().map_or("-", |p| p.at.node_id.as_str());
        let error = |error: Option<&ErrorInfo>| {
            error.map_or_else(String::new, |e| format!(": {}: {}", e.code, e.message))
        };
        let at = || {
            s.end
                .as_ref()
                .map_or_else(String::new, |end| format!(" at {}", end.node_id))
        };
        let (head, commands) = match needs {
            None if view.status == "completed" => {
                (format!("run {run} completed{}", at()), Vec::new())
            }
            None if view.status == STATUS_CANCELLED => {
                let why = reason_text(view.cancel.as_ref());
                (format!("run {run} was cancelled{why}"), Vec::new())
            }
            None => {
                let failed = match s.result {
                    Some(RunResult::Error(e)) => Some(e),
                    _ => None,
                };
                (
                    format!("run {run} failed{}{}", at(), error(failed)),
                    Vec::new(),
                )
            }
            Some(Need::Quarantined) => {
                let why = view
                    .quarantine
                    .as_ref()
                    .map_or_else(String::new, |q| format!(": {}: {}", q.code, q.details));
                let release = format!("spore release {run}");
                (format!("run {run} is quarantined{why}"), vec![release])
            }
            Some(Need::Signal) => {
                let mut commands = vec![format!("spore step show {run}")];
                let signal = s.waiting.as_ref().map_or("-", |w| w.signal);
                commands.extend(s.waiting.iter().flat_map(|w| w.commands.iter().cloned()));
                (
                    format!("run {run} waits at {node} for signal {signal}"),
                    commands,
                )
            }
            Some(Need::Child) => {
                let signal = s.waiting.as_ref().map_or("-", |w| w.signal);
                let below: Vec<&str> = self
                    .unfinished_children()
                    .map(|v| v.key.run.as_str())
                    .collect();
                let runs = if below.len() == 1 { "run" } else { "runs" };
                (
                    format!(
                        "run {run} waits at {node} for signal {signal} from {runs} it started: {}",
                        below.join(", ")
                    ),
                    Vec::new(),
                )
            }
            Some(Need::Stuck) => {
                let intervention = s.intervention.as_ref();
                let at = intervention.map_or("-", |i| i.node_id);
                let why = match error(intervention.and_then(|i| i.error)) {
                    why if !why.is_empty() => why,
                    _ => ": the outcome of its last attempt is unknown".to_string(),
                };
                let commands = intervention.map_or_else(Vec::new, |i| i.commands.clone());
                (format!("run {run} needs a person at {at}{why}"), commands)
            }
            Some(Need::ExternalReady) => {
                let actor = self.step().and_then(|s| s.actor.as_deref());
                let (who, flag) = match actor {
                    Some(actor) => (format!("actor {actor}"), format!(" --actor {actor}")),
                    None => ("an actor".to_string(), String::new()),
                };
                (
                    format!("run {run} waits at {node} for {who} to claim the step"),
                    vec![format!("spore step claim {run}{flag}")],
                )
            }
            Some(Need::Held) => {
                let step = self.step();
                let who = match step {
                    Some(s) if s.kind == "command" => "the work its async command started",
                    s => s.and_then(|s| s.actor.as_deref()).unwrap_or("an actor"),
                };
                let mut head = format!("run {run} at {node} is held by {who}");
                if let Some(step) = step {
                    head += &format!(", attempt {}", step.attempt);
                    if let Some(holder) = &step.holder {
                        head += &format!(", holder {holder}");
                    }
                    if let Some(deadline) = step.deadline_ms {
                        head += &format!(", deadline {}", utc_text(deadline));
                    }
                    if let Some(message) = &step.waits {
                        head += &format!("; its holder waits{}", told(message));
                    }
                }
                (head, vec![format!("spore step abandon {run}")])
            }
            Some(Need::CapabilityMissing) => {
                let missing = self.step().and_then(|s| s.missing.as_deref());
                let why = missing.map_or_else(String::new, |m| format!(": {m}"));
                (
                    format!("run {run} at {node} needs a spore that can run it{why}"),
                    Vec::new(),
                )
            }
            Some(Need::Driver) => {
                let line = s
                    .running
                    .as_ref()
                    .map_or_else(String::new, Running::cut_off_line);
                (
                    format!("run {run} at {node}: {line}"),
                    vec![format!("spore continue {run}")],
                )
            }
            Some(Need::Working) => (
                format!("run {run} is working at {node}"),
                vec![format!("spore continue {run}")],
            ),
        };
        let mut text = head + "\n";
        for command in &commands {
            text += &format!("  {command}\n");
        }
        Next {
            needs,
            commands,
            text,
        }
    }
}

/// The step of `run` a live worker other than `worker` runs now, if any: `continue`
/// names it instead of waiting on it in silence (spec/cli.md, What a run needs). A
/// worker known to be gone runs nothing; its try waits for its lease to run out.
pub fn elsewhere<'a>(shown: &'a Shown, run: &str, worker: &str) -> Option<&'a HeldSlot> {
    shown
        .slots
        .iter()
        .find(|slot| slot.key.run == run && slot.worker != worker && !slot.worker_gone())
}

/// `fix attempt 1 is running on worker <id> (pid <pid>, for 7m35s); waiting for it to
/// finish`, the pid and the time left out when they are not known.
pub fn elsewhere_text(slot: &HeldSlot, now_ms: u64) -> String {
    let mut about = Vec::new();
    if let Some(pid) = slot.pid {
        about.push(format!("pid {pid}"));
    }
    if let Some(claimed) = slot.claimed_at_ms {
        about.push(format!(
            "for {}",
            duration_text(now_ms.saturating_sub(claimed))
        ));
    }
    let about = match about.is_empty() {
        true => String::new(),
        false => format!(" ({})", about.join(", ")),
    };
    format!(
        "{} attempt {} is running on worker {}{about}; waiting for it to finish",
        slot.node_id, slot.attempt, slot.worker
    )
}

/// The line `continue --wait` prints on stderr before it waits on a step another worker
/// runs, as a progress line (spec/cli.md, Progress).
pub fn elsewhere_progress_text(slot: &HeldSlot, now_ms: u64) -> String {
    let line = elsewhere_text(slot, now_ms);
    format!("{}  {}  {line}", utc_text(now_ms), slot.start_key)
}

/// `elsewhere_progress_text` as a progress object, change `running-elsewhere`.
pub fn elsewhere_progress_json(slot: &HeldSlot, now_ms: u64) -> Value {
    let (_, lasted) = since(slot.claimed_at_ms, now_ms);
    json!({"atMs": now_ms.to_string(), "runId": slot.key.run, "startKey": slot.start_key,
           "change": "running-elsewhere", "nodeId": slot.node_id, "attempt": slot.attempt,
           "worker": slot.worker, "pid": slot.pid, "forMs": lasted})
}
