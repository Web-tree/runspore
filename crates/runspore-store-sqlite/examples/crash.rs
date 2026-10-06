//! Crash helper for test Q03: `crash <db> <op>` replays the prerequisite mutations of
//! `<op>` and then performs it, printing its disposition. Every request is fixed, so a
//! second run resubmits identical requests. Run it with `RUNSPORE_FAILPOINTS` set.

use runspore_store_conformance::{
    append, claim, commit, create_run, expire, finish, heartbeat, schedule, ManualClock,
};
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, RunKey, Store, Turn, DEFAULT_TENANT};

/// The operations in the order a scenario performs them.
const STEPS: [&str; 7] = [
    "create-run",
    "append-event",
    "commit-turn",
    "claim-attempt",
    "heartbeat",
    "finish-attempt",
    "expire-attempt",
];

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (path, op) = (&args[1], args[2].as_str());
    let store = SqliteStore::open_with_clock(path, ManualClock::new(1_000)).expect("open");
    let key = RunKey::new(DEFAULT_TENANT, "crash");
    let mut last = Disposition::Applied;
    for step in STEPS
        .iter()
        .filter(|s| !["heartbeat", "finish-attempt", "expire-attempt"].contains(s) || **s == op)
    {
        last = match *step {
            "create-run" => {
                store
                    .create_run(create_run(&key, "create", 0))
                    .await
                    .expect("create")
                    .disposition
            }
            "append-event" => {
                store
                    .append_event(append(&key, "append", "sig:1", serde_json::json!(1)))
                    .await
                    .expect("append")
                    .disposition
            }
            "commit-turn" => {
                let event = store
                    .list_events(&key, 0, 1)
                    .await
                    .expect("events")
                    .items
                    .remove(0)
                    .event;
                let turn = Turn {
                    key: key.clone(),
                    revision: 0,
                    applied_sequence: 0,
                    package_digest: String::new(),
                    snapshot: None,
                    snapshot_digest: None,
                    next_event: event,
                };
                store
                    .commit_turn(commit(&turn, "commit", 0, vec![schedule(&key, "a", 0).0]))
                    .await
                    .expect("commit")
                    .disposition
            }
            "claim-attempt" => {
                let lease = if op == "expire-attempt" { 0 } else { 100 };
                store
                    .claim_attempt(claim(&key, "claim", &schedule(&key, "a", 0).1, "w1", lease))
                    .await
                    .expect("claim")
                    .disposition
            }
            other => {
                let attempt = store
                    .get_receipt(&key, "claim")
                    .await
                    .expect("receipt")
                    .expect("claimed")
                    .decode::<runspore_types::store::Claim>()
                    .expect("claim")
                    .attempt;
                match other {
                    "heartbeat" => {
                        store
                            .heartbeat(heartbeat(&attempt, "hb", 200))
                            .await
                            .expect("heartbeat")
                            .disposition
                    }
                    "finish-attempt" => {
                        store
                            .finish_attempt(finish(&attempt, "finish", 1))
                            .await
                            .expect("finish")
                            .disposition
                    }
                    _ => {
                        store
                            .expire_attempt(expire(&attempt, "expire"))
                            .await
                            .expect("expire")
                            .disposition
                    }
                }
            }
        };
        if *step == op {
            break;
        }
    }
    println!(
        "{}",
        if last == Disposition::Applied {
            "applied"
        } else {
            "duplicate"
        }
    );
}
