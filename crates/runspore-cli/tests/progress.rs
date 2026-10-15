//! `run` and `worker` print each change of the runs they work on stderr, one line each,
//! and keep their final output on stdout (spec/cli.md, Progress).

mod common;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

/// `build` fails its first try and passes its second; `gate` waits for `approval`.
fn flow() -> Value {
    let build = "test -f tried || { touch tried; echo 'not yet\nsecond line' >&2; exit 1; }";
    json!({
        "format": "runspore.workflow/0.2", "name": "flow", "start": "build",
        "actions": {"build": {
            "kind": "command", "effect": "idempotent", "argv": ["/bin/sh", "-c", build],
            "retry": {"maxAttempts": 2}
        }},
        "nodes": {
            "build": {"kind": "activity", "action": "build", "outcomes": {"ok": "gate"}},
            "gate": {"kind": "await-signal", "signal": "approval",
                     "outcomes": {"go": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

/// Each stderr line without its time: what follows the first two spaces.
fn changes(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .map(|line| line.split_once("  ").map_or(line, |(_, rest)| rest))
        .collect()
}

#[test]
fn run_and_worker_print_each_change_on_stderr() {
    let dir = Dir::new("progress-text");
    dir.folder("flow", &flow());
    let out = dir.run(&args(&["run", "flow", "--key", "k"]));
    assert_eq!(out.code(), Some(3), "{}{}", out.stdout, out.stderr);
    assert_eq!(
        changes(&out.stderr),
        [
            "k  step build started, attempt 1",
            "k  step build attempt 1 failed: command.exit: exit code 1",
            "k  step build started, attempt 2",
            "k  build returned ok -> gate",
            "k  waiting at gate for signal approval",
        ],
        "{}",
        out.stderr
    );
    let time = out.stderr.split("  ").next().unwrap();
    assert!(
        time.len() == 20 && time.ends_with('Z') && time.as_bytes()[10] == b'T',
        "{time}"
    );
    // Stdout is still the run's status.
    assert!(out.stdout.contains("status     waiting"), "{}", out.stdout);

    let run = dir.run(&["--json", "list"]).json()["runs"][0]["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let signal = dir.run(&["signal", &run, "approval", "--outcome", "go"]);
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let out = dir.run(&args(&["worker", "--run", &run, "--until-parked"]));
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(
        changes(&out.stderr),
        ["k  gate returned go -> done", "k  completed at done"],
        "{}",
        out.stderr
    );
}

#[test]
fn json_prints_one_object_per_change_and_one_result_on_stdout() {
    let dir = Dir::new("progress-json");
    dir.folder("flow", &flow());
    let out = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(out.code(), Some(3), "{}{}", out.stdout, out.stderr);
    assert_eq!(out.json()["status"], "waiting");
    let lines: Vec<Value> = out
        .stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let run = out.json()["runId"].clone();
    for line in &lines {
        assert_eq!(line["format"], "runspore.cli/0.2");
        assert_eq!(line["runId"], run);
        assert_eq!(line["startKey"], "k");
        assert!(
            line["atMs"].as_str().unwrap().parse::<u64>().is_ok(),
            "{line}"
        );
    }
    let names: Vec<&str> = lines
        .iter()
        .map(|l| l["change"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "step-started",
            "try-failed",
            "step-started",
            "outcome",
            "waiting"
        ]
    );
    assert_eq!(lines[0]["nodeId"], "build");
    assert_eq!(lines[0]["attempt"], 1);
    assert_eq!(
        lines[1]["error"],
        json!({"code": "command.exit", "message": "exit code 1"})
    );
    assert_eq!(lines[2]["attempt"], 2);
    assert_eq!(
        (&lines[3]["outcome"], &lines[3]["arrow"], &lines[3]["to"]),
        (&json!("ok"), &json!("ok"), &json!("gate"))
    );
    assert_eq!(lines[4]["signal"], "approval");
}

/// Each `events` line without its sequence, its time and its ID; stream tails left out.
fn events(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| !line.starts_with("      "))
        .map(|line| {
            let rest = line.trim_start().splitn(3, "  ").nth(2).unwrap();
            rest.rsplit_once("  ").unwrap().0
        })
        .collect()
}

#[test]
fn events_show_the_time_node_outcome_and_where_the_run_went() {
    let dir = Dir::new("progress-events");
    dir.folder("flow", &flow());
    let out = dir.run(&args(&["run", "flow", "--key", "k"]));
    assert_eq!(out.code(), Some(3), "{}{}", out.stdout, out.stderr);
    let run = dir.run(&["--json", "list"]).json()["runs"][0]["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let signal = dir.run(&["signal", &run, "approval", "--outcome", "go"]);
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let out = dir.run(&args(&["worker", "--run", &run, "--until-parked"]));
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);

    let out = dir.run(&["events", &run]);
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    assert_eq!(
        events(&out.stdout),
        [
            "run.started            rev 1      build",
            "activity.result        rev 2      build  failure: command.exit: exit code 1  \
             activity.retry-scheduled",
            "activity.result        rev 3      build  ok -> gate: {\"exitCode\":0,\"stderr\":\"\",\
             \"stdout\":\"\",\"truncated\":false}; waiting for approval",
            "signal.received        rev 4      gate   go -> done; completed",
        ],
        "{}",
        out.stdout
    );
    let time = out.stdout.trim_start().split("  ").nth(1).unwrap();
    assert!(
        time.len() == 20 && time.ends_with('Z') && time.as_bytes()[10] == b'T',
        "{time}"
    );
}
