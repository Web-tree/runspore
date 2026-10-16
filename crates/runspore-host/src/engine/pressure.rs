//! Disk pressure (spec/host.md §2): a worker whose disk is under the floor takes no new
//! step, and a try that ends meanwhile is set aside instead of counted.

use std::time::Duration;

use runspore_types::model::Effect;
use runspore_types::store::{AttemptRef, RunKey, StoreFailureKind};
use serde_json::{json, Value};
use tokio::sync::watch;
use tokio::time::Interval;

use super::{lock, Engine, Inner};
use crate::activity::ActivityOutput;
use crate::disk::DiskFull;
use crate::progress::Change;

/// The code of an unsafe step's try that ended on a full disk.
pub const DISK_FULL: &str = "host.disk-full";

impl Engine {
    /// The disk of this engine's steps, `files_dir` or else `base_dir`, when its free
    /// space is under `disk_floor`: while it is, no step is claimed.
    pub fn disk_full(&self) -> Option<DiskFull> {
        self.inner.disk_full()
    }
}

/// Whether a try of `action` may run again: its effect is `pure`, `read-only` or
/// `idempotent` (kernel §7.4).
pub(super) fn repeatable(action: &Value) -> bool {
    serde_json::from_value::<Effect>(action["effect"].clone()).is_ok_and(Effect::repeatable)
}

/// What an unsafe step's try that ended on a full disk reports: the kernel asks a
/// person, as for any unsafe step whose outcome is unknown.
pub(super) fn unknown(full: &DiskFull) -> ActivityOutput {
    let details = json!({"freeBytes": full.free.to_string(), "floorBytes": full.floor.to_string(),
                         "mount": full.mount});
    ActivityOutput::Unknown {
        error: ActivityOutput::error(DISK_FULL, format!("the try ended on a {full}"), details),
    }
}

impl Inner {
    pub(super) fn disk_full(&self) -> Option<DiskFull> {
        let config = &self.config;
        let path = config.files_dir.as_deref().unwrap_or(&config.base_dir);
        config.disk.full(path, config.disk_floor)
    }

    /// Notes that a step of the run waits for disk space, telling the observer once per
    /// pause: at `node_id` when a try there was set aside.
    pub(super) async fn held_back(&self, key: &RunKey, node_id: Option<&str>, full: &DiskFull) {
        if !lock(&self.paused).insert(key.clone()) {
            return;
        }
        let change = Change::Paused {
            node_id: node_id.map(str::to_string),
            reason: full.to_string(),
        };
        self.tell_observer(key, vec![change]).await;
    }

    /// The run's steps are taken again.
    pub(super) fn unpaused(&self, key: &RunKey) {
        lock(&self.paused).remove(key);
    }

    /// Keeps the attempt's lease until the disk is over the floor again: true then,
    /// false when the lease is lost or the engine shuts down first, which leaves the
    /// attempt to its lease as after a crash.
    pub(super) async fn await_space(
        &self,
        attempt: &AttemptRef,
        beat: &mut Interval,
        shutdown: &mut watch::Receiver<bool>,
    ) -> bool {
        let poll = Duration::from_millis(self.config.poll_ms.max(1));
        loop {
            tokio::select! {
                _ = beat.tick() => {
                    if let Err(f) = self.heartbeat(attempt).await {
                        if f.kind == StoreFailureKind::Stale {
                            return false;
                        }
                    }
                }
                () = async { let _ = shutdown.wait_for(|s| *s).await; } => return false,
                () = tokio::time::sleep(poll) => {
                    if self.disk_full().is_none() {
                        self.unpaused(&attempt.key);
                        return true;
                    }
                }
            }
        }
    }
}
