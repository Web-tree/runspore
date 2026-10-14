//! A newer `spore` continues the runs an older 0.2 `spore` left in its database
//! (spec/store.md §3, spec/kernel.md Versions, spec/host.md §2 tick): the file is
//! upgraded in place, a run waiting at a signal takes the signal, and a step that was
//! handed out reports its result and moves its run on.

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

    // Its schema: what the file held before versions 5 to 7 were added.
    sql(&dir)
        .execute_batch(&format!(
            "ALTER TABLE invocations DROP COLUMN params;
             ALTER TABLE attempts DROP COLUMN holder;
             ALTER TABLE attempts DROP COLUMN deadline_ms;
             DROP TABLE meta;
             PRAGMA user_version = {OLDER};"
        ))
        .unwrap();
    let older = as_the_older_spore_reads_it(&dir);
    assert_eq!(older.0, OLDER);

    // An upgrade that stops after its first step leaves the file as the older spore
    // wrote it.
    let mut cmd = dir.cmd(&["--json", "status", &gate_run]);
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

    // The newer spore upgrades it, and status shows both runs where they were.
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
