//! Rendering of store views as CLI JSON objects and text. Counters and timestamps are
//! decimal strings.

use std::collections::HashMap;

use runspore_host::disk::DiskFull;
use runspore_host::progress::{Change, Happened, Progress};
use runspore_host::Cancelled;
use runspore_host::{Arrival, Arrow, AsyncStep, CutOff, CutOffReason, HeldSlot, StepState};
use runspore_types::model::{InvocationPhase, State, OUTCOME_EXHAUSTED, OUTCOME_OK};
use runspore_types::store::{
    Disposition, EventRecord, InvocationStatus, InvocationView, NeedsPerson, RunCancel, RunKey,
    RunView, STATUS_CANCELLED,
};
use serde_json::{json, Value};

/// The exit code a parked run maps to: quarantine first, then the run status (7 for a
/// cancelled run). A
/// running run parked at an async step waits on an actor or on the work it started; one
/// at a step this spore lacks a capability for waits on a host that has it.
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

/// The `step` object of `status`: the async step the run is at, or the step this spore
/// lacks a capability for, or null. Never its input or token. A needs-person mark says
/// that the holder waits for a person as a watch's `needs` line does, with its reason.
fn step_json(step: Option<&AsyncStep>, now_ms: u64) -> Value {
    step.map_or(Value::Null, |s| {
        let mut json = json!({"kind": s.kind, "actor": s.actor, "state": s.state.as_str(),
               "attempt": s.attempt, "holder": s.holder,
               "deadlineMs": s.deadline_ms.map(|d| d.to_string()),
               "leaseUntilMs": s.lease_until_ms.map(|l| l.to_string()),
               "missing": s.missing,
               "needsPerson": needs_person_json(s.needs_person.as_ref(), now_ms)});
        let message = s
            .needs_person
            .as_ref()
            .map(|m| &m.reason)
            .or(s.waits.as_ref());
        if let Some(message) = message {
            json["waiting"] = json!("person");
            json["message"] = json!(message);
        }
        json
    })
}

/// A needs-person mark as `status`, `list` and `step needs-person` print it, or null.
pub fn needs_person_json(mark: Option<&NeedsPerson>, now_ms: u64) -> Value {
    mark.map_or(Value::Null, |mark| {
        let (since, lasted) = since_json(Some(mark.since_ms), now_ms);
        json!({"reason": mark.reason, "sinceMs": since, "forMs": lasted})
    })
}

/// `needs a person for 58s: <reason>`, the end of a `list` line, or nothing.
fn needs_person_row_text(mark: Option<&NeedsPerson>, now_ms: u64) -> String {
    mark.map_or_else(String::new, |mark| {
        let lasted = duration_text(now_ms.saturating_sub(mark.since_ms));
        format!("  needs a person for {lasted}: {}", mark.reason)
    })
}

/// `since <time> UTC: <reason>`: when and why a step was marked as needing a person.
pub fn needs_person_since_text(mark: &NeedsPerson) -> String {
    format!("since {}: {}", utc_text(mark.since_ms), mark.reason)
}

fn step_text(step: Option<&AsyncStep>, now_ms: u64) -> String {
    let Some(s) = step else {
        return String::new();
    };
    let actor = s
        .actor
        .as_deref()
        .map_or_else(String::new, |a| format!(" for {a}"));
    let mut text = format!(
        "step       {} {}{actor}, attempt {}",
        s.kind,
        s.state.as_str(),
        s.attempt
    );
    if let Some(deadline) = s.deadline_ms {
        text += &format!(", deadline {deadline} ms");
    }
    text += "\n";
    if let Some(missing) = &s.missing {
        text += &format!("missing    {missing}\n");
    }
    match (&s.needs_person, &s.waits) {
        (Some(mark), _) => {
            let lasted = duration_text(now_ms.saturating_sub(mark.since_ms));
            text += &format!(
                "waiting    for a person since {}, for {lasted}: {}\n",
                utc_text(mark.since_ms),
                mark.reason
            );
        }
        (None, Some(message)) => text += &format!("waiting    for a person{}\n", told(message)),
        (None, None) => {}
    }
    text
}

/// A watch's message after a colon, or nothing when it gave none.
fn told(message: &str) -> String {
    match message {
        "" => String::new(),
        message => format!(": {message}"),
    }
}

fn state(view: &RunView) -> Option<State> {
    serde_json::from_slice(view.snapshot.as_deref()?).ok()
}

fn arrow_json(arrow: Option<&Arrow>) -> Value {
    arrow.map_or(
        Value::Null,
        |a| json!({"nodeId": a.node_id, "outcome": a.outcome}),
    )
}

/// The `maxAttempts` of an action of a canonical package: its retry policy's, else 1.
fn max_attempts(package: &Value, action_id: &str) -> Value {
    let declared = package
        .pointer(&format!("/actions/{action_id}/retry/maxAttempts"))
        .filter(|n| n.is_u64());
    declared.cloned().unwrap_or(json!(1))
}

/// The state's `streaks`, each with its node's loop limit for that outcome from the
/// package (null without it).
fn streaks_json(state: &State, package: Option<&Value>) -> Value {
    let streaks = state.streaks.iter().map(|(node_id, streak)| {
        let limit = package
            .and_then(|p| {
                p.get("nodes")?
                    .get(node_id)?
                    .get("loopLimits")?
                    .get(&streak.outcome)
            })
            .cloned()
            .unwrap_or(Value::Null);
        let entry = json!({"outcome": streak.outcome, "count": streak.count, "limit": limit});
        (node_id.clone(), entry)
    });
    Value::Object(streaks.collect())
}

/// What `status` shows of a run beside its view: the arrow into its position and when
/// it came to stand there, its package as JSON, its invocations and external step, the
/// payload of the latest `activity.result` of its outstanding invocation, the runs
/// below it, the steps that take workers' slots and how many slots a worker has, the
/// steps that wait for a slot in the order a worker takes them, and the time the
/// command read it all.
pub struct Shown {
    pub arrival: Arrival,
    pub package: Option<Value>,
    pub invocations: Vec<InvocationView>,
    pub step: Option<AsyncStep>,
    pub last_result: Option<Value>,
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
    /// The `status` object of spec/cli.md.
    pub fn json(&self, view: &RunView) -> Value {
        let mut json = status_json(view, self);
        json["step"] = step_json(self.step.as_ref(), self.now_ms);
        json["children"] = self.children.json(self.now_ms);
        json["paused"] = paused_json(self.paused_for(view));
        json
    }

    pub fn text(&self, view: &RunView) -> String {
        let mut text = paused_text(self.paused_for(view))
            + &status_text(view, self)
            + &step_text(self.step.as_ref(), self.now_ms);
        if !self.children.rows.is_empty() {
            text += "children\n";
            text += &self.children.text(1, self.now_ms);
            text += "\n";
        }
        text
    }
}

