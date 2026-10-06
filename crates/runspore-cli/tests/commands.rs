//! The command surface of spec/cli.md, driven through the compiled binary.

mod common;

use std::time::{Duration, Instant};

use common::{effect_argv, Dir};
use serde_json::{json, Value};

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

fn flow(sleep: &str, effect: &str) -> Value {
    json!({
        "format": "runspore.workflow/0.1", "name": "surface", "start": "work",
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
    dir.write("flow.json", &flow("0", "idempotent"));
    let out = dir.run(&["validate", "flow.json"]);
    assert_eq!(out.code(), Some(0));
    assert_eq!(
        dir.run(&["--json", "validate", "flow.json"]).json()["valid"],
        true
    );
    assert!(
        !dir.file("runspore.db").exists(),
        "validate created a database"
    );
    assert_eq!(dir.run(&["validate", "missing.json"]).code(), Some(2));
    std::fs::write(dir.file("bad.json"), "{").unwrap();
    let bad = dir.run(&["--json", "validate", "bad.json"]);
    assert_eq!(
        (bad.code(), bad.json()["format"].clone()),
        (Some(2), json!("runspore.cli/0.1"))
    );
    assert_eq!(dir.run(&["frobnicate"]).code(), Some(2));
    assert_eq!(
        dir.run(&["--json", "status", "run_nope"]).json()["error"]["code"],
        "run.not-found"
    );

    let start = dir
        .run(&args(&["--json", "start", "flow.json", "--key", "k1"]))
        .json();
    let again = dir
        .run(&args(&["--json", "start", "flow.json", "--key", "k1"]))
        .json();
    assert_eq!(
        (start["created"].clone(), again["created"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(start["runId"], again["runId"]);
    let run = start["runId"].as_str().unwrap();
    assert_eq!(dir.run(&args(&["start", "flow.json"])).code(), Some(0));
    assert_eq!(
        dir.run(&args(&["run", "flow.json", "--key", "k1"])).code(),
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
                assert_eq!(out.json()["format"], "runspore.cli/0.1");
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
    let done = dir.run(&args(&["--json", "run", "flow.json", "--key", "k1"]));
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
        .run(&args(&["--json", "start", "flow.json", "--key", "k2"]))
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
        dir.run(&args(&["run", "flow.json", "--key", "k2"])).code(),
        Some(1)
    );
}

#[test]
fn unsafe_step_needs_intervention_and_resolve_is_idempotent() {
    let dir = Dir::new("unsafe");
    dir.write("flow.json", &flow("5", "unsafe"));
    let mut child = dir.spawn(&args(&["run", "flow.json", "--key", "u"]));
    while dir.effects().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let parked = dir.run(&args(&["--json", "run", "flow.json", "--key", "u"]));
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
        dir.run(&args(&["run", "flow.json", "--key", "u"])).code(),
        Some(3)
    );
    assert_eq!(dir.effects().len(), 1);
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
        "format": "runspore.workflow/0.1", "name": "oversized", "start": "a",
        "actions": {"fetch": {"kind": "command", "effect": "read-only", "output": "json", "argv": argv}},
        "nodes": nodes
    })
}

#[test]
fn a_run_over_the_state_budget_is_quarantined_until_released() {
    let dir = Dir::new("quarantine");
    dir.write("flow.json", &oversized());
    let run = ["--json", "run", "flow.json", "--key", "q"];
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
    dir.write("flow.json", &flow("0", "idempotent"));
    std::fs::create_dir(dir.file("a-directory")).unwrap();
    std::fs::write(dir.file("not-a-database"), "plain text\n").unwrap();
    for (db, code) in [
        ("a-directory", "store.io"),
        ("not-a-database", "store.corrupt"),
    ] {
        for command in [&["list"][..], &["run", "flow.json"]] {
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
        text.stderr.starts_with("runspore: store.io: "),
        "{}",
        text.stderr
    );
}

#[test]
fn a_worker_and_a_separate_signal_process_share_one_database() {
    let dir = Dir::new("two");
    dir.write("flow.json", &flow("0", "idempotent"));
    let run = dir
        .run(&args(&["--json", "start", "flow.json", "--key", "w"]))
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
    assert!(worker.wait().unwrap().success());
}

#[test]
fn sigint_stops_a_worker_within_the_grace_period_and_another_finishes() {
    let dir = Dir::new("sigint");
    dir.write("flow.json", &flow("30", "idempotent"));
    let mut worker = dir.spawn(&args(&[
        "--grace-ms",
        "300",
        "run",
        "flow.json",
        "--key",
        "s",
    ]));
    while dir.effects().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let sent = Instant::now();
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(worker.id() as i32),
        nix::sys::signal::SIGINT,
    )
    .unwrap();
    let exit = worker.wait().unwrap();
    assert!(
        sent.elapsed() < Duration::from_millis(300 * 2 + 1500),
        "took {:?}",
        sent.elapsed()
    );
    assert_eq!(exit.code(), Some(130));
    dir.write("flow.json", &flow("30", "idempotent"));
    let next = dir
        .run(&args(&["--json", "start", "flow.json", "--key", "s"]))
        .json();
    std::fs::write(
        dir.file("flow.json"),
        serde_json::to_vec_pretty(&flow("30", "idempotent")).unwrap(),
    )
    .unwrap();
    let run = next["runId"].as_str().unwrap();
    let mut finisher = dir.spawn(&args(&["worker", "--run", run, "--until-parked"]));
    let deadline = Instant::now() + Duration::from_secs(5);
    while dir.effects().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "the second worker did not retry the step"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = finisher.kill();
    let _ = finisher.wait();
    let effects = dir.effects();
    assert_eq!(
        (effects[0].key.clone(), effects[1].attempt),
        (effects[1].key.clone(), 2)
    );
}
