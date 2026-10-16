//! The queue (spec/host.md §2, Dispatch): due steps in the order a worker or an actor
//! takes them. A higher action `priority` goes first, as Kubernetes schedules a pod of
//! a higher priority class first; among equals the step ready longest goes first, as
//! Temporal's task queues hand out tasks in the order they were scheduled.

use std::collections::HashSet;

use runspore_types::store::*;

use super::{lock, Engine, Inner};
use crate::error::HostError;

impl Inner {
    /// The `priority` of a due invocation's action: 0 when it sets none, or when its
    /// run cannot be read. Remembered, since an invocation's action never changes.
    async fn priority(&self, key: &RunKey, invocation_id: &str) -> i64 {
        let slot = (key.clone(), invocation_id.to_string());
        if let Some(priority) = lock(&self.priorities).get(&slot) {
            return *priority;
        }
        let Ok(Some((invocation, package, _))) = self.invocation_package(key, invocation_id).await
        else {
            return 0;
        };
        let priority = package
            .actions
            .get(&invocation.action_id)
            .and_then(|action| action.get("priority"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        lock(&self.priorities).insert(slot, priority);
        priority
    }

    /// `due`, in `scan_due` order (oldest `notBeforeMs` first), put in the order they
    /// are taken: highest priority first, keeping that order among equals.
    pub(super) async fn ordered(&self, due: Vec<(RunKey, String)>) -> Vec<(RunKey, String)> {
        let mut ranked = Vec::with_capacity(due.len());
        for (key, invocation_id) in due {
            let priority = self.priority(&key, &invocation_id).await;
            ranked.push((priority, key, invocation_id));
        }
        ranked.sort_by_key(|(priority, _, _)| std::cmp::Reverse(*priority));
        ranked.into_iter().map(|(_, k, i)| (k, i)).collect()
    }

    /// The due invocations of `items`, in the order they are taken. Forgets the
    /// priorities of invocations no longer due.
    pub(super) async fn queue(&self, items: Vec<DueItem>) -> Vec<(RunKey, String)> {
        let due: Vec<(RunKey, String)> = items
            .into_iter()
            .filter_map(|item| match item {
                DueItem::Outbox { key, invocation_id } => Some((key, invocation_id)),
                DueItem::Lease { .. } => None,
            })
            .collect();
        let still: HashSet<&(RunKey, String)> = due.iter().collect();
        lock(&self.priorities).retain(|slot, _| still.contains(slot));
        self.ordered(due).await
    }
}

impl Engine {
    /// The steps of this engine's tenant that wait for a worker's free slot, in the
    /// order a worker takes them: due, and neither `external` nor of an action this
    /// engine falls short of.
    pub async fn queue(&self) -> Result<Vec<(RunKey, String)>, HostError> {
        let inner = &self.inner;
        let tenant = &inner.config.tenant;
        let mut waiting = Vec::new();
        for (key, invocation_id) in inner.queue(inner.scan_due().await?).await {
            if &key.tenant == tenant && !inner.left_alone(&key, &invocation_id).await {
                waiting.push((key, invocation_id));
            }
        }
        Ok(waiting)
    }
}
