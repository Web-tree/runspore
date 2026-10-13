//! The command surface of spec/cli.md, driven through the compiled binary.

mod common;

use std::time::{Duration, Instant};

use common::{effect_argv, Dir};
use serde_json::{json, Value};

/// The heartbeat renews the lease well before it runs out on a busy machine.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

fn flow(sleep: &str, effect: &str) -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "surface", "start": "work",
        "actions": {"work": {"kind": "command", "effect": effect, "retry": {"maxAttempts": 3, "backoffMs": "10"},
                             "argv": effect_argv("work", &format!("sleep {sleep}"))}},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done", "rejected": "broken"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "approve", "outcome"]}},
            "broken": {"kind": "fail", "error": {"code": "rejected", "message": "rejected"}}
        }
    })
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

#[test]
fn every_command_in_text_and_json_with_its_exit_codes() {
    let dir = Dir::new("surface");
    dir.folder("flow", &flow("0", "idempotent"));
    let out = dir.run(&["validate", "flow"]);
    assert_eq!(out.code(), Some(0));
    assert_eq!(
        dir.run(&["--json", "validate", "flow"]).json()["valid"],
        true
    );
    assert!(
        !dir.file("runspore.db").exists(),
        "validate created a database"
    );
    assert_eq!(dir.run(&["validate", "missing"]).code(), Some(2));
    dir.folder("bad", &flow("0", "idempotent"));
    std::fs::write(dir.file("bad/settings.json"), "{").unwrap();
    let bad = dir.run(&["--json", "validate", "bad"]);
    assert_eq!(
        (bad.code(), bad.json()["format"].clone()),
        (Some(2), json!("runspore.cli/0.2"))
    );
    assert_eq!(dir.run(&["frobnicate"]).code(), Some(2));
    assert_eq!(
        dir.run(&["--json", "status", "run_nope"]).json()["error"]["code"],
        "run.not-found"
    );

    let start = dir
        .run(&args(&["--json", "start", "flow", "--key", "k1"]))
        .json();
    let again = dir
        .run(&args(&["--json", "start", "flow", "--key", "k1"]))
        .json();
    assert_eq!(
        (start["created"].clone(), again["created"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(start["runId"], again["runId"]);
    let elsewhere = dir
        .run(&args(&[
            "--json", "--db", "other.db", "start", "flow", "--key", "k1",
        ]))
        .json();
    assert_eq!(elsewhere["created"], json!(true));
    assert_ne!(
        elsewhere["runId"], start["runId"],
        "a start key names a run only in its own database"
    );
    let run = start["runId"].as_str().unwrap();
    assert_eq!(dir.run(&args(&["start", "flow"])).code(), Some(0));
    assert_eq!(
        dir.run(&args(&["run", "flow", "--key", "k1"])).code(),
        Some(3)
    );
    assert_eq!(
        dir.run(&args(&["worker", "--run", run, "--until-parked"]))
            .code(),
        Some(3)
    );
    for mode in [&[][..], &["--json"][..]] {
        for cmd in [&["status", run][..], &["list"][..], &["events", run][..]] {
            let mut all = args(mode);
            all.extend_from_slice(cmd);
            let out = dir.run(&all);
            assert_eq!(out.code(), Some(0), "{cmd:?}: {}", out.stderr);
            if !mode.is_empty() {
                assert_eq!(out.json()["format"], "runspore.cli/0.2");
            }
        }
    }
    let events = dir.run(&args(&["--json", "events", run])).json();
    assert_eq!(events["events"][0]["kind"], "run.started");
    assert_eq!(events["events"][0]["consumedRevision"], "1");
    let signal = [
        "signal",
        run,
        "approval",
        "--outcome",
        "approved",
        "--id",
        "m1",
    ];
    assert_eq!(dir.run(&args(&signal)).code(), Some(0));
    let repeated = dir
        .run(&args(&[&["--json"][..], &signal[..]].concat()))
        .json();
    assert_eq!(repeated["disposition"], "duplicate");
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "k1"]));
    assert_eq!(
        (done.code(), done.json()["result"]["output"].clone()),
        (Some(0), json!("approved"))
    );
    let release = dir.run(&args(&["release", run]));
    assert_eq!(
        release.code(),
        Some(0),
        "release without quarantine: {}",
        release.stderr
    );

    let rejected = dir
        .run(&args(&["--json", "start", "flow", "--key", "k2"]))
        .json();
    let run2 = rejected["runId"].as_str().unwrap();
    dir.run(&args(&[
        "signal",
        run2,
        "approval",
        "--outcome",
        "rejected",
    ]));
    assert_eq!(
        dir.run(&args(&["run", "flow", "--key", "k2"])).code(),
        Some(1)
    );
}

