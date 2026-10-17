//! A newer `spore` continues the runs an older 0.2 `spore` left in its database
//! (spec/store.md §3, spec/kernel.md Versions, spec/host.md §2 tick): opening the file
//! does not upgrade it; `migrate` does, in place, unless a worker of the older `spore`
//! is still at work on it; then a run waiting at a signal takes the signal, and a step
//! that was handed out reports its result and moves its run on.

mod common;

use common::Dir;
use runspore_store_sqlite::SCHEMA_VERSION;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

/// The schema version the older `spore` wrote: before the writer and an attempt's
/// holder and deadline, so its upgrade takes two steps.
const OLDER: i64 = 4;

/// `build` runs, then the run waits at `approve` for the signal `approval`.
fn gate() -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "gate", "start": "build",
        "actions": {"build": {"kind": "command", "effect": "read-only", "argv": ["echo", "built"]}},
        "nodes": {
            "build": {"kind": "activity", "action": "build", "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done"}},
            "done": {"kind": "complete", "output": {"$get": ["from"]}}
        }
    })
}

/// `review`, an external step for actor `reviewer`, whose output is the run's.
fn review(dir: &Dir) {
    let flow = dir.file("review");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(
        flow.join("workflow.dot"),
        "digraph \"review\" {\n  dialect = \"runspore/0.2\";\n  start = \"review\";\n  \
         done [kind=complete];\n  review -> done [on=\"approved\"];\n}\n",
    )
    .unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {"review": {"kind": "external", "actor": "reviewer", "effect": "unsafe",
                               "outcomes": ["approved"], "async": {"timeoutMs": "600000"}}},
        "nodes": {
            "review": {"action": "review", "input": {"$get": ["input"]}},
            "done": {"output": {"$get": ["nodes", "review", "output"]}}
        }
    });
    std::fs::write(flow.join("settings.json"), settings.to_string()).unwrap();
}

/// `spore --json …`: the exit code and the JSON it printed.
fn json(dir: &Dir, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(&FAST);
    all.extend_from_slice(extra);
    let out = dir.run(&all);
    (out.code(), out.json())
}

/// Where `status` shows the run: its position, with the arrow into it, and its step.
fn place(dir: &Dir, run: &str) -> (Value, Value) {
    let (code, status) = json(dir, &["status", run]);
    assert_eq!(code, Some(0), "{status}");
    (status["position"].clone(), status["step"]["kind"].clone())
}

fn sql(dir: &Dir) -> rusqlite::Connection {
    rusqlite::Connection::open(dir.file("runspore.db")).unwrap()
}

