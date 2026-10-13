//! Loop limits through the compiled binary (spec/kernel.md 7.4, spec/dot.md §4,
//! spec/cli.md): a folder whose test step may send the run back twice in a row, then
//! takes its exhausted edge to a person.

mod common;

use common::{effect_argv, Dir};
use serde_json::{json, Value};

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

const DOT: &str = r#"digraph "loop" {
  dialect = "runspore/0.2";
  start = "implement";
  stuck   [kind="await-signal"];
  approve [kind="await-signal"];
  done    [kind=complete];
  stopped [kind=fail];
  implement -> test;
  test -> approve   [on="ok"];
  test -> implement [on="red"];
  test -> stuck     [on="exhausted"];
  stuck -> implement [on="keep-going"];
  stuck -> stopped   [on="stop"];
  approve -> done      [on="approved"];
  approve -> implement [on="rejected"];
  approve -> stopped   [on="exhausted"];
}
"#;

/// `test` is red until the file `pass` exists.
fn folder(name: &str, test_limits: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let implement = effect_argv("implement", "true");
    let test = effect_argv("test", "test -f pass");
    let settings = format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "actions": {{
    "implement": {{"kind": "command", "effect": "idempotent", "argv": {implement}}},
    "test": {{"kind": "command", "effect": "read-only", "argv": {test},
             "outcomes": ["ok", "red"], "exitOutcomes": {{"1": "red"}}}}
  }},
  "nodes": {{
    "implement": {{"action": "implement"}},
    "test": {{"action": "test", "loopLimits": {test_limits}}},
    "stuck": {{"signal": "tests-stuck"}},
    "approve": {{"signal": "approval", "loopLimits": {{"rejected": 2}}}},
    "stopped": {{"error": {{"code": "loop.stopped", "message": "stopped"}}}}
  }}
}}
"#
    );
    std::fs::write(flow.join("settings.json"), settings).unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn steps(dir: &Dir) -> Vec<String> {
    dir.effects().into_iter().map(|e| e.node).collect()
}

/// Delivers a signal and works the run until it parks; returns the status JSON.
fn answer(dir: &Dir, run: &str, name: &str, outcome: &str) -> Value {
    let signal = dir.run(&args(&[
        "--json",
        "signal",
        run,
        name,
        "--outcome",
        outcome,
    ]));
    assert_eq!(signal.code(), Some(0), "{}", signal.stderr);
    let worked = dir.run(&args(&["--json", "worker", "--run", run, "--until-parked"]));
    worked.json()
}

#[test]
fn a_used_up_loop_limit_takes_the_exhausted_edge() {
    let dir = folder("loop-exhausted", r#"{"red": 2}"#);
    let valid = dir.run(&["--json", "validate", "flow"]);
    assert_eq!(valid.code(), Some(0), "{}", valid.stderr);

    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    let status = parked.json();
    assert_eq!(parked.code(), Some(3), "{status}");
    assert_eq!(status["position"]["nodeId"], "stuck");
    assert_eq!(
        status["position"]["from"],
        json!({"nodeId": "test", "outcome": "exhausted"})
    );
    assert_eq!(status["nodes"]["test"]["outcome"], "red", "the real result");
    assert_eq!(status["streaks"], json!({}), "taking exhausted resets");
    assert_eq!(
        steps(&dir),
        [
            "implement",
            "test",
            "implement",
            "test",
            "implement",
            "test"
        ]
    );
    let run = status["runId"].as_str().unwrap().to_string();

    // The next red counts afresh: one streak of one, with its limit.
    std::fs::write(dir.file("pass"), "").unwrap();
    let status = answer(&dir, &run, "tests-stuck", "keep-going");
    assert_eq!(status["position"]["nodeId"], "approve", "{status}");
    assert_eq!(status["streaks"], json!({}));
    let status = answer(&dir, &run, "approval", "rejected");
    assert_eq!(status["position"]["nodeId"], "approve", "{status}");
    assert_eq!(
        status["streaks"],
        json!({"approve": {"outcome": "rejected", "count": 1, "limit": 2}})
    );
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("streak     approve rejected 1 of 2 in a row"),
        "{text}"
    );

    answer(&dir, &run, "approval", "rejected");
    let status = answer(&dir, &run, "approval", "rejected");
    assert_eq!(status["status"], "failed", "{status}");
    assert_eq!(
        status["end"],
        json!({"nodeId": "stopped", "from": {"nodeId": "approve", "outcome": "exhausted"}})
    );
}

/// W12 and W14 are the kernel's, reported after the build; the node is named.
#[test]
fn a_loop_limit_validation_error_names_the_node() {
    // The build words each problem itself; the kernel's rule numbers never show.
    let check = |dir: &Dir, wanted: &[&str]| {
        let out = dir.run(&["validate", "flow"]);
        assert_eq!(out.code(), Some(2), "{}", out.stderr);
        for w in wanted {
            assert!(out.stderr.contains(w), "no {w:?} in {}", out.stderr);
        }
        assert!(
            !out.stderr.contains("W12") && !out.stderr.contains("kernel"),
            "{}",
            out.stderr
        );
    };
    let dir = folder("loop-invalid", r#"{"red": 16}"#);
    check(
        &dir,
        &[
            "node.limit-too-high",
            "flow/settings.json:",
            "step \"test\"",
        ],
    );

    let dir = folder("loop-no-limit", r#"{"red": 2}"#);
    let settings = std::fs::read_to_string(dir.file("flow/settings.json")).unwrap();
    let settings = settings.replace(r#", "loopLimits": {"red": 2}"#, "");
    std::fs::write(dir.file("flow/settings.json"), settings).unwrap();
    check(
        &dir,
        &[
            "dot.exhausted-arrow",
            "flow/workflow.dot:",
            "\"loopLimits\"",
        ],
    );

    let dir = folder("loop-orphan", r#"{"red": 2}"#);
    let dot = DOT.replace(
        "  stopped [kind=fail];",
        "  stopped [kind=fail];\n  orphan [kind=complete];",
    );
    std::fs::write(dir.file("flow/workflow.dot"), dot).unwrap();
    check(
        &dir,
        &["dot.unreachable", "Step \"orphan\" can't be reached"],
    );
}