#[test]
fn unsafe_step_needs_intervention_and_resolve_is_idempotent() {
    let dir = Dir::new("unsafe");
    dir.folder("flow", &flow("5", "unsafe"));
    let mut child = dir.spawn(&args(&["run", "flow", "--key", "u"]));
    while dir.effects().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "u"]));
    assert_eq!(parked.code(), Some(4));
    let status = parked.json();
    let (run, inv) = (
        status["runId"].as_str().unwrap(),
        status["invocation"]["invocationId"].as_str().unwrap(),
    );
    let resolve = [
        "--json",
        "resolve",
        run,
        inv,
        "--complete",
        "ok",
        "--id",
        "r1",
    ];
    assert_eq!(dir.run(&args(&resolve)).json()["disposition"], "applied");
    assert_eq!(dir.run(&args(&resolve)).json()["disposition"], "duplicate");
    assert_eq!(
        dir.run(&args(&["run", "flow", "--key", "u"])).code(),
        Some(3)
    );
    assert_eq!(dir.effects().len(), 1);
}

/// spore 0.2 reads no JSON workflow document (spec/cli.md): `validate`, `start` and
/// `run` refuse one with exit 2 and say that a workflow is a folder, and nothing is
/// created.
#[test]
fn a_json_workflow_document_is_refused() {
    let dir = Dir::new("document");
    dir.write("flow.json", &flow("0", "idempotent"));
    for command in ["validate", "start", "run"] {
        let out = dir.run(&args(
            &["--json", command, "flow.json", "--key", "d"][..command_args(command)],
        ));
        let error = &out.json()["error"];
        assert_eq!(
            (out.code(), error["code"].clone()),
            (Some(2), json!("workflow.folder")),
            "{command}: {error}"
        );
        let message = error["message"].as_str().unwrap();
        for wanted in ["flow.json", "folder", "workflow.dot", "settings.json"] {
            assert!(
                message.contains(wanted),
                "{command}: no `{wanted}` in {message}"
            );
        }
        assert!(
            !dir.file("runspore.db").exists(),
            "{command} created a database"
        );
    }
    let text = dir.run(&["validate", "flow.json"]);
    assert_eq!(text.code(), Some(2));
    assert!(
        text.stderr.starts_with("spore: workflow.folder: "),
        "{}",
        text.stderr
    );
}

/// How many of `["--json", cmd, "flow.json", "--key", "d"]` a command takes:
/// `validate` has no key.
fn command_args(command: &str) -> usize {
    if command == "validate" {
        3
    } else {
        5
    }
}

