//! `status --json`: `position.from` and `end` (spec/cli.md), and a step reading why it
//! was entered (`from`, `$get` with `default`, spec/kernel.md section 4).

mod common;

use common::Dir;
use serde_json::{json, Value};

/// No test here lets a lease run out, so the lease is long enough that a stalled
/// heartbeat on a busy CI machine does not expire a step and stop the run at exit 4.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "5000",
    "--heartbeat-ms",
    "100",
];

/// `test` is red once, then green. `fix` echoes its input, which says why it was
/// entered. `approve` ends at `done` or, abandoned, at `broken`.
fn coding_loop() -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "coding-loop", "start": "test",
        "actions": {
            "test": {"kind": "command", "effect": "read-only", "outcomes": ["ok", "red"],
                     "exitOutcomes": {"1": "red"},
                     "argv": ["sh", "-c", "if [ -f green ]; then echo green; else touch green; echo 'parser fails'; exit 1; fi"]},
            "fix": {"kind": "command", "effect": "idempotent", "argv": ["cat"]}
        },
        "nodes": {
            "test": {"kind": "activity", "action": "test", "outcomes": {"ok": "approve", "red": "fix"}},
            "fix": {"kind": "activity", "action": "fix", "outcomes": {"ok": "test"},
                    "input": {"why": {"$get": ["from"]},
                              "failures": {"$get": ["from", "output", "stdout"], "default": "none"},
                              "approval": {"$get": ["nodes", "approve", "output"], "default": null}}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done", "rejected": "fix", "abandoned": "broken"}},
            "done": {"kind": "complete", "output": {"$get": ["from"]}},
            "broken": {"kind": "fail", "error": {"code": "abandoned", "message": "abandoned"}}
        }
    })
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn status(dir: &Dir, run: &str) -> Value {
    dir.run(&args(&["--json", "status", run])).json()
}

/// What `fix` was given, read back from its echoed stdout.
fn fix_input(status: &Value) -> Value {
    let stdout = status["nodes"]["fix"]["output"]["stdout"].as_str().unwrap();
    serde_json::from_str(stdout).unwrap()
}

#[test]
fn status_shows_the_arrow_taken_and_the_end() {
    let dir = Dir::new("arrival");
    dir.folder("flow", &coding_loop());
    let run = dir
        .run(&args(&["--json", "start", "flow", "--key", "a"]))
        .json()["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let created = status(&dir, &run);
    assert_eq!(
        (&created["status"], &created["position"], &created["end"]),
        (&json!("created"), &Value::Null, &Value::Null)
    );

    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "a"]));
    assert_eq!(parked.code(), Some(3), "{}", parked.stdout);
    assert_eq!(
        parked.json()["position"]["from"],
        json!({"nodeId": "test", "outcome": "ok"})
    );
    let waiting = status(&dir, &run);
    assert_eq!(
        (
            &waiting["position"]["activationId"],
            &waiting["position"]["from"]
        ),
        (
            &json!("approve/1"),
            &json!({"nodeId": "test", "outcome": "ok"})
        )
    );
    let fix = fix_input(&waiting);
    assert_eq!(
        (&fix["why"]["node"], &fix["why"]["outcome"]),
        (&json!("test"), &json!("red"))
    );
    assert_eq!(fix["failures"], "parser fails\n");
    assert_eq!(fix["approval"], Value::Null);
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("approve (approve/1, from test on ok)"),
        "{text}"
    );

    // Rejected: fix is entered from the approval, and its failures fall back.
    dir.run(&args(&[
        "signal",
        &run,
        "approval",
        "--outcome",
        "rejected",
        "--data",
        r#"{"hint": "smaller"}"#,
    ]));
    dir.run(&args(&["run", "flow", "--key", "a"]));
    let again = status(&dir, &run);
    assert_eq!(again["position"]["activationId"], "approve/2");
    let fix = fix_input(&again);
    assert_eq!(
        fix["why"],
        json!({"node": "approve", "outcome": "rejected", "output": {"hint": "smaller"}})
    );
    assert_eq!(fix["failures"], "none");
    assert_eq!(fix["approval"], json!({"hint": "smaller"}));

    dir.run(&args(&[
        "signal",
        &run,
        "approval",
        "--outcome",
        "approved",
    ]));
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "a"]));
    assert_eq!(done.code(), Some(0));
    let done = status(&dir, &run);
    assert_eq!(done["status"], "completed");
    assert_eq!(done["position"], Value::Null);
    assert_eq!(
        done["end"],
        json!({"nodeId": "done", "from": {"nodeId": "approve", "outcome": "approved"}})
    );
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("end        done, from approve on approved"),
        "{text}"
    );
}

#[test]
fn the_start_node_was_entered_by_no_arrow() {
    let dir = Dir::new("arrival-start");
    let mut workflow = coding_loop();
    workflow["start"] = json!("approve");
    dir.folder("flow", &workflow);
    let out = dir.run(&args(&["--json", "run", "flow", "--key", "s"]));
    assert_eq!(out.code(), Some(3), "{}", out.stdout);
    let position = &out.json()["position"];
    assert_eq!(
        (&position["activationId"], &position["from"]),
        (&json!("approve/1"), &Value::Null)
    );
}

#[test]
fn a_failed_run_ends_at_its_fail_node_or_the_step_that_failed() {
    let dir = Dir::new("arrival-failed");
    dir.folder("flow", &coding_loop());
    let run = |key: &str| {
        dir.run(&args(&["--json", "run", "flow", "--key", key]))
            .json()["runId"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let abandoned = run("b");
    dir.run(&args(&[
        "signal",
        &abandoned,
        "approval",
        "--outcome",
        "abandoned",
    ]));
    assert_eq!(
        dir.run(&args(&["run", "flow", "--key", "b"])).code(),
        Some(1)
    );
    let failed = status(&dir, &abandoned);
    assert_eq!(failed["status"], "failed");
    assert_eq!(
        failed["end"],
        json!({"nodeId": "broken", "from": {"nodeId": "approve", "outcome": "abandoned"}})
    );

    // A step that fails with no `failed` arrow is stuck; failed by a person, the run
    // ends at that step, and the arrow is the one that led into it.
    let mut broken = coding_loop();
    broken["actions"]["fix"]["argv"] = json!(["sh", "-c", "exit 7"]);
    dir.folder("broken", &broken);
    std::fs::remove_file(dir.file("green")).unwrap();
    let out = dir.run(&args(&["--json", "run", "broken", "--key", "c"]));
    assert_eq!(out.code(), Some(4), "{}", out.stdout);
    assert_eq!(out.json()["end"], Value::Null);
    let stuck = out.json()["runId"].as_str().unwrap().to_string();
    dir.run(&args(&["resolve", &stuck, "--fail", "giving up"]));
    let out = dir.run(&args(&["--json", "run", "broken", "--key", "c"]));
    assert_eq!(out.code(), Some(1), "{}", out.stdout);
    assert_eq!(
        out.json()["end"],
        json!({"nodeId": "fix", "from": {"nodeId": "test", "outcome": "red"}})
    );
}
