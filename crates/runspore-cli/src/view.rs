//! Rendering of store views as CLI JSON objects and text. Counters and timestamps are
//! decimal strings.

use std::collections::HashMap;

use runspore_host::disk::DiskFull;
use runspore_host::needs::Need;
use runspore_host::progress::{Change, Happened, Progress};
use runspore_host::Cancelled;
use runspore_types::store::{Disposition, EventRecord, RunCancel, RunView, STATUS_CANCELLED};
use serde::Serialize;
use serde_json::{json, Value};

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
    pub needs: Option<Need>,
    pub actor: Option<String>,
}

impl Tree {
    fn each(&self) -> impl Iterator<Item = (&RunView, &Row)> {
        self.rows.iter().map(|row| (&self.runs[row.index], row))
    }

    pub fn json(&self) -> Value {
        Value::Array(self.each().map(|(v, r)| tree_json(v, r)).collect())
    }

    /// The runs directly below the tree's root that have not ended.
    pub fn unfinished_below(&self) -> impl Iterator<Item = &RunView> {
        self.each()
            .filter(|(v, r)| r.depth == 0 && !v.ended())
            .map(|(v, _)| v)
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
                    "{:<runs$}  {:<workflows$}  {:<nodes$}  {:<14} {since:>7}  {}{actor}{}",
                    v.key.run,
                    r.workflow.as_deref().unwrap_or("-"),
                    r.node_id.as_deref().unwrap_or("-"),
                    r.needs.map_or("-", Need::as_str),
                    v.start_key,
                    quarantine_row_text(v)
                )
            })
            .collect();
        lines.join("\n")
    }
}

/// The command that lifts a run's quarantine.
pub(crate) fn release_command(view: &RunView) -> String {
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

/// `{"reason", "atMs"}` of a cancel.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelJson<'a> {
    reason: Option<&'a str>,
    at_ms: String,
}

impl<'a> CancelJson<'a> {
    pub(crate) fn of(cancel: &'a RunCancel) -> Self {
        Self {
            reason: cancel.reason.as_deref(),
            at_ms: cancel.at_ms.to_string(),
        }
    }
}

/// `: <reason>` of a cancel that was given one, else nothing.
pub(crate) fn reason_text(cancel: Option<&RunCancel>) -> String {
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
        "status": cancelled.status, "cancel": cancelled.cancel.as_ref().map(CancelJson::of),
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

/// A time in milliseconds since the Unix epoch, in UTC: `2026-10-14T20:46:03Z`.
pub(crate) fn utc_text(ms: u64) -> String {
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
pub(crate) fn duration_text(ms: u64) -> String {
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

fn tree_json(v: &RunView, run: &Row) -> Value {
    json!({
        "runId": v.key.run, "startKey": v.start_key, "status": v.status,
        "revision": v.revision.to_string(), "quarantined": v.quarantine.is_some(),
        "parent": parent_json(v), "depth": run.depth, "nodeId": run.node_id,
        "sinceMs": run.since_ms.map(|s| s.to_string()),
        "workflow": run.workflow, "needs": run.needs,
        "updatedAtMs": v.updated_at_ms.to_string(), "actor": run.actor,
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
        "{head:<width$}  {:<18}  {node:<12} {since:>7}  {}{from}{}",
        status_shown(v),
        v.start_key,
        quarantine_row_text(v)
    )
}

/// The run and node that started `view`, or null.
pub(crate) fn parent_json(view: &RunView) -> Value {
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
pub(crate) fn tails_text(error: &Value, indent: &str) -> String {
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
