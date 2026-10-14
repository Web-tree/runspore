//! Worker slots (spec/host.md §2): each worker runs at most `max_concurrent_activities`
//! attempts at once, and every attempt it holds takes one slot until it ends.

use runspore_types::store::*;

use super::{retry, Engine, PAGE};
use crate::error::HostError;

/// An attempt that takes a worker's slot: claimed by a worker, unfinished, with a lease
/// that has not run out, at a step that is not external.
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
            for run in page.items.iter().filter(|r| !finished(&r.status)) {
                let invocations = retry(false, || inner.store.list_invocations(&run.key)).await?;
                for invocation in invocations {
                    let live = invocation.lease_until_ms.is_some_and(|l| now_ms < l);
                    if invocation.status != InvocationStatus::Running || !live {
                        continue;
                    }
                    if inner.is_external(&run.key, &invocation.invocation_id).await {
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
                        pid: invocation.process.map(|p| p.pgid),
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

fn finished(status: &str) -> bool {
    matches!(status, "completed" | "failed")
}