impl Shown {
    /// What the run needs, `child` while it waits on a run it started, `driver` while
    /// its try is cut off.
    pub fn needs(&self, view: &RunView) -> Option<&'static str> {
        let child_unfinished = self.unfinished_children().next().is_some();
        let needs = needs_below(view, self.step.as_ref(), child_unfinished);
        needs_driver(needs, self.cut_off(view).as_ref())
    }

    /// The run's claimed try when nothing works it any more; never at an async step.
    fn cut_off(&self, view: &RunView) -> Option<CutOff> {
        match self.step {
            Some(_) => None,
            None => cut_off_of(view, &self.invocations, self.now_ms),
        }
    }

    /// The runs the run started that have not ended.
    fn unfinished_children(&self) -> impl Iterator<Item = &RunView> {
        self.children
            .each()
            .filter(|(v, r)| r.depth == 0 && !v.ended())
            .map(|(v, _)| v)
    }

    /// The pause, for a run that waits for a worker to take its step.
    fn paused_for(&self, view: &RunView) -> Option<&DiskFull> {
        let working = needs(view, self.step.as_ref()) == Some("working");
        self.paused.as_ref().filter(|_| working)
    }
}

/// The `paused` object of `status` and `list` while the worker's disk is under its
/// floor, else null (spec/cli.md, A full disk).
pub fn paused_json(paused: Option<&DiskFull>) -> Value {
    paused.map_or(Value::Null, |full| {
        json!({"reason": "disk-full", "message": full.to_string(),
               "freeBytes": full.free.to_string(), "floorBytes": full.floor.to_string(),
               "mount": full.mount})
    })
}

/// `paused: disk full (312 MiB free on /)` as a line, or nothing.
pub fn paused_text(paused: Option<&DiskFull>) -> String {
    paused.map_or_else(String::new, |full| format!("paused: {full}\n"))
}

/// What `continue` prints once it has worked a run: what the run needs, the commands a
/// person types next, and both as text (spec/cli.md, What a run needs).
pub struct Next {
    pub needs: Option<&'static str>,
    pub commands: Vec<String>,
    pub text: String,
}