fn user_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// What the older `spore` reads first: the schema version, and every run in it.
fn as_the_older_spore_reads_it(dir: &Dir) -> (i64, Vec<String>) {
    let conn = sql(dir);
    let mut runs = conn
        .prepare("SELECT run_id, status FROM runs ORDER BY run_id")
        .unwrap();
    let runs = runs
        .query_map([], |r| {
            Ok(format!(
                "{} {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (user_version(&conn), runs)
}

#[test]
fn a_newer_spore_upgrades_the_database_and_finishes_the_older_runs() {
    let dir = Dir::new("upgrade");
    dir.folder("gate", &gate());
    review(&dir);

    // The older spore: one run waits at a signal, one step is handed out.
    let (code, waiting) = json(&dir, &["run", "gate", "--key", "a"]);
    assert_eq!(code, Some(3), "{waiting}");
    let gate_run = waiting["runId"].as_str().unwrap().to_string();
    let (code, parked) = json(
        &dir,
        &["run", "review", "--input", r#"{"change": 7}"#, "--key", "b"],
    );
    assert_eq!(code, Some(6), "{parked}");
    let review_run = parked["runId"].as_str().unwrap().to_string();
    let (code, claim) = json(&dir, &["step", "claim", &review_run, "--actor", "reviewer"]);
    assert_eq!(code, Some(0), "{claim}");
    let token = claim["token"].as_str().unwrap().to_string();
    let before = [place(&dir, &gate_run), place(&dir, &review_run)];
    assert_eq!(
        before[0].0["from"],
        json!({"nodeId": "build", "outcome": "ok"})
    );
    assert_eq!(before[1].1, "external");

    // Its schema: what the file held before versions 5 to 12 were added.
    sql(&dir)
        .execute_batch(&format!(
            "ALTER TABLE runs DROP COLUMN cancel_reason;
             ALTER TABLE runs DROP COLUMN cancel_at_ms;
             ALTER TABLE runs DROP COLUMN cancel_request_id;
             ALTER TABLE runs DROP COLUMN result_channels;
             ALTER TABLE attempts DROP COLUMN claimed_at_ms;
             ALTER TABLE runs DROP COLUMN parent_run_id;
             ALTER TABLE runs DROP COLUMN parent_node_id;
             ALTER TABLE invocations DROP COLUMN params;
             ALTER TABLE attempts DROP COLUMN holder;
             ALTER TABLE attempts DROP COLUMN deadline_ms;
             DROP TABLE meta;
             DROP TABLE workers;
             PRAGMA user_version = {OLDER};"
        ))
        .unwrap();
    let older = as_the_older_spore_reads_it(&dir);
    assert_eq!(older.0, OLDER);

    // Opening it does not upgrade it: a command that reads, one that writes, and one
    // that reads even with --migrate are each refused, and the file is left as the
    // older spore wrote it.
    for command in [
        vec!["status", gate_run.as_str()],
        vec![
            "signal",
            gate_run.as_str(),
            "approval",
            "--outcome",
            "approved",
        ],
        vec!["--migrate", "list"],
    ] {
        let (code, refused) = json(&dir, &command);
        assert_eq!(
            (code, &refused["error"]["code"]),
            (Some(10), &json!("schema.needs-migrate")),
            "{command:?}: {refused}"
        );
        let message = refused["error"]["message"].as_str().unwrap();
        for wanted in ["runspore.db", "schema version 4", "spore migrate"] {
            assert!(message.contains(wanted), "no `{wanted}` in {message}");
        }
        assert_eq!(as_the_older_spore_reads_it(&dir), older, "{command:?}");
    }

    // An upgrade that stops after its first step leaves the file as the older spore
    // wrote it.
    let mut cmd = dir.cmd(&["--json", "migrate"]);
    cmd.env(runspore_types::failpoint::ENV, "store.upgrade.after-step");
    let crashed = common::run(cmd);
    assert!(!crashed.status.success(), "the upgrade did not stop");
    assert_eq!(as_the_older_spore_reads_it(&dir), older);
    let tables: i64 = sql(&dir)
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'meta'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "the first upgrade step was rolled back");

    // `migrate` upgrades it, and status shows both runs where they were.
    let (code, migrated) = json(&dir, &["migrate"]);
    assert_eq!(code, Some(0), "{migrated}");
    assert_eq!(
        (&migrated["migrated"], &migrated["from"], &migrated["to"]),
        (&json!(true), &json!(OLDER), &json!(SCHEMA_VERSION))
    );
    let after = [place(&dir, &gate_run), place(&dir, &review_run)];
    assert_eq!(after, before);
    assert_eq!(user_version(&sql(&dir)), SCHEMA_VERSION);

    let (code, signalled) = json(
        &dir,
        &["signal", &gate_run, "approval", "--outcome", "approved"],
    );
    assert_eq!(code, Some(0), "{signalled}");
    let (code, done) = json(&dir, &["run", "gate", "--key", "a"]);
    assert_eq!(
        (code, &done["status"]),
        (Some(0), &json!("completed")),
        "{done}"
    );
    assert_eq!(
        done["result"]["output"],
        json!({"node": "approve", "outcome": "approved", "output": null})
    );

    let complete = [
        "step",
        "complete",
        &token,
        "--outcome",
        "approved",
        "--output",
        r#"{"ok": 1}"#,
    ];
    let (code, reported) = json(&dir, &complete);
    assert_eq!(
        (code, &reported["disposition"]),
        (Some(0), &json!("applied")),
        "{reported}"
    );
    let (code, done) = json(&dir, &["worker", "--run", &review_run, "--until-parked"]);
    assert_eq!(
        (code, &done["status"]),
        (Some(0), &json!("completed")),
        "{done}"
    );
    assert_eq!(done["result"]["output"], json!({"ok": 1}));

    // The older spore refuses the upgraded file, whose version is above its own. No
    // older binary ships with this test, so this one stands in for it, on a file one
    // version above its own: every 0.2 spore refuses such a file the same way.
    const _: () = assert!(SCHEMA_VERSION > OLDER);
    sql(&dir)
        .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
        .unwrap();
    let refused = dir.run(&["--json", "list"]);
    assert_eq!(
        (refused.code(), &refused.json()["error"]["code"]),
        (Some(10), &json!("schema.too-new"))
    );
}

/// `nap`, a step that sleeps until it is stopped.
fn slow() -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "slow", "start": "nap",
        "actions": {"nap": {"kind": "command", "effect": "read-only", "argv": ["sleep", "30"]}},
        "nodes": {
            "nap": {"kind": "activity", "action": "nap", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !done() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The workers the file records: ID and version.
fn workers(dir: &Dir) -> Vec<(String, String)> {
    let conn = sql(dir);
    let mut rows = conn
        .prepare("SELECT worker_id, version FROM workers ORDER BY worker_id")
        .unwrap();
    rows.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// A worker that works on runs, started with a short lease so it is soon gone once
/// stopped.
fn worker(dir: &Dir) -> std::process::Child {
    dir.spawn(&[
        "--native-kernel",
        "--poll-ms",
        "20",
        "--lease-ms",
        "1000",
        "--heartbeat-ms",
        "200",
        "--grace-ms",
        "200",
        "--stop-grace-ms",
        "200",
        "worker",
    ])
}

fn stop(mut worker: std::process::Child) {
    let pid = nix::unistd::Pid::from_raw(worker.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).unwrap();
    worker.wait().unwrap();
}

/// A worker records itself while it works and is gone once it stops. `migrate` waits
/// while a worker of an older `spore`, which records no version, still runs a step;
/// once it has stopped, `--migrate` on a command that writes upgrades the file first.
#[test]
fn migrate_waits_for_the_workers_of_the_older_spore() {
    let dir = Dir::new("migrate-live");
    dir.folder("slow", &slow());
    let (code, started) = json(&dir, &["start", "slow", "--key", "s"]);
    assert_eq!(code, Some(0), "{started}");
    let run = started["runId"].as_str().unwrap().to_string();

    let busy = worker(&dir);
    wait_for("the worker to take the step", || {
        sql(&dir)
            .query_row(
                "SELECT count(*) FROM attempts WHERE status = 'running'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    let registered = workers(&dir);
    assert_eq!(registered.len(), 1, "{registered:?}");
    let (worker_id, version) = &registered[0];
    assert!(worker_id.starts_with("worker-"), "{worker_id}");
    assert_eq!(version, runspore_store_sqlite::VERSION);

    // The file as spore 0.3.0 left it, whose workers record no version: this worker
    // stands in for one of them.
    sql(&dir)
        .execute_batch("DROP TABLE workers; PRAGMA user_version = 11;")
        .unwrap();
    let (code, refused) = json(&dir, &["migrate"]);
    assert_eq!(
        (code, &refused["error"]["code"]),
        (Some(10), &json!("migrate.workers-live")),
        "{refused}"
    );
    let message = refused["error"]["message"].as_str().unwrap();
    for wanted in [worker_id.as_str(), &format!("step nap of run {run}")] {
        assert!(message.contains(wanted), "no `{wanted}` in {message}");
    }
    assert_eq!(user_version(&sql(&dir)), 11, "the file is left as it was");

    stop(busy);
    wait_for("the stopped worker's lease to pass", || {
        let (code, _) = json(&dir, &["--migrate", "release", &run]);
        code == Some(0)
    });
    assert_eq!(user_version(&sql(&dir)), SCHEMA_VERSION);

    let again = worker(&dir);
    wait_for("the new worker to record itself", || {
        workers(&dir).len() == 1
    });
    stop(again);
    assert_eq!(workers(&dir), [], "a stopped worker removes its record");
}