/// A database another spore wrote is refused by every command that opens one, with
/// exit 10, the version that wrote it, and the fix (spec/cli.md, databases from
/// another version). The file is left as it was.
#[test]
fn a_database_from_another_version_is_refused() {
    let dir = Dir::new("other-version");
    dir.folder("flow", &flow("0", "idempotent"));
    let db = dir.file("runspore.db");

    // A 0.1 database: schema version 1, which 0.2 never upgrades.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE runs (run_id TEXT); PRAGMA user_version = 1;")
        .unwrap();
    drop(conn);
    for command in [
        &["list"][..],
        &["status", "run_x"][..],
        &["start", "flow", "--key", "k"][..],
    ] {
        let mut all = vec!["--json"];
        all.extend_from_slice(command);
        let out = dir.run(&all);
        let error = &out.json()["error"];
        assert_eq!(
            (out.code(), error["code"].clone()),
            (Some(10), json!("schema.too-old")),
            "{command:?}: {error}"
        );
        let message = error["message"].as_str().unwrap();
        for wanted in [
            "runspore.db",
            "spore 0.1",
            "0.1 binary",
            "move the file aside",
        ] {
            assert!(
                message.contains(wanted),
                "{command:?}: no `{wanted}` in {message}"
            );
        }
    }
    let conn = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (version, journal.as_str()),
        (1, "delete"),
        "the 0.1 database was changed"
    );
    drop(conn);

    // A database a newer spore wrote: it records its writer.
    std::fs::remove_file(&db).unwrap();
    let started = dir.run(&args(&["--json", "start", "flow", "--key", "k"]));
    assert_eq!(started.code(), Some(0), "{}", started.stderr);
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "UPDATE meta SET value = '9.9.9' WHERE key = 'writer'; PRAGMA user_version = 99;",
    )
    .unwrap();
    drop(conn);
    let out = dir.run(&["--json", "list"]);
    let error = &out.json()["error"];
    assert_eq!(
        (out.code(), error["code"].clone()),
        (Some(10), json!("schema.too-new")),
        "{error}"
    );
    let message = error["message"].as_str().unwrap();
    for wanted in ["runspore.db", "spore 9.9.9", "move the file aside"] {
        assert!(message.contains(wanted), "no `{wanted}` in {message}");
    }
}

#[test]
fn every_example_validates_as_a_folder() {
    let dir = Dir::new("examples");
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for name in ["hello", "review-loop", "unsafe-deploy"] {
        let text = std::fs::read_to_string(examples.join(format!("{name}.json"))).unwrap();
        dir.folder(name, &serde_json::from_str(&text).unwrap());
        let out = dir.run(&["--native-kernel", "validate", name]);
        assert_eq!(out.code(), Some(0), "{name}: {}", out.stderr);
    }
}

/// Five steps that each return a 60 KB string: the fifth result takes the snapshot
/// past the kernel's 256 KiB state budget.
fn oversized() -> Value {
    let argv = json!([
        "/bin/sh",
        "-c",
        r#"printf '{"blob": "%s"}' $(yes x | head -n 60000 | tr -d '\n')"#
    ]);
    let steps = ["a", "b", "c", "d", "e", "done"];
    let mut nodes = json!({"done": {"kind": "complete"}});
    for pair in steps.windows(2) {
        nodes[pair[0]] =
            json!({"kind": "activity", "action": "fetch", "outcomes": {"ok": pair[1]}});
    }
    json!({
        "format": "runspore.workflow/0.2", "name": "oversized", "start": "a",
        "actions": {"fetch": {"kind": "command", "effect": "read-only", "output": "json", "argv": argv}},
        "nodes": nodes
    })
}

#[test]
fn a_run_over_the_state_budget_is_quarantined_until_released() {
    let dir = Dir::new("quarantine");
    dir.folder("flow", &oversized());
    let run = ["--json", "run", "flow", "--key", "q"];
    let out = dir.run(&args(&run));
    let status = out.json();
    assert_eq!(
        (out.code(), status["quarantine"]["code"].clone()),
        (Some(5), json!("budget.state-bytes"))
    );
    let run_id = status["runId"].as_str().unwrap();
    let listed = dir.run(&args(&["--json", "list"])).json();
    assert_eq!(listed["runs"][0]["quarantined"], true);

    let release = dir.run(&args(&["--json", "release", run_id]));
    assert_eq!(
        (release.code(), release.json()["disposition"].clone()),
        (Some(0), json!("applied"))
    );
    let lifted = dir.run(&args(&["--json", "status", run_id])).json();
    assert_eq!(lifted["quarantine"], Value::Null);
    // The same transition fails the same way: the run is quarantined again.
    assert_eq!(dir.run(&args(&run)).code(), Some(5));
}

