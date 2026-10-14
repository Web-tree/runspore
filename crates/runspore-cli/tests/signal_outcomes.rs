//! `spore signal` and the outcomes of the node waiting for it (spec/cli.md, spec/host.md
//! §2): while the run waits at an await-signal node for that signal, an outcome the node
//! does not route exits 2 with `signal.outcome-unrouted`, prints the outcomes the node
//! accepts and records nothing. A signal sent before the run waits for it is recorded as
//! before, and the kernel drops it with the `signal.outcome-unrouted` diagnostic.

mod common;

use common::Dir;
use serde_json::json;

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

const DOT: &str = r#"digraph "signals" {
  dialect = "runspore/0.2";
  start = "review";
  review [kind="await-signal"];
  done [kind=complete];
  dropped [kind=complete];
  review -> done [on="approved"];
  review -> dropped [on="rejected"];
}
"#;

const SETTINGS: &str = r#"{
  "format": "runspore.workflow/0.2",
  "actions": {},
  "nodes": {
    "review": {"signal": "approval"},
    "done": {"output": "approved"},
    "dropped": {"output": "rejected"}
  }
}
"#;

fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    std::fs::write(flow.join("settings.json"), SETTINGS).unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn event_count(dir: &Dir, run: &str) -> usize {
    let events = dir.run(&args(&["--json", "events", run])).json();
    events["events"].as_array().unwrap().len()
}

#[test]
fn a_waiting_run_refuses_an_outcome_its_node_does_not_route() {
    let dir = folder("signal-unrouted");
    let waiting = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(waiting.code(), Some(3), "{}", waiting.stdout);
    let run = waiting.json()["runId"].as_str().unwrap().to_string();
    let run = run.as_str();
    let before = event_count(&dir, run);

    let refused = dir.run(&args(&["signal", run, "approval", "--outcome", "approve"]));
    assert_eq!(refused.code(), Some(2), "{}", refused.stderr);
    assert_eq!(
        refused.stderr,
        "spore: signal.outcome-unrouted: the run waits at review for signal approval, \
         which does not route outcome \"approve\"\n\
         accepted outcomes: approved, rejected\n"
    );
    let json = dir.run(&args(&["--json", "signal", run, "approval"]));
    assert_eq!(json.code(), Some(2), "{}", json.stdout);
    let error = &json.json()["error"];
    assert_eq!(error["code"], "signal.outcome-unrouted");
    assert_eq!(
        error["details"],
        json!({"nodeId": "review", "signal": "approval", "outcome": "received",
               "outcomes": ["approved", "rejected"]})
    );
    assert_eq!(
        event_count(&dir, run),
        before,
        "a refused signal is not recorded"
    );

    let approved = dir.run(&args(&["signal", run, "approval", "--outcome", "approved"]));
    assert_eq!(approved.code(), Some(0), "{}", approved.stderr);
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(
        (done.code(), done.json()["result"]["output"].clone()),
        (Some(0), json!("approved"))
    );
}

#[test]
fn a_signal_sent_before_the_run_waits_is_recorded_and_dropped() {
    let dir = folder("signal-early");
    let started = dir.run(&args(&["--json", "start", "flow", "--key", "k"]));
    let run = started.json()["runId"].as_str().unwrap().to_string();
    let run = run.as_str();

    let early = dir.run(&args(&["signal", run, "approval", "--outcome", "approve"]));
    assert_eq!(early.code(), Some(0), "{}", early.stderr);
    let waiting = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(waiting.code(), Some(3), "{}", waiting.stdout);
    let events = dir.run(&args(&["--json", "events", run])).json();
    let codes: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["diagnostics"].as_array().cloned().unwrap_or_default())
        .map(|d| d["code"].clone())
        .collect();
    assert!(
        codes.contains(&json!("signal.outcome-unrouted")),
        "{events}"
    );
}