impl Shown {
    pub fn next(&self, view: &RunView) -> Next {
        let s = status_json(view, self);
        let run = &view.key.run;
        let needs = self.needs(view);
        let listed = |detail: &Value| -> Vec<String> {
            let commands = detail["commands"].as_array().into_iter().flatten();
            commands
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        };
        let text = |value: &Value| value.as_str().unwrap_or("-").to_string();
        let node = text(&s["position"]["nodeId"]);
        let error = |error: &Value| match error["code"].as_str() {
            Some(code) => format!(": {code}: {}", text(&error["message"])),
            None => String::new(),
        };
        let (head, commands) = match needs {
            None if view.status == "completed" => {
                let at = s["end"]["nodeId"]
                    .as_str()
                    .map_or_else(String::new, |n| format!(" at {n}"));
                (format!("run {run} completed{at}"), Vec::new())
            }
            None if view.status == STATUS_CANCELLED => {
                let why = reason_text(view.cancel.as_ref());
                (format!("run {run} was cancelled{why}"), Vec::new())
            }
            None => {
                let at = s["end"]["nodeId"]
                    .as_str()
                    .map_or_else(String::new, |n| format!(" at {n}"));
                let why = error(&s["result"]["error"]);
                (format!("run {run} failed{at}{why}"), Vec::new())
            }
            Some("quarantined") => {
                let why = view
                    .quarantine
                    .as_ref()
                    .map_or_else(String::new, |q| format!(": {}: {}", q.code, q.details));
                let release = format!("spore release {run}");
                (format!("run {run} is quarantined{why}"), vec![release])
            }
            Some("signal") => {
                let waiting = &s["waiting"];
                let mut commands = vec![format!("spore step show {run}")];
                commands.extend(listed(waiting));
                let signal = text(&waiting["signal"]);
                (
                    format!("run {run} waits at {node} for signal {signal}"),
                    commands,
                )
            }
            Some("child") => {
                let signal = text(&s["waiting"]["signal"]);
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
            Some("stuck") => {
                let intervention = &s["intervention"];
                let at = text(&intervention["nodeId"]);
                let why = match error(&intervention["error"]) {
                    why if !why.is_empty() => why,
                    _ => ": the outcome of its last attempt is unknown".to_string(),
                };
                (
                    format!("run {run} needs a person at {at}{why}"),
                    listed(intervention),
                )
            }
            Some("external-ready") => {
                let actor = self.step.as_ref().and_then(|s| s.actor.as_deref());
                let (who, flag) = match actor {
                    Some(actor) => (format!("actor {actor}"), format!(" --actor {actor}")),
                    None => ("an actor".to_string(), String::new()),
                };
                (
                    format!("run {run} waits at {node} for {who} to claim the step"),
                    vec![format!("spore step claim {run}{flag}")],
                )
            }
            Some("held") => {
                let step = self.step.as_ref();
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
                    match (&step.needs_person, &step.waits) {
                        (Some(mark), _) => {
                            head += &format!(
                                "; it waits for a person {}",
                                needs_person_since_text(mark)
                            );
                        }
                        (None, Some(message)) => {
                            head += &format!("; it waits for a person{}", told(message));
                        }
                        (None, None) => {}
                    }
                }
                (head, vec![format!("spore step abandon {run}")])
            }
            Some("capability-missing") => {
                let missing = self.step.as_ref().and_then(|s| s.missing.as_deref());
                let why = missing.map_or_else(String::new, |m| format!(": {m}"));
                (
                    format!("run {run} at {node} needs a spore that can run it{why}"),
                    Vec::new(),
                )
            }
            Some("driver") => (
                format!("run {run} at {node}: {}", cut_off_line(&s["running"])),
                vec![format!("spore continue {run}")],
            ),
            Some(_) => (
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
    let (_, lasted) = since_json(slot.claimed_at_ms, now_ms);
    json!({"atMs": now_ms.to_string(), "runId": slot.key.run, "startKey": slot.start_key,
           "change": "running-elsewhere", "nodeId": slot.node_id, "attempt": slot.attempt,
           "worker": slot.worker, "pid": slot.pid, "forMs": lasted})
}

/// Runs as a tree, parents first, each unfinished one with its node and since when it
/// has stood there (spec/cli.md, `list`); or, `flat`, the unfinished runs alone, newest
/// first (spec/cli.md, What a run needs).
pub struct Tree {
    pub runs: Vec<RunView>,
    pub rows: Vec<Row>,
    pub flat: bool,
}

/// One run of a `list` or `status` tree: how deep it sits under the runs that started
/// it, while it is unfinished its node and since when it has been there, its workflow,
/// what it needs and the actor of its external step.
pub struct Row {
    /// Index into `runs`.
    pub index: usize,
    pub depth: usize,
    pub node_id: Option<String>,
    pub since_ms: Option<u64>,
    pub workflow: Option<String>,
    pub needs: Option<&'static str>,
    pub actor: Option<String>,
    /// The needs-person mark of the async step the run is at.
    pub needs_person: Option<NeedsPerson>,
}

impl Tree {
    fn each(&self) -> impl Iterator<Item = (&RunView, &Row)> {
        self.rows.iter().map(|row| (&self.runs[row.index], row))
    }

    /// The rows, with the time `now_ms` that the marks' `forMs` count up to.
    pub fn json(&self, now_ms: u64) -> Value {
        Value::Array(self.each().map(|(v, r)| tree_json(v, r, now_ms)).collect())
    }

    /// One line per run, `indent` levels in, with times up to `now_ms`.
    pub fn text(&self, indent: usize, now_ms: u64) -> String {
        if self.flat {
            return self.flat_text(now_ms);
        }
        let width = self
            .each()
            .map(|(v, r)| 2 * (r.depth + indent) + v.key.run.len())
            .max()
            .unwrap_or(0);
        let lines: Vec<String> = self
            .each()
            .map(|(v, r)| tree_text(v, r, indent, width, now_ms))
            .collect();
        lines.join("\n")
    }

    /// One line per unfinished run: the run, its workflow, its node, what it needs, how
    /// long ago it last changed, its start key and the actor of its external step.
    fn flat_text(&self, now_ms: u64) -> String {
        let width = |f: &dyn Fn(&RunView, &Row) -> usize| {
            self.each().map(|(v, r)| f(v, r)).max().unwrap_or(0)
        };
        let runs = width(&|v, _| v.key.run.len());
        let workflows = width(&|_, r| r.workflow.as_deref().map_or(1, str::len));
        let nodes = width(&|_, r| r.node_id.as_deref().map_or(1, str::len));
        let lines: Vec<String> = self
            .each()
            .map(|(v, r)| {
                let since = r.since_ms.map_or_else(
                    || "-".to_string(),
                    |s| duration_text(now_ms.saturating_sub(s)),
                );
                let actor = r
                    .actor
                    .as_deref()
                    .map_or_else(String::new, |a| format!("  actor {a}"));
                format!(
                    "{:<runs$}  {:<workflows$}  {:<nodes$}  {:<14} {since:>7}  {}{actor}{}{}",
                    v.key.run,
                    r.workflow.as_deref().unwrap_or("-"),
                    r.node_id.as_deref().unwrap_or("-"),
                    r.needs.unwrap_or("-"),
                    v.start_key,
                    needs_person_row_text(r.needs_person.as_ref(), now_ms),
                    quarantine_row_text(v)
                )
            })
            .collect();
        lines.join("\n")
    }
}

/// What an unfinished run needs from someone (spec/cli.md, What a run needs); `None`
/// once it has finished. `step` is the async step it is at, if any.
pub fn needs(view: &RunView, step: Option<&AsyncStep>) -> Option<&'static str> {
    if view.quarantine.is_some() {
        return Some("quarantined");
    }
    let pending = view.applied_sequence + 1 < view.next_sequence;
    match view.status.as_str() {
        "completed" | "failed" | STATUS_CANCELLED => None,
        _ if pending => Some("working"),
        "waiting" => Some("signal"),
        "needs-intervention" => Some("stuck"),
        _ => Some(match step.map(|s| s.state) {
            Some(StepState::Ready) => "external-ready",
            Some(StepState::Held) => "held",
            Some(StepState::CapabilityMissing) => "capability-missing",
            None => "working",
        }),
    }
}

/// The try the run is at when it is claimed and nothing works it any more (spec/cli.md,
/// What a run needs): `invocations` are the run's.
pub fn cut_off_of(view: &RunView, invocations: &[InvocationView], now_ms: u64) -> Option<CutOff> {
    let invocation = state(view)?.invocation?;
    let stored = invocations
        .iter()
        .find(|i| i.invocation_id == invocation.invocation_id)?;
    runspore_host::cut_off(stored, now_ms)
}

/// `driver` for a `working` run whose try is cut off: no worker will report it, and it
/// waits for a driver to run it again (spec/cli.md, What a run needs).
pub fn needs_driver(needs: Option<&'static str>, cut_off: Option<&CutOff>) -> Option<&'static str> {
    match needs {
        Some("working") if cut_off.is_some() => Some("driver"),
        needs => needs,
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

/// `implement attempt 1 of 3 was cut off: pid 31295 is gone`, from `running`.
fn cut_off_line(running: &Value) -> String {
    let of = running["maxAttempts"]
        .as_u64()
        .map_or_else(String::new, |m| format!(" of {m}"));
    format!(
        "{} attempt {}{of} was cut off: {}",
        running["nodeId"].as_str().unwrap_or_default(),
        running["attempt"],
        running["cutOff"]["message"].as_str().unwrap_or_default()
    )
}

/// `needs`, but `child` for a run that waits for a signal while a run it started has
/// not ended: that run, or one below it, sends the signal, so the wait is no person's
/// decision (spec/cli.md, What a run needs).
pub fn needs_below(
    view: &RunView,
    step: Option<&AsyncStep>,
    child_unfinished: bool,
) -> Option<&'static str> {
    match needs(view, step) {
        Some("signal") if child_unfinished => Some("child"),
        needs => needs,
    }
}

/// The `status` object of spec/cli.md but `step`, plus `startKey` and the latest result
/// per node. With the run's package, the invocation carries its action's `maxAttempts`;
/// it shows the process record of its latest attempt.
fn status_json(view: &RunView, shown: &Shown) -> Value {
    let (arrival, package) = (&shown.arrival, shown.package.as_ref());
    let state = state(view);
    let field = |f: fn(&State) -> Value| state.as_ref().map_or(Value::Null, f);
    let mut position = field(|s| json!(s.position));
    if let Some(position) = position.as_object_mut() {
        position.insert("from".into(), arrow_json(arrival.from.as_ref()));
    }
    let mut invocation = field(|s| json!(s.invocation));
    if let (Some(package), Some(i)) = (package, invocation.as_object_mut()) {
        let action_id = i["actionId"].as_str().unwrap_or_default().to_string();
        i.insert("maxAttempts".into(), max_attempts(package, &action_id));
    }
    if let Value::Object(fields) = &mut invocation {
        let process = shown
            .invocations
            .iter()
            .find(|i| fields.get("invocationId").and_then(Value::as_str) == Some(&i.invocation_id))
            .and_then(|i| i.process.as_ref());
        fields.insert("process".into(), json!(process));
    }
    let stands = state.as_ref().map(|s| Stands::new(view, s, shown));
    let detail = |status: &str| stands.as_ref().filter(|_| view.status == status);
    json!({
        "runId": view.key.run,
        "startKey": view.start_key,
        "status": view.status,
        "revision": view.revision.to_string(),
        "packageDigest": view.package_digest,
        "artifactDigest": view.artifact_digest,
        "parent": parent_json(view),
        "position": position,
        "invocation": invocation,
        "waiting": detail("waiting").map_or(Value::Null, Stands::waiting),
        "intervention": detail("needs-intervention").map_or(Value::Null, Stands::intervention),
        "running": detail("running").map_or(Value::Null, Stands::running),
        "slots": slots_json(shown),
        "streaks": state.as_ref().map_or(Value::Null, |s| streaks_json(s, package)),
        "result": field(|s| json!(s.result)),
        "end": arrival.end.as_ref().map(|end| json!({
            "nodeId": end.node_id, "from": arrow_json(end.from.as_ref())})),
        "nodes": field(|s| json!(s.nodes)),
        "quarantine": view.quarantine.as_ref().map(|q| json!({
            "code": q.code, "details": q.details, "atMs": q.at_ms.to_string(),
            "commands": [release_command(view)]})),
        "cancel": view.cancel.as_ref().map(cancel_json),
    })
}

/// The command that lifts a run's quarantine.
fn release_command(view: &RunView) -> String {
    format!("spore release {}", view.key.run)
}

/// The status a person reads: `quarantined` while the run is, whatever its kernel
/// status, which `--json` keeps in `status`.
fn status_shown(view: &RunView) -> &str {
    if view.quarantine.is_some() {
        "quarantined"
    } else {
        &view.status
    }
}

/// What a `list` row shows beside a quarantined run: its code, the first line of its
/// details and the release; nothing for a run that is not quarantined.
fn quarantine_row_text(view: &RunView) -> String {
    view.quarantine.as_ref().map_or_else(String::new, |q| {
        let details = q.details.lines().next().unwrap_or_default();
        format!("  {}: {details}  {}", q.code, release_command(view))
    })
}

/// `{"reason", "at"}` of a cancel.
fn cancel_json(cancel: &RunCancel) -> Value {
    json!({"reason": cancel.reason, "at": cancel.at_ms.to_string()})
}

/// `: <reason>` of a cancel that was given one, else nothing.
fn reason_text(cancel: Option<&RunCancel>) -> String {
    cancel
        .and_then(|c| c.reason.as_deref())
        .map_or_else(String::new, |r| format!(": {r}"))
}

/// What `spore cancel` prints as JSON (spec/cli.md, Cancelling a run).
pub fn cancelled_json(cancelled: &Cancelled, request_id: &str) -> Value {
    let stops: Vec<Value> = cancelled
        .stops
        .iter()
        .map(|s| json!({"invocationId": s.invocation_id, "attempt": s.attempt, "exit": s.exit}))
        .collect();
    let disposition = match cancelled.disposition {
        Disposition::Applied => "applied",
        Disposition::Duplicate => "duplicate",
    };
    json!({
        "runId": cancelled.key.run, "requestId": request_id, "disposition": disposition,
        "status": cancelled.status, "cancel": cancelled.cancel.as_ref().map(cancel_json),
        "stops": stops,
    })
}

/// What `spore cancel` prints as text: that the run was cancelled and each stop that
/// ran, or that nothing changed.
pub fn cancelled_text(cancelled: &Cancelled, request_id: &str) -> String {
    let run = &cancelled.key.run;
    if cancelled.disposition == Disposition::Duplicate {
        let had = match cancelled.status.as_str() {
            STATUS_CANCELLED => "been cancelled",
            status => status,
        };
        return format!("run {run} had already {had}; nothing changed");
    }
    let mut text = format!("run {run} cancelled (request ID {request_id})");
    for stop in &cancelled.stops {
        let ended = stop.exit.map_or_else(
            || "did not exit by itself".to_string(),
            |code| format!("exited {code}"),
        );
        text += &format!(
            "\nstop of {} attempt {} {ended}",
            stop.invocation_id, stop.attempt
        );
    }
    text
}

/// The line `events` ends with for a cancelled run: its cancel, a store record.
pub fn cancel_line(cancel: &RunCancel) -> String {
    let head = format!(
        "{:>4}  {}  {:<22} {:<10}",
        "-",
        utc_text(cancel.at_ms),
        "run.cancelled",
        "store"
    );
    [cancel.reason.as_deref(), Some(cancel.request_id.as_str())]
        .into_iter()
        .flatten()
        .fold(head, |line, part| format!("{line}  {part}"))
}

/// How a run that is not terminal stands, read from its state, its package and the
/// store: what it waits for, since when, and the commands that move it on.
struct Stands<'a> {
    run_id: &'a str,
    state: &'a State,
    shown: &'a Shown,
    /// The node the run is parked at, from the package.
    node: &'a Value,
}

impl<'a> Stands<'a> {
    fn new(view: &'a RunView, state: &'a State, shown: &'a Shown) -> Self {
        let node_id = state.position.as_ref().map_or("", |p| p.node_id.as_str());
        let node = shown
            .package
            .as_ref()
            .and_then(|p| p.get("nodes")?.get(node_id))
            .unwrap_or(&Value::Null);
        Self {
            run_id: &view.key.run,
            state,
            shown,
            node,
        }
    }

    fn since(&self, since: Option<u64>) -> (Value, Value) {
        since_json(since, self.shown.now_ms)
    }

    fn output_schemas(&self) -> Value {
        self.node
            .get("outputSchemas")
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    /// ` <flag> <json>` when `outcome` has an output schema, which its data must fit.
    fn data_flag(&self, flag: &str, outcome: &str) -> String {
        match self.node.get("outputSchemas").and_then(|s| s.get(outcome)) {
            Some(_) => format!(" {flag} <json>"),
            None => String::new(),
        }
    }

    /// At an await-signal node: the signal, since when, its outcomes and their schemas,
    /// and one `spore signal` per outcome.
    fn waiting(&self) -> Value {
        let signal = self.node["signal"].as_str().unwrap_or_default();
        let outcomes: Vec<&str> = self.node["outcomes"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(outcome, _)| outcome.as_str())
            .filter(|outcome| *outcome != OUTCOME_EXHAUSTED)
            .collect();
        let commands: Vec<String> = outcomes
            .iter()
            .map(|outcome| {
                format!(
                    "spore signal {} {signal} --outcome {outcome}{}",
                    self.run_id,
                    self.data_flag("--data", outcome)
                )
            })
            .collect();
        let (since, lasted) = self.since(self.shown.arrival.since_ms);
        json!({"nodeId": self.state.position.as_ref().map(|p| &p.node_id),
               "signal": signal, "sinceMs": since, "forMs": lasted,
               "outcomes": outcomes, "outputSchemas": self.output_schemas(),
               "commands": commands})
    }

    /// At a stuck step, or an unsafe step whose outcome is unknown: why, the step's last
    /// error, its attempts, since when, and the `spore resolve` commands.
    fn intervention(&self) -> Value {
        let Some(invocation) = &self.state.invocation else {
            return Value::Null;
        };
        let reason = match invocation.state {
            InvocationPhase::Stuck => "stuck",
            InvocationPhase::Unknown => "unknown",
            InvocationPhase::Scheduled => return Value::Null,
        };
        let last = self.shown.last_result.as_ref();
        let error = invocation.error.as_ref().map_or_else(
            || last.map_or(Value::Null, |r| r["error"].clone()),
            |kept| json!(kept),
        );
        let package = self.shown.package.as_ref();
        let outcomes = package
            .and_then(|p| p.pointer(&format!("/actions/{}/outcomes", invocation.action_id)))
            .cloned()
            .unwrap_or_else(|| json!([OUTCOME_OK]));
        let resolve = format!("spore resolve {}", self.run_id);
        let mut commands = vec![format!("{resolve} --retry")];
        for outcome in outcomes
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let output = self.data_flag("--output", outcome);
            commands.push(format!("{resolve} --complete {outcome}{output}"));
        }
        commands.push(format!("{resolve} --fail <message>"));
        let (since, lasted) = self.since(self.shown.arrival.since_ms);
        json!({"nodeId": invocation.node_id, "reason": reason,
               "invocationId": invocation.invocation_id, "attempt": invocation.attempt,
               "maxAttempts": package.map(|p| max_attempts(p, &invocation.action_id)),
               "attemptStatus": last.map(|r| r["status"].clone()),
               "error": error, "sinceMs": since, "forMs": lasted,
               "outcomes": outcomes, "outputSchemas": self.output_schemas(),
               "commands": commands})
    }

    /// At an activity: its attempt, whether a worker or an actor holds it, and since
    /// when. A started try runs from its claim, with the worker and the pid of its
    /// program, or, cut off, with why nothing works it and the command that runs it
    /// again; a queued one waits, from when it was due, for a worker with a free slot,
    /// at its place in the queue; one in backoff waits until it is due.
    fn running(&self) -> Value {
        let Some(invocation) = &self.state.invocation else {
            return Value::Null;
        };
        let stored = self
            .shown
            .invocations
            .iter()
            .find(|i| i.invocation_id == invocation.invocation_id);
        let due = self
            .shown
            .arrival
            .since_ms
            .max(stored.map(|i| i.not_before_ms));
        let claimed = stored.map(|i| i.status == InvocationStatus::Running);
        // A worker's try whose program, worker or lease is gone; an async step's hold
        // is the `step`'s to show.
        let cut = stored
            .filter(|_| self.shown.step.is_none())
            .and_then(|i| runspore_host::cut_off(i, self.shown.now_ms));
        let state = match (claimed, due) {
            (Some(true), _) if cut.is_some() => "cut-off",
            (Some(true), _) => "started",
            (_, Some(due)) if due > self.shown.now_ms => "backoff",
            _ => "queued",
        };
        let started = stored.filter(|_| matches!(state, "started" | "cut-off"));
        let cut_off = cut.as_ref().map_or(Value::Null, |cut| {
            json!({"reason": cut.reason.as_str(), "message": cut_off_text(cut),
                   "commands": [format!("spore continue {}", self.run_id)]})
        });
        // A try claimed before the store recorded claims runs from when it was due.
        let (since, lasted) = self.since(started.and_then(|i| i.claimed_at_ms).or(due));
        // An actor's hold takes no slot; `step` shows it.
        let slot = self
            .shown
            .slots
            .iter()
            .find(|s| s.key.run == self.run_id && s.invocation_id == invocation.invocation_id);
        let pid = started.and_then(|i| i.process.as_ref()).map(|p| p.pgid);
        let queue = &self.shown.queue;
        let place = queue
            .iter()
            .position(|(k, i)| k.run == self.run_id && i == &invocation.invocation_id)
            .filter(|_| state == "queued")
            .map(|n| json!({"place": n + 1, "of": queue.len()}));
        let package = self.shown.package.as_ref();
        json!({"nodeId": invocation.node_id, "actionId": invocation.action_id,
               "invocationId": invocation.invocation_id, "attempt": invocation.attempt,
               "maxAttempts": package.map(|p| max_attempts(p, &invocation.action_id)),
               "claimed": claimed, "state": state, "worker": slot.map(|s| &s.worker),
               "pid": pid, "queue": place, "cutOff": cut_off, "sinceMs": since,
               "forMs": lasted})
    }
}

/// `sinceMs` and `forMs` of a stand that began at `since`, as decimal strings.
fn since_json(since: Option<u64>, now_ms: u64) -> (Value, Value) {
    since.map_or((Value::Null, Value::Null), |since| {
        let lasted = now_ms.saturating_sub(since);
        (json!(since.to_string()), json!(lasted.to_string()))
    })
}

/// The `slots` object of `status`: how many steps a worker runs at once, and each step
/// a worker runs now, with the worker, since its claim and the pid of its program.
fn slots_json(shown: &Shown) -> Value {
    let held: Vec<Value> = shown
        .slots
        .iter()
        .map(|slot| {
            let (since, lasted) = since_json(slot.claimed_at_ms, shown.now_ms);
            json!({"worker": slot.worker, "runId": slot.key.run, "startKey": slot.start_key,
                   "nodeId": slot.node_id, "attempt": slot.attempt, "pid": slot.pid,
                   "sinceMs": since, "forMs": lasted})
        })
        .collect();
    json!({"limit": shown.slot_limit, "held": held})
}

/// `node on outcome`, the way an arrow reads in text.
fn arrow_text(arrow: &Value) -> String {
    match (arrow["nodeId"].as_str(), arrow["outcome"].as_str()) {
        (Some(node), Some(outcome)) => format!(", from {node} on {outcome}"),
        _ => String::new(),
    }
}

fn status_text(view: &RunView, shown: &Shown) -> String {
    let s = status_json(view, shown);
    let mut text = format!(
        "run        {}\nstart key  {}\n",
        view.key.run, view.start_key
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
        None => text += &format!("status     {}\n", view.status),
    }
    text += &format!(
        "revision   {}\npackage    {}\n",
        view.revision, view.package_digest
    );
    if let Some(artifact) = &view.artifact_digest {
        text += &format!("artifact   {artifact}\n");
    }
    if let Some(parent) = &view.parent {
        text += &format!("parent     {} at {}\n", parent.run_id, parent.node_id);
    }
    if let Some(p) = s["position"].as_object() {
        text += &format!(
            "position   {} ({}{})\n",
            p["nodeId"].as_str().unwrap_or_default(),
            p["activationId"].as_str().unwrap_or_default(),
            arrow_text(&p["from"])
        );
    }
    if let Some(end) = s["end"].as_object() {
        text += &format!(
            "end        {}{}\n",
            end["nodeId"].as_str().unwrap_or_default(),
            arrow_text(&end["from"])
        );
    }
    if let Some(i) = s["invocation"].as_object() {
        let of = i
            .get("maxAttempts")
            .map_or_else(String::new, |m| format!(" of {m}"));
        text += &format!(
            "invocation {} attempt {}{of} {}\n",
            i["invocationId"].as_str().unwrap_or_default(),
            i["attempt"],
            i["state"].as_str().unwrap_or_default()
        );
        if let Some(error) = i.get("error") {
            text += &error_text(error);
        }
        if let Some(p) = i.get("process").and_then(Value::as_object) {
            let gone = match s["running"]["cutOff"]["reason"].as_str() {
                Some("process-gone") => ", gone",
                _ => "",
            };
            text += &format!(
                "process    group {} on {}{gone}\n",
                p["pgid"],
                p["host"].as_str().unwrap_or_default()
            );
        }
    }
    text += &waiting_text(&s["waiting"]);
    text += &intervention_text(&s["intervention"], &s["invocation"]);
    // An external step waits for an actor, and a capability-missing one for a spore
    // that can run it, not for a worker's slot; a quarantined run waits for a release.
    let waits = shown.step.as_ref().map(|step| step.state);
    if view.quarantine.is_none() {
        text += &running_text(&s["running"], waits);
        if s["running"].is_object() && waits.is_none() {
            text += &slots_text(&s["slots"]);
        }
    }
    for (node, streak) in s["streaks"].as_object().into_iter().flatten() {
        text += &format!(
            "streak     {node} {} {} of {} in a row\n",
            streak["outcome"].as_str().unwrap_or_default(),
            streak["count"],
            streak["limit"]
        );
    }
    if !s["result"].is_null() {
        text += &format!("result     {}\n", s["result"]);
        text += &tails_text(&s["result"]["error"], "");
    }
    if let Some(cancel) = &view.cancel {
        text += &format!(
            "cancelled  {}{}\n",
            utc_text(cancel.at_ms),
            reason_text(Some(cancel))
        );
    }
    text
}

/// An error's code and message, then the ends of its command's streams.
fn error_text(error: &Value) -> String {
    format!(
        "error      {}: {}\n",
        error["code"].as_str().unwrap_or_default(),
        error["message"].as_str().unwrap_or_default()
    ) + &tails_text(error, "")
}

/// ` since <time>, for <duration>` from a detail's `sinceMs` and `forMs`.
fn since_text(detail: &Value) -> String {
    let ms = |key: &str| detail[key].as_str().and_then(|v| v.parse::<u64>().ok());
    match (ms("sinceMs"), ms("forMs")) {
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

fn commands_text(label: &str, detail: &Value) -> String {
    let commands = detail["commands"].as_array().into_iter().flatten();
    lines_text(
        label,
        commands.filter_map(Value::as_str).map(str::to_string),
    )
}

/// The signal and since when, each outcome with the schema its data must fit, and the
/// `spore signal` commands.
fn waiting_text(waiting: &Value) -> String {
    if !waiting.is_object() {
        return String::new();
    }
    let header = format!(
        "waiting    for signal {}{}\n",
        waiting["signal"].as_str().unwrap_or_default(),
        since_text(waiting)
    );
    let outcomes = waiting["outcomes"].as_array().into_iter().flatten();
    let outcomes = outcomes.filter_map(Value::as_str).map(|outcome| {
        match waiting["outputSchemas"].get(outcome) {
            Some(schema) => format!("{outcome}, with data that fits {schema}"),
            None => outcome.to_string(),
        }
    });
    header + &lines_text("outcomes", outcomes) + &commands_text("answer", waiting)
}

/// Why a person is needed and since when, the last error when it is not the kept one
/// shown with the invocation, and the `spore resolve` commands.
fn intervention_text(intervention: &Value, invocation: &Value) -> String {
    if !intervention.is_object() {
        return String::new();
    }
    let why = match (
        intervention["reason"].as_str(),
        intervention["attemptStatus"].as_str(),
    ) {
        (Some("unknown"), Some("expired")) => "lost its last attempt's lease with no result",
        (Some("unknown"), _) => "has a last attempt whose outcome is unknown",
        _ => "gave up and has no failed arrow",
    };
    let mut text = format!(
        "needs      a person: step {} {why}{}\n",
        intervention["nodeId"].as_str().unwrap_or_default(),
        since_text(intervention)
    );
    let error = &intervention["error"];
    if error.is_object() && invocation.get("error") != Some(error) {
        text += &error_text(error);
    }
    text + &commands_text("resolve", intervention)
}

/// The try, and whether its program runs, it was cut off, it waits for a slot (an `external` step: for
/// an actor; a capability-missing one: for a spore that can run it) or it waits to
/// retry.
fn running_text(running: &Value, waits: Option<StepState>) -> String {
    if !running.is_object() {
        return String::new();
    }
    if running["state"] == "cut-off" {
        return format!(
            "running    {}; {} runs it again\n",
            cut_off_line(running),
            running["cutOff"]["commands"][0]
                .as_str()
                .unwrap_or_default()
        );
    }
    let of = running["maxAttempts"]
        .as_u64()
        .map_or_else(String::new, |m| format!(" of {m}"));
    let since = since_text(running);
    let stands = match running["state"].as_str() {
        Some("started") => {
            let by = running["worker"]
                .as_str()
                .map_or_else(String::new, |w| format!(" by {w}"));
            format!("started{by}{since}{}", pid_text(&running["pid"]))
        }
        Some("backoff") => match running["sinceMs"].as_str().and_then(|v| v.parse().ok()) {
            Some(due) => format!("waiting to retry, due {}", utc_text(due)),
            None => "waiting to retry".to_string(),
        },
        _ if waits == Some(StepState::CapabilityMissing) => {
            format!("queued for a spore that can run it{since}")
        }
        _ if waits.is_some() => format!("queued for an actor to claim it{since}"),
        _ => format!(
            "queued for a free worker slot{}{since}",
            place_text(&running["queue"])
        ),
    };
    format!(
        "running    {} attempt {}{of}, {stands}\n",
        running["nodeId"].as_str().unwrap_or_default(),
        running["attempt"],
    )
}

/// `, 3rd of 16 waiting`: the step's place in the queue for a worker's slot.
fn place_text(queue: &Value) -> String {
    match (queue["place"].as_u64(), queue["of"].as_u64()) {
        (Some(place), Some(of)) => format!(", {} of {of} waiting", ordinal(place)),
        _ => String::new(),
    }
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
fn ordinal(n: u64) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn pid_text(pid: &Value) -> String {
    pid.as_i64()
        .map_or_else(String::new, |pid| format!(", pid {pid}"))
}

/// Per worker, how many of its slots are in use and the step in each; or that no
/// worker runs a step.
fn slots_text(slots: &Value) -> String {
    let limit = &slots["limit"];
    let held = slots["held"].as_array().map_or(&[][..], Vec::as_slice);
    if held.is_empty() {
        return format!("slots      none of {limit} in use: no worker runs a step\n");
    }
    let mut workers: Vec<&str> = Vec::new();
    for worker in held.iter().filter_map(|h| h["worker"].as_str()) {
        if !workers.contains(&worker) {
            workers.push(worker);
        }
    }
    let mut lines = Vec::new();
    for worker in workers {
        let steps: Vec<&Value> = held.iter().filter(|h| h["worker"] == worker).collect();
        lines.push(format!("{} of {limit} in use by {worker}", steps.len()));
        for step in steps {
            lines.push(format!(
                "  {} at {} attempt {}{}{}",
                step["startKey"].as_str().unwrap_or_default(),
                step["nodeId"].as_str().unwrap_or_default(),
                step["attempt"],
                pid_text(&step["pid"]),
                since_text(step)
            ));
        }
    }
    lines_text("slots", lines)
}

/// A time in milliseconds since the Unix epoch, in UTC: `2026-10-14T20:46:03Z`.
fn utc_text(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // The civil date of a day count since 1970-01-01 (Howard Hinnant's days_from_civil
    // inverse), in eras of 400 years from 0000-03-01.
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// A duration in milliseconds, in its two largest units: `45s`, `3m12s`, `2h05m`, `3d4h`.
fn duration_text(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m{:02}s", secs / 60, secs % 60),
        3600..86_400 => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d{}h", secs / 86_400, secs % 86_400 / 3600),
    }
}

/// `runs` as a tree, parents first, each with its depth. Under `root`, the runs it
/// started and theirs, the direct children at depth 0; without it, every run, a run
/// whose parent is not among `runs` at the top. Siblings keep the order of `runs`.
pub fn tree<'a>(runs: &'a [RunView], root: Option<&str>) -> Vec<(&'a RunView, usize)> {
    let index: HashMap<&str, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.key.run.as_str(), i))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); runs.len()];
    let mut tops = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        match run
            .parent
            .as_ref()
            .and_then(|p| index.get(p.run_id.as_str()))
        {
            Some(&parent) => children[parent].push(i),
            None => tops.push(i),
        }
    }
    if let Some(root) = root {
        tops = index
            .get(root)
            .map_or_else(Vec::new, |&r| children[r].clone());
    }
    let mut out = Vec::new();
    let mut seen = vec![false; runs.len()];
    let mut walk = |top: usize, out: &mut Vec<(&'a RunView, usize)>| {
        let mut stack = vec![(top, 0)];
        while let Some((i, depth)) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            out.push((&runs[i], depth));
            stack.extend(children[i].iter().rev().map(|&c| (c, depth + 1)));
        }
    };
    for top in tops {
        walk(top, &mut out);
    }
    // Runs whose parents start each other have no top; they still show.
    if root.is_none() {
        for i in 0..runs.len() {
            walk(i, &mut out);
        }
    }
    out
}

