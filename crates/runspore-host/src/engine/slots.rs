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
