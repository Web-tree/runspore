//! Cancelling a run (spec/host.md §2, Client operations, `cancel`): the store ends the
//! run and fences out its attempts; the host then runs the stop of each held one.

use runspore_types::store::*;
use serde_json::json;

use super::{mutation, retry, Engine, Inner};
use crate::error::HostError;

/// What `Engine::cancel` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cancelled {
    pub key: RunKey,
    pub disposition: Disposition,
    /// `cancelled`, or the status the run had already ended with.
    pub status: String,
    /// The run's cancel record; `None` for a run that ended otherwise.
    pub cancel: Option<RunCancel>,
    /// The stops this cancel ran, one per held attempt whose action declares one.
    pub stops: Vec<Stopped>,
}

/// A stop that ran: the attempt it was for and its exit code, `None` when it did not
/// exit by itself or could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub invocation_id: String,
    pub attempt: u32,
    pub exit: Option<i32>,
}

impl Engine {
    /// Ends an unfinished run as `cancelled` (store §2 `cancel_run`), then runs the stop
    /// of each attempt it expired that was held outside the worker. Repeating
    /// `request_id` is a duplicate and runs no stop; a run that has already ended is
    /// left as it is.
    pub async fn cancel(
        &self,
        key: &RunKey,
        request_id: &str,
        reason: Option<&str>,
    ) -> Result<Cancelled, HostError> {
        let inner = &self.inner;
        let request = CancelRun {
            mutation: mutation(
                "cancel_run",
                format!("cancel/{request_id}"),
                json!([key, reason]),
            )?,
            key: key.clone(),
            reason: reason.map(str::to_string),
        };
        let receipt = retry(true, || inner.store.cancel_run(request.clone())).await?;
        let stops = match receipt.disposition {
            Disposition::Applied => inner.stop_expired(key, &receipt.value.expired).await,
            Disposition::Duplicate => Vec::new(),
        };
        Ok(Cancelled {
            key: key.clone(),
            disposition: receipt.disposition,
            status: receipt.value.status,
            cancel: receipt.value.cancel,
            stops,
        })
    }
}

impl Inner {
    /// Runs, one after another, the stop of each of `expired` that was held outside the
    /// worker and whose action declares one. Its failures are ignored.
    async fn stop_expired(&self, key: &RunKey, expired: &[CancelledAttempt]) -> Vec<Stopped> {
        if expired.is_empty() {
            return Vec::new();
        }
        let (Ok(Some(view)), Ok(invocations)) = (
            retry(false, || self.store.get_run(key)).await,
            retry(false, || self.store.list_invocations(key)).await,
        ) else {
            return Vec::new();
        };
        let mut stops = Vec::new();
        for cancelled in expired {
            let Some(invocation) = invocations
                .iter()
                .find(|i| i.attempt.as_ref() == Some(&cancelled.attempt))
            else {
                continue;
            };
            let Some(held) = self.attempt_held(&view, invocation).await else {
                continue;
            };
            let Some(exit) = self.stop(&held).await else {
                continue;
            };
            stops.push(Stopped {
                invocation_id: invocation.invocation_id.clone(),
                attempt: invocation.attempt_number,
                exit,
            });
        }
        stops
    }
}