#[test]
fn a_path_that_cannot_be_a_database_is_a_store_error() {
    let dir = Dir::new("store-error");
    dir.folder("flow", &flow("0", "idempotent"));
    std::fs::create_dir(dir.file("a-directory")).unwrap();
    std::fs::write(dir.file("not-a-database"), "plain text\n").unwrap();
    for (db, code) in [
        ("a-directory", "store.io"),
        ("not-a-database", "store.corrupt"),
    ] {
        for command in [&["list"][..], &["run", "flow"]] {
            let mut argv = vec!["--json", "--db", db];
            argv.extend_from_slice(command);
            let out = dir.run(&args(&argv));
            assert_eq!(
                (out.code(), out.json()["error"]["code"].clone()),
                (Some(10), json!(code)),
                "{argv:?}"
            );
        }
    }
    let text = dir.run(&args(&["--db", "a-directory", "list"]));
    assert_eq!(text.code(), Some(10));
    assert!(
        text.stderr.starts_with("spore: store.io: "),
        "{}",
        text.stderr
    );
}

#[test]
fn a_worker_and_a_separate_signal_process_share_one_database() {
    let dir = Dir::new("two");
    dir.folder("flow", &flow("0", "idempotent"));
    let run = dir
        .run(&args(&["--json", "start", "flow", "--key", "w"]))
        .json()["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut worker = dir.spawn(&args(&["worker"]));
    let status = |d: &Dir| d.run(&args(&["--json", "status", &run])).json()["status"].clone();
    while status(&dir) != "waiting" {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        dir.run(&args(&[
            "signal",
            &run,
            "approval",
            "--outcome",
            "approved"
        ]))
        .code(),
        Some(0)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while status(&dir) != "completed" {
        assert!(
            Instant::now() < deadline,
            "the worker did not complete the run"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(worker.id() as i32),
        nix::sys::signal::SIGTERM,
    )
    .unwrap();
    assert_eq!(worker.wait().unwrap().code(), Some(130));
}

/// SIGINT to `run` and to `worker`, and SIGHUP (a closed terminal) to `worker`, while
/// a 30-second idempotent step is in flight:
/// the process stops the command, exits 130 within twice the grace period plus a
/// margin, and has already applied the stopped attempt, so attempt 2 is authorized
/// before any other worker starts. A second worker then performs it.
#[test]
fn an_interrupt_stops_a_worker_within_the_grace_period_and_another_finishes() {
    use nix::sys::signal::{SIGHUP, SIGINT};
    for (name, command, interrupt) in [
        ("sigint-run", &["run", "flow", "--key", "s"][..], SIGINT),
        ("sigint-worker", &["worker"][..], SIGINT),
        ("sighup-worker", &["worker"][..], SIGHUP),
    ] {
        let dir = Dir::new(name);
        dir.folder("flow", &flow("30", "idempotent"));
        let run = dir
            .run(&args(&["--json", "start", "flow", "--key", "s"]))
            .json()["runId"]
            .as_str()
            .unwrap()
            .to_string();
        let mut argv = vec!["--grace-ms", "300"];
        argv.extend_from_slice(command);
        let mut worker = dir.spawn(&args(&argv));
        while dir.effects().is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let sent = Instant::now();
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(worker.id() as i32), interrupt).unwrap();
        let exit = worker.wait().unwrap();
        assert!(
            sent.elapsed() < Duration::from_millis(300 * 2 + 1500),
            "{name} took {:?}",
            sent.elapsed()
        );
        assert_eq!(exit.code(), Some(130), "{name}");
        let status = dir.run(&args(&["--json", "status", &run])).json();
        assert_eq!(
            (
                &status["status"],
                &status["invocation"]["attempt"],
                &status["invocation"]["state"]
            ),
            (&json!("running"), &json!(2), &json!("scheduled")),
            "{name}: {status}"
        );

        let mut finisher = dir.spawn(&args(&["worker", "--run", &run, "--until-parked"]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while dir.effects().len() < 2 {
            assert!(
                Instant::now() < deadline,
                "{name}: the second worker did not retry the step"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = finisher.kill();
        let _ = finisher.wait();
        let effects = dir.effects();
        assert_eq!(
            (effects[0].key.clone(), effects[1].attempt),
            (effects[1].key.clone(), 2),
            "{name}"
        );
    }
}
