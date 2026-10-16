//! A worker on a full disk takes no new step, and `status` and `list` say why (spec/cli.md,
//! A full disk). The project's `worker.diskFloorMb` sets the floor; one far above any
//! disk stands in for a full one.

mod common;

use std::io::Read;
use std::process::Stdio;
use std::time::Duration;

use common::{effect_argv, Dir};
use serde_json::Value;

const DOT: &str = r#"digraph "disk" {
  dialect = "runspore/0.2";
  start = "work";
  done [kind=complete];
  work -> done;
}
"#;

/// A project whose workflow `flow` runs one idempotent command, with `floor` MiB as its
/// disk floor.
fn project(name: &str, floor: u64) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let argv = effect_argv("work", "true");
    let settings = format!(
        r#"{{"format": "runspore.workflow/0.2",
  "actions": {{"work": {{"kind": "command", "effect": "idempotent", "argv": {argv}}}}},
  "nodes": {{"work": {{"action": "work"}}}}}}"#
    );
    std::fs::write(flow.join("settings.json"), settings).unwrap();
    std::fs::create_dir_all(dir.file(".spore")).unwrap();
    set_floor(&dir, floor);
    dir
}

fn set_floor(dir: &Dir, floor: u64) {
    let settings = format!(r#"{{"worker": {{"diskFloorMb": {floor}}}}}"#);
    std::fs::write(dir.file(".spore/settings.json"), settings).unwrap();
}

fn spore(dir: &Dir, args: &[&str]) -> common::Out {
    let mut cmd = dir.cmd(&[&["--native-kernel"], args].concat());
    cmd.env("RUNSPORE_DB", dir.file(".spore/runspore.db"));
    common::run(cmd)
}

/// `paused: disk full (<free> free on <mount>)`, as one line.
fn says_paused(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("paused: disk full (") && l.contains(" free on /"))
}

#[test]
fn a_worker_on_a_full_disk_takes_no_step_and_status_and_list_say_why() {
    let dir = project("disk-full", 1 << 40);
    let started = spore(&dir, &["--json", "start", "flow", "--key", "k"]);
    assert_eq!(started.code(), Some(0), "{}", started.stdout);
    let run = started.json()["runId"].as_str().unwrap().to_string();

    let mut worker = dir
        .cmd(&["--native-kernel", "--poll-ms", "20", "worker"])
        .env("RUNSPORE_DB", dir.file(".spore/runspore.db"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    worker.kill().unwrap();
    worker.wait().unwrap();
    let mut stderr = String::new();
    worker
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(dir.effects().is_empty(), "a step ran on a full disk");
    assert!(
        stderr.contains("  k  paused: disk full ("),
        "the worker did not say why: {stderr}"
    );

    let status = spore(&dir, &["status", &run]).stdout;
    assert!(says_paused(&status), "{status}");
    let json = spore(&dir, &["--json", "status", &run]).json();
    let paused = &json["paused"];
    assert_eq!(paused["reason"], "disk-full", "{json}");
    assert!(
        paused["mount"].as_str().unwrap().starts_with('/'),
        "{paused}"
    );
    assert_eq!(
        paused["floorBytes"],
        Value::from(((1u64 << 40) << 20).to_string()),
        "{paused}"
    );
    let message = paused["message"].as_str().unwrap();
    assert!(says_paused(&format!("paused: {message}")), "{paused}");

    let list = spore(&dir, &["list"]).stdout;
    assert!(says_paused(list.lines().next().unwrap()), "{list}");
    assert!(list.contains(&run), "{list}");
    let list = spore(&dir, &["--json", "list"]).json();
    assert_eq!(list["paused"]["reason"], "disk-full", "{list}");

    // Space returns: the pause is gone, and the step runs.
    set_floor(&dir, 0);
    let json = spore(&dir, &["--json", "status", &run]).json();
    assert!(json["paused"].is_null(), "{json}");
    assert!(spore(&dir, &["--json", "list"]).json()["paused"].is_null());
    let worked = spore(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(worked.code(), Some(0), "{}{}", worked.stdout, worked.stderr);
    assert_eq!(dir.effects().len(), 1);
    let done = spore(&dir, &["status", &run]).stdout;
    assert!(!done.contains("paused"), "{done}");
}

#[test]
fn a_disk_floor_that_is_not_a_whole_number_is_refused() {
    let dir = Dir::new("disk-floor-invalid");
    std::fs::create_dir_all(dir.file(".spore")).unwrap();
    std::fs::write(
        dir.file(".spore/settings.json"),
        r#"{"worker": {"diskFloorMb": -1}}"#,
    )
    .unwrap();
    let out = spore(&dir, &["--json", "worker"]);
    assert_eq!(out.code(), Some(2), "{}", out.stdout);
    let error = out.json();
    assert_eq!(error["error"]["code"], "project.settings-invalid");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("worker.diskFloorMb: expected a whole number from 0"),
        "{message}"
    );
}
