//! Runs that start other runs (spec/cli.md, `list` and `status`): a run started from
//! inside a step records that run and node as its parent, and `list` and `status`
//! show the runs as a tree.

mod common;

use common::{Dir, BIN};
use serde_json::{json, Value};

/// The heartbeat renews the lease well before it runs out on a busy machine. No test
/// here waits for a lease to run out, so the lease is long: a step that starts `spore`
/// itself can stall the worker's heartbeats past a short one under load.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "5000",
    "--heartbeat-ms",
    "100",
];

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

/// `spawn` starts the `child` workflow with `spore start`, as a build starts its
/// ticket runs, then the run waits at `hold`.
fn parent() -> Value {
    let start = format!("'{BIN}' --json start child --key ticket-1 > /dev/null");
    json!({
        "format": "runspore.workflow/0.2", "name": "parent", "start": "spawn",
        "actions": {"spawn": {"kind": "command", "effect": "idempotent", "output": "none",
                              "argv": ["/bin/sh", "-c", start]}},
        "nodes": {
            "spawn": {"kind": "activity", "action": "spawn", "outcomes": {"ok": "hold"}},
            "hold": {"kind": "await-signal", "signal": "go", "outcomes": {"go": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn child() -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "child", "start": "gate",
        "nodes": {
            "gate": {"kind": "await-signal", "signal": "go", "outcomes": {"go": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn row<'a>(runs: &'a Value, run: &str) -> &'a Value {
    runs.as_array()
        .unwrap()
        .iter()
        .find(|r| r["runId"] == run)
        .unwrap_or_else(|| panic!("{run} not in {runs}"))
}

#[test]
fn a_run_started_from_a_step_shows_under_its_parent() {
    let dir = Dir::new("children");
    dir.folder("parent", &parent());
    dir.folder("child", &child());
    let out = dir.run(&args(&["--json", "run", "parent", "--key", "build"]));
    assert_eq!(out.code(), Some(3), "{}", out.stderr);
    let build = out.json()["runId"].as_str().unwrap().to_string();
    let other = dir.run(&args(&["--json", "start", "child", "--key", "alone"]));
    let other = other.json()["runId"].as_str().unwrap().to_string();

    let list = dir.run(&args(&["--json", "list"])).json();
    let ticket = list["runs"].as_array().unwrap()[1]["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let worked = dir.run(&args(&["worker", "--run", &ticket, "--until-parked"]));
    assert_eq!(worked.code(), Some(3), "{}", worked.stderr);
    let list = dir.run(&args(&["--json", "list"])).json();
    let runs = &list["runs"];
    let order: Vec<(&str, u64)> = runs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["runId"].as_str().unwrap(), r["depth"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        order,
        [
            (build.as_str(), 0),
            (ticket.as_str(), 1),
            (other.as_str(), 0)
        ],
        "parents first, each child under its parent: {runs}"
    );
    let child_row = row(runs, &ticket);
    assert_eq!(child_row["startKey"], "ticket-1");
    assert_eq!(
        child_row["parent"],
        json!({"runId": build, "nodeId": "spawn"})
    );
    assert_eq!(child_row["status"], "waiting");
    assert_eq!(child_row["nodeId"], "gate");
    assert!(child_row["sinceMs"].is_string(), "{child_row}");
    let build_row = row(runs, &build);
    assert_eq!(build_row["parent"], Value::Null);
    assert_eq!(build_row["nodeId"], "hold");
    assert_eq!(
        row(runs, &other)["parent"],
        Value::Null,
        "started by a person"
    );

    let status = dir.run(&args(&["--json", "status", &build])).json();
    assert_eq!(status["parent"], Value::Null);
    let children = status["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "{status}");
    assert_eq!(children[0]["runId"], ticket.as_str());
    assert_eq!(children[0]["depth"], 0);
    assert_eq!(children[0]["nodeId"], "gate");
    let status = dir.run(&args(&["--json", "status", &ticket])).json();
    assert_eq!(status["parent"], json!({"runId": build, "nodeId": "spawn"}));
    assert_eq!(status["children"], json!([]));

    let text = dir.run(&args(&["list"])).stdout;
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with(&build), "{text}");
    assert!(lines[1].starts_with(&format!("  {ticket}")), "{text}");
    assert!(
        lines[1].contains("gate") && lines[1].ends_with("from spawn"),
        "{text}"
    );
    let text = dir.run(&args(&["status", &build])).stdout;
    assert!(text.contains(&format!("children\n  {ticket}")), "{text}");
}

/// The run and node come from the step's environment, with no flag; a start again
/// with the same key keeps the parent it was created with. A finished run is at no
/// node.
#[test]
fn the_parent_comes_from_the_step_environment() {
    let dir = Dir::new("grandchildren");
    dir.folder("child", &child());
    let start = |key: &str, parent: Option<(&str, &str)>| {
        let mut cmd = dir.cmd(&args(&["--json", "start", "child", "--key", key]));
        if let Some((run, node)) = parent {
            cmd.env("RUNSPORE_RUN_ID", run)
                .env("RUNSPORE_NODE_ID", node);
        }
        let out = common::run(cmd);
        assert_eq!(out.code(), Some(0), "{}", out.stderr);
        out.json()["runId"].as_str().unwrap().to_string()
    };
    let top = start("top", None);
    let middle = start("middle", Some((&top, "gate")));
    let bottom = start("bottom", Some((&middle, "gate")));
    assert_eq!(start("bottom", Some((&top, "elsewhere"))), bottom);
    let signal = dir.run(&args(&["signal", &top, "go", "--outcome", "go"]));
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let worked = dir.run(&args(&["worker", "--run", &top, "--until-parked"]));
    assert_eq!(worked.code(), Some(0), "{}", worked.stderr);

    let runs = dir.run(&args(&["--json", "list"])).json()["runs"].clone();
    let depths: Vec<(&str, u64)> = runs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["runId"].as_str().unwrap(), r["depth"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        depths,
        [
            (top.as_str(), 0),
            (middle.as_str(), 1),
            (bottom.as_str(), 2)
        ]
    );
    assert_eq!(
        row(&runs, &bottom)["parent"],
        json!({"runId": middle, "nodeId": "gate"}),
        "a repeated start keeps the first parent"
    );
    let finished = row(&runs, &top);
    assert_eq!(finished["status"], "completed");
    assert_eq!(
        (&finished["nodeId"], &finished["sinceMs"]),
        (&Value::Null, &Value::Null)
    );

    let status = dir.run(&args(&["--json", "status", &top])).json();
    let below: Vec<(&str, u64)> = status["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["runId"].as_str().unwrap(), r["depth"].as_u64().unwrap()))
        .collect();
    assert_eq!(below, [(middle.as_str(), 0), (bottom.as_str(), 1)]);
}

/// A child run's steps get the parent's run ID as `RUNSPORE_PARENT_RUN_ID`, so its last
/// step signals the parent with no input naming it. The steps of a run with no parent
/// get none, also when the worker was started with one set.
#[test]
fn a_child_step_names_its_parent_run() {
    let dir = Dir::new("parent-run-id");
    let spawn = format!(
        "printf %s \"${{RUNSPORE_PARENT_RUN_ID-unset}}\" > top-parent; \
         '{BIN}' --json start reporter --key ticket-1 > /dev/null"
    );
    let mut parent = parent();
    parent["actions"]["spawn"]["argv"] = json!(["/bin/sh", "-c", spawn]);
    dir.folder("parent", &parent);
    let report =
        format!("'{BIN}' --native-kernel signal \"$RUNSPORE_PARENT_RUN_ID\" go --outcome go");
    dir.folder(
        "reporter",
        &json!({
            "format": "runspore.workflow/0.2", "name": "reporter", "start": "gate",
            "actions": {"report": {"kind": "command", "effect": "idempotent", "output": "none",
                                   "argv": ["/bin/sh", "-c", report]}},
            "nodes": {
                "gate": {"kind": "await-signal", "signal": "go", "outcomes": {"go": "report"}},
                "report": {"kind": "activity", "action": "report", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"}
            }
        }),
    );
    let mut cmd = dir.cmd(&args(&["--json", "run", "parent", "--key", "build"]));
    cmd.env("RUNSPORE_PARENT_RUN_ID", "run_stale");
    let out = common::run(cmd);
    assert_eq!(out.code(), Some(3), "{}", out.stderr);
    let build = out.json()["runId"].as_str().unwrap().to_string();
    let top_parent = std::fs::read_to_string(dir.file("top-parent")).unwrap();
    assert_eq!(top_parent, "unset", "a run with no parent names none");

    let list = dir.run(&args(&["--json", "list"])).json();
    let ticket = list["runs"].as_array().unwrap()[1]["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let signal = dir.run(&args(&["signal", &ticket, "go", "--outcome", "go"]));
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let worked = dir.run(&args(&["worker", "--run", &ticket, "--until-parked"]));
    assert_eq!(worked.code(), Some(0), "{}", worked.stderr);
    let worked = dir.run(&args(&["worker", "--run", &build, "--until-parked"]));
    assert_eq!(worked.code(), Some(0), "{}", worked.stderr);
    let runs = dir.run(&args(&["--json", "list"])).json()["runs"].clone();
    assert_eq!(row(&runs, &build)["status"], "completed", "{runs}");
}

/// The ticket run of a build: it waits at `gate` for a person, then its last step
/// signals its parent `child-done`.
fn ticket() -> Value {
    let report = format!(
        "'{BIN}' --native-kernel signal \"$RUNSPORE_PARENT_RUN_ID\" child-done --outcome ok"
    );
    json!({
        "format": "runspore.workflow/0.2", "name": "ticket", "start": "gate",
        "actions": {"report": {"kind": "command", "effect": "idempotent", "output": "none",
                               "argv": ["/bin/sh", "-c", report]}},
        "nodes": {
            "gate": {"kind": "await-signal", "signal": "go", "outcomes": {"go": "report"}},
            "report": {"kind": "activity", "action": "report", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

/// A build that starts `ticket` and waits for its `child-done`.
fn build() -> Value {
    let start = format!("'{BIN}' --json start ticket --key ticket-1 > /dev/null");
    let mut build = parent();
    build["actions"]["spawn"]["argv"] = json!(["/bin/sh", "-c", start]);
    build["nodes"]["hold"] = json!({"kind": "await-signal", "signal": "child-done",
                                    "outcomes": {"ok": "done"}});
    build
}

/// `continue <parent> --wait` works the run it started too, and returns when that run
/// needs a person: the parent's wait shows as waiting on its child, with the child's
/// commands, never the parent's signal as a decision.
#[test]
fn continue_works_the_runs_a_run_started() {
    let dir = Dir::new("continue-children");
    dir.folder("build", &build());
    dir.folder("ticket", &ticket());
    let out = dir.run(&args(&["--json", "start", "build", "--key", "build"]));
    let build = out.json()["runId"].as_str().unwrap().to_string();

    let out = dir.run(&args(&["--json", "continue", &build, "--wait"]));
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    let parked = out.json();
    assert_eq!(parked["status"], "waiting", "{parked}");
    assert_eq!(parked["needs"], "child", "{parked}");
    let ticket = parked["children"][0]["runId"].as_str().unwrap().to_string();
    assert_eq!(parked["children"][0]["status"], "waiting", "{parked}");
    assert_eq!(parked["children"][0]["needs"], "signal", "{parked}");
    let commands: Vec<&str> = parked["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(commands[0], format!("spore step show {ticket}"), "{parked}");
    assert!(
        commands[1].starts_with(&format!("spore signal {ticket} go")),
        "{parked}"
    );
    assert!(
        commands.iter().all(|c| !c.contains(&build)),
        "the parent's wait is offered as a decision: {parked}"
    );

    let list = dir.run(&args(&["--json", "list"])).json();
    assert_eq!(row(&list["runs"], &build)["needs"], "child", "{list}");
    assert_eq!(row(&list["runs"], &ticket)["needs"], "signal", "{list}");
    let text = dir.run(&args(&["continue", &build])).stdout;
    let head = format!(
        "run {build} waits at hold for signal child-done from run it started: {ticket}\n\
         run {ticket} waits at gate for signal go\n"
    );
    assert!(text.starts_with(&head), "{text}");

    let signal = dir.run(&args(&["signal", &ticket, "go", "--outcome", "go"]));
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let out = dir.run(&args(&["--json", "continue", &build, "--wait"]));
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    let done = out.json();
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["needs"], Value::Null, "{done}");
    assert_eq!(done["children"][0]["status"], "completed", "{done}");
}