fn tree_json(v: &RunView, run: &Row, now_ms: u64) -> Value {
    json!({
        "runId": v.key.run, "startKey": v.start_key, "status": v.status,
        "revision": v.revision.to_string(), "quarantined": v.quarantine.is_some(),
        "parent": parent_json(v), "depth": run.depth, "nodeId": run.node_id,
        "sinceMs": run.since_ms.map(|s| s.to_string()),
        "workflow": run.workflow, "needs": run.needs,
        "since": v.updated_at_ms.to_string(), "actor": run.actor,
        "needsPerson": needs_person_json(run.needs_person.as_ref(), now_ms),
    })
}

/// One line: the run indented under its parent, its status, node, time at the node,
/// start key, and the parent's node that started it. `indent` levels go before every
/// run; `width` is the width of the column of indented run IDs.
fn tree_text(v: &RunView, run: &Row, indent: usize, width: usize, now_ms: u64) -> String {
    let node = run.node_id.as_deref().unwrap_or("-");
    let since = run.since_ms.map_or_else(
        || "-".to_string(),
        |s| duration_text(now_ms.saturating_sub(s)),
    );
    let from = match (&v.parent, run.depth + indent) {
        (Some(p), 1..) => format!("  from {}", p.node_id),
        _ => String::new(),
    };
    let head = format!("{:1$}{2}", "", 2 * (run.depth + indent), v.key.run);
    format!(
        "{head:<width$}  {:<18}  {node:<12} {since:>7}  {}{from}{}{}",
        status_shown(v),
        v.start_key,
        needs_person_row_text(run.needs_person.as_ref(), now_ms),
        quarantine_row_text(v)
    )
}

