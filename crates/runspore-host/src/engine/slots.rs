//! Worker slots (spec/host.md §2): each worker runs at most `max_concurrent_activities`
//! attempts at once, and every attempt it holds takes one slot until it ends.

use runspore_types::store::*;

use super::{retry, worker_pid, Engine, PAGE};
use crate::error::HostError;
use crate::process;

/// An attempt that takes a worker's slot: claimed by a worker, unfinished, with a lease
/// that has not run out, at a step that is not external and not held by an async
/// command's holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSlot {
    pub key: RunKey,
    pub start_key: String,
    pub node_id: String,
    pub invocation_id: String,
    pub attempt: u32,
    /// The worker that claimed it.
    pub worker: String,
    pub claimed_at_ms: Option<u64>,
    /// The leader of the process group its program runs in, once it has started.
    pub pid: Option<i32>,
    /// The process record of its program, once it has started.
    pub process: Option<ProcessRecord>,
}

impl HeldSlot {
    /// Whether the worker that claimed it is known to be gone though its lease has not
    /// run out yet: it ran on this machine, and its process no longer runs. Its try is
    /// then left to lease expiry (spec/cli.md, Coming back after a kill).
    pub fn worker_gone(&self) -> bool {
        match (&self.process, worker_pid(&self.worker)) {
            (Some(record), Some(pid)) => process::starter_gone(record, pid),
            _ => false,
        }
    }
}

/// Why nothing works a claimed try any more (spec/cli.md, What a run needs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutOffReason {
    /// Its process group no longer runs on this machine.
    ProcessGone,
    /// The worker that claimed it no longer runs on this machine.
    WorkerGone,
    /// This machine has booted since its program started.
    Rebooted,
    /// Its lease ran out and nobody has expired it yet.
    LeaseExpired,
}

impl CutOffReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcessGone => "process-gone",
            Self::WorkerGone => "worker-gone",
            Self::Rebooted => "rebooted",
            Self::LeaseExpired => "lease-expired",
        }
    }
}

/// A try that a worker claimed and that nothing works any more: its result will never
/// come, and the next driver runs the step again once the lease is out of the way.
/// Like a pod on an unreachable node, which kubectl shows as `Unknown`, not `Running`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutOff {
    pub reason: CutOffReason,
    /// The group leader of its program, from its process record.
    pub pid: Option<i32>,
    /// The machine its program ran on, from its process record.
    pub host: Option<String>,
    pub worker: Option<String>,
    pub lease_until_ms: Option<u64>,
}

/// Whether the claimed try of `invocation` is cut off at `now_ms`. On the machine its
/// process record names, its group is gone (while its worker is not known to run, which
/// may be about to report the result), its worker is gone, or the machine has booted
/// since; anywhere, its lease has run out. A try held outside the worker, by an actor
/// or an async command's holder, is never cut off here: its probe and its lease say.
pub fn cut_off(invocation: &InvocationView, now_ms: u64) -> Option<CutOff> {
    if invocation.status != InvocationStatus::Running || invocation.deadline_ms.is_some() {
        return None;
    }
    let record = invocation.process.as_ref();
    let worker = invocation.owner.as_deref();
    let worker_runs = record
        .zip(worker.and_then(worker_pid))
        .and_then(|(record, pid)| process::starter_runs(record, pid));
    let reason = match record.map(process::look) {
        Some(process::Seen::Rebooted) => Some(CutOffReason::Rebooted),
        Some(process::Seen::Gone) if worker_runs != Some(true) => Some(CutOffReason::ProcessGone),
        _ if worker_runs == Some(false) => Some(CutOffReason::WorkerGone),
        _ => None,
    };
    let lease_out = invocation.lease_until_ms.is_some_and(|l| l <= now_ms);
    let reason = reason.or(lease_out.then_some(CutOffReason::LeaseExpired))?;
    Some(CutOff {
        reason,
        pid: record.map(|r| r.pgid),
        host: record.map(|r| r.host.clone()),
        worker: invocation.owner.clone(),
        lease_until_ms: invocation.lease_until_ms,
    })
}

