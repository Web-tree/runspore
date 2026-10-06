//! Crash injection for durability tests.
//!
//! `RUNSPORE_FAILPOINTS` holds a comma-separated list of `name` or `name:n`.
//! When execution reaches a named point for the n-th time (default first), the
//! process aborts without unwinding or flushing, as a power cut would. The
//! registered names are listed in `spec/host.md`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const ENV: &str = "RUNSPORE_FAILPOINTS";

static PLAN: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

fn plan() -> &'static Mutex<HashMap<String, u64>> {
    PLAN.get_or_init(|| {
        let mut map = HashMap::new();
        if let Ok(spec) = std::env::var(ENV) {
            for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
                let (name, nth) = match entry.rsplit_once(':') {
                    Some((name, n)) => (name, n.parse().unwrap_or(1)),
                    None => (entry, 1),
                };
                map.insert(name.to_string(), nth.max(1));
            }
        }
        Mutex::new(map)
    })
}

/// Aborts the process if `name` is armed and this is its configured hit.
pub fn hit(name: &str) {
    let mut plan = plan()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(remaining) = plan.get_mut(name) {
        *remaining -= 1;
        if *remaining == 0 {
            eprintln!("runspore: failpoint {name} hit, aborting");
            std::process::abort();
        }
    }
}