/// The run and node that started `view`, or null.
fn parent_json(view: &RunView) -> Value {
    view.parent.as_ref().map_or(
        Value::Null,
        |p| json!({"runId": p.run_id, "nodeId": p.node_id}),
    )
}

pub fn event_json(record: &EventRecord) -> Value {
    let event = &record.event;
    let payload: Value = serde_json::from_slice(&event.body).unwrap_or(Value::Null);
    let diagnostics: Vec<Value> = record
        .diagnostics
        .iter()
        .map(|d| {
            json!({"code": d.code, "nodeId": d.node_id,
                   "details": serde_json::from_slice::<Value>(&d.details).unwrap_or(Value::Null)})
        })
        .collect();
    json!({
        "sequence": event.sequence.to_string(),
        "id": event.id,
        "kind": event.kind,
        "acceptedAtMs": event.accepted_at_ms.to_string(),
        "consumedRevision": record.consumed_revision.map(|r| r.to_string()),
        "diagnostics": diagnostics,
        "payload": payload,
    })
}

/// The ends of a command's streams an error's details keep (spec/host.md §3,
/// `command`), each line under the stream's name, `indent` before every line. Empty
/// when there are none.
fn tails_text(error: &Value, indent: &str) -> String {
    let mut text = String::new();
    for (key, stream) in [("stdoutTail", "stdout"), ("stderrTail", "stderr")] {
        let Some(tail) = error["details"][key].as_str().filter(|t| !t.is_empty()) else {
            continue;
        };
        for (i, line) in tail.lines().enumerate() {
            let label = if i == 0 { stream } else { "" };
            text += &format!("{indent}{label:<10} {line}\n");
        }
    }
    text
}

