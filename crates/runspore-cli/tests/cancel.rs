//! `spore cancel` through the compiled binary (spec/cli.md, Cancelling a run): a run
//! ends as `cancelled` with its position kept, `run`, `worker --until-parked` and
//! `continue` exit 7, a running command stops, and a held step's stop runs once.

mod common;

use std::time::{Duration, Instant};

use common::Dir;
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde_json::{json, Value};

const FAST: [&str; 5] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--heartbeat-ms",
    "100",
];

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

/// `spore --json …`: the exit code and the JSON it printed.
fn json(dir: &Dir, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(extra);
    let out = dir.run(&args(&all));
    (out.code(), out.json())
}

fn read(dir: &Dir, name: &str) -> String {
    std::fs::read_to_string(dir.file(name)).unwrap_or_default()
}

/// Waits up to 30 s for `name` to exist and returns what it holds.
fn wait_for(dir: &Dir, name: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.file(name).exists() {
        assert!(Instant::now() < deadline, "{name} never appeared");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    read(dir, name)
}

/// `work` runs `argv` (with `async` when given), then `approve` waits for a signal.
fn flow(argv: Value, async_spec: Option<Value>) -> Value {
    let mut work = json!({"kind": "command", "effect": "idempotent", "argv": argv});
    if let Some(spec) = async_spec {
        work["async"] = spec;
    }
    json!({
        "format": "runspore.workflow/0.2", "name": "cancel", "start": "work",
        "actions": {"work": work},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

/// Starts the workflow folder `wf` with key `key`; returns the run ID.
fn start(dir: &Dir, wf: &str, key: &str) -> String {
    let (code, started) = json(dir, &["start", wf, "--key", key]);
    assert_eq!(code, Some(0), "{started}");
    started["runId"].as_str().unwrap().to_string()
}

#[test]
fn a_waiting_run_is_cancelled_and_keeps_its_position() {
    let dir = Dir::new("cancel-waiting");
    dir.folder("wf", &flow(json!(["true"]), None));
    let run = start(&dir, "wf", "waiting");
    let (code, parked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(3), "{parked}");

    let (code, cancelled) = json(
        &dir,
        &["cancel", &run, "--reason", "no longer needed", "--id", "c1"],
    );
    assert_eq!(code, Some(0), "{cancelled}");
    assert_eq!(
        (
            &cancelled["disposition"],
            &cancelled["status"],
            &cancelled["cancel"]["reason"],
            &cancelled["stops"]
        ),
        (
            &json!("applied"),
            &json!("cancelled"),
            &json!("no longer needed"),
            &json!([])
        )
    );
    assert_eq!(cancelled["format"], "runspore.cli/0.2");

    let (code, status) = json(&dir, &["status", &run]);
    assert_eq!(code, Some(0), "{status}");
    assert_eq!(status["status"], "cancelled");
    assert_eq!(status["position"]["nodeId"], "approve", "position kept");
    assert_eq!(status["cancel"]["reason"], "no longer needed");
    assert_eq!(status["cancel"]["at"], cancelled["cancel"]["at"]);
    assert!(status["cancel"]["at"].is_string());
    assert!(status["end"].is_null() && status["waiting"].is_null() && status["step"].is_null());

    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!((code, &out["needs"]), (Some(7), &Value::Null), "{out}");
    let (code, out) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(7), "{out}");
    let (code, listed) = json(&dir, &["list", "--unfinished"]);
    assert_eq!((code, &listed["runs"]), (Some(0), &json!([])));
    let (_, listed) = json(&dir, &["list"]);
    let row = &listed["runs"][0];
    assert_eq!(
        (&row["status"], &row["needs"], &row["nodeId"]),
        (&json!("cancelled"), &Value::Null, &Value::Null)
    );
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(
        (code, &out["runs"]),
        (Some(0), &json!([])),
        "nothing to continue"
    );
    let (code, out) = json(&dir, &["step", "show", &run]);
    assert_eq!(
        (code, &out["error"]["code"]),
        (Some(2), &json!("step.none")),
        "{out}"
    );

    let (_, events) = json(&dir, &["events", &run]);
    assert_eq!(events["cancel"]["requestId"], "cancel/c1");
    assert_eq!(events["cancel"]["reason"], "no longer needed");
    let text = dir.run(&args(&["events", &run]));
    let last = text.stdout.trim_end().lines().last().unwrap().to_string();
    assert!(
        last.contains("run.cancelled") && last.contains("no longer needed"),
        "{last}"
    );

    let (code, again) = json(
        &dir,
        &["cancel", &run, "--reason", "no longer needed", "--id", "c1"],
    );
    assert_eq!(
        (code, &again["disposition"]),
        (Some(0), &json!("duplicate"))
    );
    let (code, other) = json(&dir, &["cancel", &run]);
    assert_eq!(
        (code, &other["disposition"]),
        (Some(0), &json!("duplicate"))
    );
    let (code, reused) = json(&dir, &["cancel", &run, "--reason", "other", "--id", "c1"]);
    assert_eq!(
        (code, &reused["error"]["code"]),
        (Some(2), &json!("cancel.id-reused")),
        "{reused}"
    );
}

#[test]
fn run_exits_7_and_its_command_stops_when_the_run_is_cancelled_meanwhile() {
    let dir = Dir::new("cancel-running");
    dir.folder(
        "wf",
        &flow(json!(["/bin/sh", "-c", "echo $$ > pid; sleep 30"]), None),
    );
    let child = dir.spawn(&args(&["--json", "run", "wf", "--key", "running"]));
    let pid: i32 = wait_for(&dir, "pid").trim().parse().unwrap();
    let (_, listed) = json(&dir, &["list"]);
    let run = listed["runs"][0]["runId"].as_str().unwrap().to_string();

    let cancelled_at = Instant::now();
    let (code, cancelled) = json(&dir, &["cancel", &run]);
    assert_eq!(code, Some(0), "{cancelled}");
    let out = child.wait_with_output().unwrap();
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(10),
        "run returned soon after the cancel"
    );
    assert_eq!(out.status.code(), Some(7));
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(printed["status"], "cancelled");
    assert_eq!(
        kill(Pid::from_raw(pid), None),
        Err(Errno::ESRCH),
        "the command is gone"
    );
}

#[test]
fn a_held_async_step_is_stopped_with_its_handle() {
    let dir = Dir::new("cancel-held");
    let stop = "cat > stopped; printf %s \"$RUNSPORE_DB\" > db";
    let spec = json!({"timeoutMs": "60000",
                      "stop": {"command": {"argv": ["/bin/sh", "-c", stop]}}});
    let start_command = json!(["/bin/sh", "-c", "echo '{\"job\": 7}'"]);
    dir.folder("wf", &flow(start_command, Some(spec)));
    let (code, parked) = json(&dir, &["run", "wf", "--key", "held"]);
    assert_eq!(code, Some(6), "{parked}");
    let run = parked["runId"].as_str().unwrap().to_string();

    let (code, cancelled) = json(&dir, &["cancel", &run, "--reason", "stop it"]);
    assert_eq!(code, Some(0), "{cancelled}");
    let stops = cancelled["stops"].as_array().unwrap();
    assert_eq!(stops.len(), 1, "{cancelled}");
    assert_eq!(
        (&stops[0]["attempt"], &stops[0]["exit"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(read(&dir, "stopped"), r#"{"job":7}"#, "the handle on stdin");
    assert_eq!(
        read(&dir, "db"),
        dir.file("runspore.db").display().to_string()
    );

    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(7), "{out}");
}

#[test]
fn a_cancel_cut_off_at_its_commit_is_whole_or_absent() {
    for (position, before_retry, retried) in [
        ("before-commit", "waiting", "applied"),
        ("after-commit", "cancelled", "duplicate"),
    ] {
        let dir = Dir::new(&format!("cancel-crash-{position}"));
        dir.folder("wf", &flow(json!(["true"]), None));
        let run = start(&dir, "wf", "crash");
        let (code, _) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
        assert_eq!(code, Some(3));
        let mut cut = dir.cmd(&args(&["--json", "cancel", &run, "--id", "c1"]));
        cut.env(
            "RUNSPORE_FAILPOINTS",
            format!("store.cancel-run.{position}"),
        );
        assert!(!common::run(cut).status.success(), "{position}: aborted");
        let (_, status) = json(&dir, &["status", &run]);
        assert_eq!(status["status"], before_retry, "{position}");
        let (code, again) = json(&dir, &["cancel", &run, "--id", "c1"]);
        assert_eq!(
            (code, &again["disposition"], &again["status"]),
            (Some(0), &json!(retried), &json!("cancelled")),
            "{position}"
        );
    }
}

#[test]
fn an_ended_run_is_left_as_it_is() {
    let dir = Dir::new("cancel-ended");
    let package = json!({
        "format": "runspore.workflow/0.2", "name": "done", "start": "work",
        "actions": {"work": {"kind": "command", "effect": "idempotent", "argv": ["true"]}},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    });
    dir.folder("wf", &package);
    let (code, parked) = json(&dir, &["run", "wf", "--key", "ended"]);
    assert_eq!(code, Some(0), "{parked}");
    let run = parked["runId"].as_str().unwrap().to_string();
    let (code, cancelled) = json(&dir, &["cancel", &run]);
    assert_eq!(code, Some(0), "{cancelled}");
    assert_eq!(
        (
            &cancelled["disposition"],
            &cancelled["status"],
            &cancelled["cancel"]
        ),
        (&json!("duplicate"), &json!("completed"), &Value::Null)
    );
    let text = dir.run(&args(&["cancel", &run]));
    assert!(
        text.stdout
            .contains("had already completed; nothing changed"),
        "{}",
        text.stdout
    );
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["status"], "completed");

    let (code, missing) = json(&dir, &["cancel", "run_missing"]);
    assert_eq!(
        (code, &missing["error"]["code"]),
        (Some(2), &json!("run.not-found"))
    );
}