impl Engine {
    /// The attempts workers hold at `now_ms`, in every unfinished run of this engine's
    /// tenant, by run ID then invocation ID. Steps held by actors take no slot.
    pub async fn held_slots(&self, now_ms: u64) -> Result<Vec<HeldSlot>, HostError> {
        let inner = &self.inner;
        let tenant = &inner.config.tenant;
        let mut held = Vec::new();
        let mut cursor = None;
        loop {
            let page = retry(false, || {
                inner.store.list_runs(tenant, cursor.clone(), PAGE)
            })
            .await?;
            for run in page.items.iter().filter(|r| !r.ended()) {
                let invocations = retry(false, || inner.store.list_invocations(&run.key)).await?;
                for invocation in invocations {
                    let live = invocation.lease_until_ms.is_some_and(|l| now_ms < l);
                    if invocation.status != InvocationStatus::Running || !live {
                        continue;
                    }
                    // A deadline marks an attempt held outside the worker: claimed by an
                    // actor, or handed over by an async start command.
                    if invocation.deadline_ms.is_some()
                        || inner.is_external(&run.key, &invocation.invocation_id).await
                    {
                        continue;
                    }
                    let Some(worker) = invocation.owner else {
                        continue;
                    };
                    held.push(HeldSlot {
                        key: run.key.clone(),
                        start_key: run.start_key.clone(),
                        node_id: invocation.node_id,
                        invocation_id: invocation.invocation_id,
                        attempt: invocation.attempt_number,
                        worker,
                        claimed_at_ms: invocation.claimed_at_ms,
                        pid: invocation.process.as_ref().map(|p| p.pgid),
                        process: invocation.process,
                    });
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                return Ok(held);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;

    use super::*;

    fn slot(worker: &str) -> HeldSlot {
        let record = process::record(std::process::id() as i32).unwrap();
        HeldSlot {
            key: RunKey {
                tenant: "t".into(),
                run: "run_1".into(),
            },
            start_key: "k".into(),
            node_id: "fix".into(),
            invocation_id: "inv_1".into(),
            attempt: 1,
            worker: worker.into(),
            claimed_at_ms: None,
            pid: Some(record.pgid),
            process: Some(record),
        }
    }

    /// A claimed try of `worker` whose lease runs until `lease_until_ms`, its program
    /// the group `process` names.
    fn claimed(
        worker: &str,
        process: Option<ProcessRecord>,
        lease_until_ms: u64,
    ) -> InvocationView {
        InvocationView {
            invocation_id: "inv_1".into(),
            node_id: "implement".into(),
            action_id: "implement".into(),
            status: InvocationStatus::Running,
            attempt_number: 1,
            not_before_ms: 0,
            lease_until_ms: Some(lease_until_ms),
            owner: Some(worker.into()),
            process,
            attempt: None,
            holder: None,
            deadline_ms: None,
            claimed_at_ms: Some(0),
            input: b"{}".to_vec(),
            params: None,
            needs_person: None,
        }
    }

    #[test]
    fn a_try_is_cut_off_when_its_program_or_worker_is_gone_or_its_lease_ran_out() {
        let me = format!("worker-{:x}-1-0", std::process::id());
        let gone_worker = format!("worker-{:x}-1-0", i32::MAX);
        let mut program = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let mine = process::record(program.id() as i32).unwrap();
        // The same group number with a later start: no process of the recorded group.
        let mut gone_group = mine.clone();
        gone_group.started = (mine.started.parse::<u64>().unwrap() + 1).to_string();
        let reason = |i: &InvocationView| cut_off(i, 1_000).map(|c| c.reason);

        assert_eq!(reason(&claimed(&me, Some(mine.clone()), 2_000)), None);
        assert_eq!(
            reason(&claimed(&me, Some(gone_group.clone()), 2_000)),
            None,
            "a live worker may be about to report its program's result"
        );
        let cut = cut_off(&claimed("reviewer", Some(gone_group.clone()), 2_000), 1_000);
        assert_eq!(
            cut.as_ref().map(|c| c.reason),
            Some(CutOffReason::ProcessGone)
        );
        assert_eq!(cut.unwrap().pid, Some(mine.pgid));
        assert_eq!(
            reason(&claimed(&gone_worker, Some(gone_group), 2_000)),
            Some(CutOffReason::ProcessGone)
        );
        assert_eq!(
            reason(&claimed(&gone_worker, Some(mine.clone()), 2_000)),
            Some(CutOffReason::WorkerGone)
        );
        let mut rebooted = mine.clone();
        rebooted.boot = "another-boot".into();
        assert_eq!(
            reason(&claimed(&me, Some(rebooted), 2_000)),
            Some(CutOffReason::Rebooted)
        );
        let mut elsewhere = mine.clone();
        elsewhere.host = format!("{}-elsewhere", elsewhere.host);
        assert_eq!(
            reason(&claimed(&gone_worker, Some(elsewhere.clone()), 2_000)),
            None
        );
        assert_eq!(
            reason(&claimed(&gone_worker, Some(elsewhere), 1_000)),
            Some(CutOffReason::LeaseExpired)
        );
        assert_eq!(
            reason(&claimed(&me, None, 500)),
            Some(CutOffReason::LeaseExpired)
        );

        let mut held = claimed(&gone_worker, Some(mine), 500);
        held.deadline_ms = Some(9_000);
        assert_eq!(reason(&held), None, "a held step's probe and lease say");
        held.deadline_ms = None;
        held.status = InvocationStatus::Pending;
        assert_eq!(reason(&held), None, "nobody claimed it");
        program.kill().unwrap();
        program.wait().unwrap();
    }

    #[test]
    fn a_worker_on_this_machine_whose_process_is_gone_is_gone() {
        let me = format!("worker-{:x}-1-0", std::process::id());
        assert!(!slot(&me).worker_gone());
        assert!(slot(&format!("worker-{:x}-1-0", i32::MAX)).worker_gone());
        assert!(
            !slot("reviewer").worker_gone(),
            "an ID that names no process"
        );
    }
}
