//! The queue for a worker's slots (spec/host.md §2, Dispatch; spec/cli.md, `running`):
//! a worker takes the step ready longest first, a step whose action has a higher
//! `priority` before it, and `status` shows a queued step's place.

mod common;

use std::time::Duration;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "2000",
    "--heartbeat-ms",
    "200",
];

const DOT: &str = r#"digraph "ticket" {
  dialect = "runspore/0.2";
  start = "implement";
  done [kind=complete];
  implement -> ship -> done;
}
"#;

/// A ticket workflow, `implement` then `ship`, and a worker with one slot. Every step
/// logs its node and run; `implement` then waits for the file `gate`. `ship` has
/// `priority` when it is given.
fn project(name: &str, ship_priority: Option<i32>) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let argv = |node: &str, then: &str| {
        json!([
            "/bin/sh",
            "-c",
            format!("echo \"{node} $RUNSPORE_RUN_ID\" >> \"$EFFECTS_LOG\"; {then}")
        ])
    };
    let mut ship = json!({"kind": "command", "effect": "idempotent",
                          "argv": argv("ship", "true")});
    if let Some(priority) = ship_priority {
        ship["priority"] = json!(priority);
    }
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "implement": {"kind": "command", "effect": "idempotent",
                          "argv": argv("implement", "while [ ! -f gate ]; do sleep 0.02; done")},
            "ship": ship
        },
        "nodes": {"implement": {"action": "implement"}, "ship": {"action": "ship"}}
    });
    std::fs::write(flow.join("settings.json"), settings.to_string()).unwrap();
    std::fs::create_dir_all(dir.file(".spore")).unwrap();
    std::fs::write(
        dir.file(".spore/settings.json"),
        r#"{"worker": {"slots": 1}}"#,
    )
    .unwrap();
    dir
}

/// `spore` on the project's database, so it reads the project's settings.
fn cmd(dir: &Dir, extra: &[&str]) -> std::process::Command {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    let mut cmd = dir.cmd(&all);
    cmd.env("RUNSPORE_DB", dir.file(".spore/runspore.db"));
    cmd
}

fn spore(dir: &Dir, extra: &[&str]) -> common::Out {
    common::run(cmd(dir, extra))
}

/// The `node run` lines the steps logged, in the order they started.
fn log(dir: &Dir) -> Vec<String> {
    std::fs::read_to_string(dir.file("effects.log"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..1500 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

/// Starts three runs a moment apart, runs a worker with one slot until the first
/// run's `implement` holds it and the other two wait, checks their places, opens the
/// gate, and returns the order the six steps ran in, by run 1, 2 and 3.
fn ticket_order(name: &str, ship_priority: Option<i32>) -> Vec<String> {
    let dir = project(name, ship_priority);
    let runs: Vec<String> = (1..=3)
        .map(|n| {
            std::thread::sleep(Duration::from_millis(5));
            let key = format!("ticket-{n}");
            let started = spore(&dir, &["--json", "start", "flow", "--key", &key]).json();
            started["runId"].as_str().unwrap().to_string()
        })
        .collect();
    let mut worker = cmd(&dir, &["worker"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let status = |run: &str| spore(&dir, &["--json", "status", run]).json();
    wait_for("the second and third run to queue", || {
        runs[1..]
            .iter()
            .all(|run| status(run)["running"]["state"] == "queued")
    });
    assert_eq!(status(&runs[0])["running"]["queue"], Value::Null);
    for (n, run) in runs[1..].iter().enumerate() {
        let queue = &status(run)["running"]["queue"];
        assert_eq!(*queue, json!({"place": n + 1, "of": 2}), "run {}", n + 2);
    }
    let text = spore(&dir, &["status", &runs[2]]).stdout;
    assert!(
        text.contains(
            "implement attempt 1 of 3, queued for a free worker slot, 2nd of 2 waiting since "
        ),
        "{text}"
    );

    std::fs::write(dir.file("gate"), "").unwrap();
    wait_for("six steps", || log(&dir).len() == 6);
    worker.kill().unwrap();
    worker.wait().unwrap();
    log(&dir)
        .iter()
        .map(|line| {
            let (node, run) = line.split_once(' ').unwrap();
            let n = runs.iter().position(|r| r == run).unwrap() + 1;
            format!("{node} {n}")
        })
        .collect()
}

#[test]
fn a_worker_takes_the_step_ready_longest_first() {
    assert_eq!(
        ticket_order("queue-fifo", None),
        [
            "implement 1",
            "implement 2",
            "implement 3",
            "ship 1",
            "ship 2",
            "ship 3"
        ]
    );
}

/// The feedback-E build: `ship` steps that finish work wait behind new `implement`
/// steps unless `ship` has the higher priority.
#[test]
fn a_step_of_higher_priority_goes_before_steps_ready_longer() {
    assert_eq!(
        ticket_order("queue-priority", Some(1)),
        [
            "implement 1",
            "ship 1",
            "implement 2",
            "ship 2",
            "implement 3",
            "ship 3"
        ]
    );
}