/// One line per event: its sequence, its time in UTC, its kind, the revision that
/// consumed it, the node it was for (`-` when none or not consumed yet, padded to
/// `width`), what it said and what the run then did, its diagnostics and its ID. Then
/// the ends of the streams of a command that failed.
pub fn event_text(record: &EventRecord, happened: Option<&Happened>, width: usize) -> String {
    let event = &record.event;
    let revision = record
        .consumed_revision
        .map_or_else(|| "pending".to_string(), |r| format!("rev {r}"));
    let codes: Vec<&str> = record.diagnostics.iter().map(|d| d.code.as_str()).collect();
    let payload: Value = serde_json::from_slice(&event.body).unwrap_or(Value::Null);
    let node = happened.and_then(|h| h.node_id.as_deref()).unwrap_or("-");
    let said = happened.map(happened_text).unwrap_or_default();
    let line = [said, codes.join(","), event.id.clone()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .fold(
            format!(
                "{:>4}  {}  {:<22} {:<10} {node:<width$}",
                event.sequence,
                utc_text(event.accepted_at_ms),
                event.kind,
                revision,
            ),
            |line, part| format!("{line}  {part}"),
        );
    let tails = tails_text(&payload["error"], "      ");
    match tails.strip_suffix('\n') {
        Some(tails) => format!("{line}\n{tails}"),
        None => line,
    }
}

/// What an event said and what the run then did: its outcome and the arrow the run took
/// from its node, then the first line of its error or output, then the status the run
/// came to. For example `red -> implement`, `failure: command.exit: exit code 1;
/// needs intervention`.
fn happened_text(happened: &Happened) -> String {
    let mut text = happened.outcome.clone().unwrap_or_default();
    let left = happened.changes.iter().find_map(|change| match change {
        Change::Outcome {
            node_id, arrow, to, ..
        } if Some(node_id) == happened.node_id.as_ref() => Some((arrow, to)),
        _ => None,
    });
    if let Some((arrow, to)) = left {
        if Some(arrow) != happened.outcome.as_ref() {
            text += &format!(", took {arrow}");
        }
        text += &format!(" -> {to}");
    }
    if let Some(detail) = &happened.detail {
        if !text.is_empty() {
            text += ": ";
        }
        text += detail;
    }
    let came = happened.changes.iter().find_map(|change| match change {
        Change::Waiting { signal, .. } => Some(format!("waiting for {signal}")),
        Change::NeedsIntervention { .. } => Some("needs intervention".to_string()),
        Change::Completed { .. } => Some("completed".to_string()),
        Change::Failed { .. } => Some("failed".to_string()),
        _ => None,
    });
    if let Some(came) = came {
        if !text.is_empty() {
            text += "; ";
        }
        text += &came;
    }
    text
}

/// A change of a run as one line for a person: the time in UTC, the run's start key and
/// the change (spec/cli.md, Progress).
pub fn progress_text(progress: &Progress) -> String {
    let change = match &progress.change {
        Change::StepStarted { node_id, attempt } => {
            format!("step {node_id} started, attempt {attempt}")
        }
        Change::TryFailed {
            node_id,
            attempt,
            code,
            message,
        } => format!("step {node_id} attempt {attempt} failed: {code}: {message}"),
        Change::Outcome {
            node_id,
            outcome,
            arrow,
            to,
        } if arrow != outcome => format!("{node_id} returned {outcome}, took {arrow} -> {to}"),
        Change::Outcome {
            node_id,
            outcome,
            to,
            ..
        } => format!("{node_id} returned {outcome} -> {to}"),
        Change::Waiting { node_id, signal } => {
            format!("waiting at {node_id} for signal {signal}")
        }
        Change::NeedsIntervention { node_id, reason } => {
            format!("needs intervention at {node_id}: {reason}")
        }
        Change::Completed {
            node_id: Some(node),
        } => format!("completed at {node}"),
        Change::Completed { node_id: None } => "completed".to_string(),
        Change::Failed {
            node_id,
            code,
            message,
        } => {
            let at = node_id
                .as_deref()
                .map_or_else(String::new, |n| format!(" at {n}"));
            format!("failed{at}: {code}: {message}")
        }
        Change::Quarantined { code, details } => format!("quarantined: {code}: {details}"),
        Change::Paused {
            node_id: Some(node),
            reason,
        } => format!("paused at {node}: {reason}"),
        Change::Paused {
            node_id: None,
            reason,
        } => format!("paused: {reason}"),
    };
    format!(
        "{}  {}  {change}",
        utc_text(progress.at_ms),
        progress.start_key
    )
}

/// A change of a run as one JSON object: `atMs`, `runId`, `startKey`, `change`, and the
/// change's fields.
pub fn progress_json(progress: &Progress) -> Value {
    let mut json = match &progress.change {
        Change::StepStarted { node_id, attempt } => json!({"nodeId": node_id, "attempt": attempt}),
        Change::TryFailed {
            node_id,
            attempt,
            code,
            message,
        } => json!({"nodeId": node_id, "attempt": attempt,
                    "error": {"code": code, "message": message}}),
        Change::Outcome {
            node_id,
            outcome,
            arrow,
            to,
        } => json!({"nodeId": node_id, "outcome": outcome, "arrow": arrow, "to": to}),
        Change::Waiting { node_id, signal } => json!({"nodeId": node_id, "signal": signal}),
        Change::NeedsIntervention { node_id, reason } => {
            json!({"nodeId": node_id, "reason": reason})
        }
        Change::Completed { node_id } => json!({"nodeId": node_id}),
        Change::Failed {
            node_id,
            code,
            message,
        } => json!({"nodeId": node_id, "error": {"code": code, "message": message}}),
        Change::Quarantined { code, details } => json!({"code": code, "details": details}),
        Change::Paused { node_id, reason } => json!({"nodeId": node_id, "reason": reason}),
    };
    json["atMs"] = json!(progress.at_ms.to_string());
    json["runId"] = json!(progress.key.run);
    json["startKey"] = json!(progress.start_key);
    json["change"] = json!(progress.change.name());
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_in_utc() {
        assert_eq!(utc_text(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_text(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(utc_text(1_791_924_363_999), "2026-10-13T20:46:03Z");
    }

    #[test]
    fn durations_read_in_their_two_largest_units() {
        assert_eq!(duration_text(999), "0s");
        assert_eq!(duration_text(45_000), "45s");
        assert_eq!(duration_text(192_000), "3m12s");
        assert_eq!(duration_text(7_500_000), "2h05m");
        assert_eq!(duration_text(273_600_000), "3d4h");
    }
}
