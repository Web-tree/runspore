//! A stuck step through the compiled binary (spec/kernel.md 7.4, spec/cli.md): a 0.2
//! workflow whose step gives up waits for a person, who retries, completes or fails it.

mod common;

use common::{effect_argv, Dir};
use serde_json::Value;

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

const DOT: &str = r#"digraph "stuck" {
  dialect = "runspore/0.2";
  start = "work";
  done [kind=complete];
  work -> done;
}
"#;

/// `work` fails until the file `ok` exists, with two tries allowed.
fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let argv = effect_argv("work", "test -f ok || exit 3");
    let settings = format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "actions": {{
    "work": {{"kind": "command", "effect": "idempotent", "argv": {argv},
             "retry": {{"maxAttempts": 2, "backoffMs": "10"}}}}
  }},
  "nodes": {{
    "work": {{"action": "work"}},
    "done": {{"output": {{"$get": ["nodes", "work", "outcome"]}}}}
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

/// Runs `flow` under `key` until it parks; returns the exit code and the status JSON.
fn run(dir: &Dir, key: &str) -> (Option<i32>, Value) {
    let out = dir.run(&args(&["--json", "run", "flow", "--key", key]));
    (out.code(), out.json())
}

#[test]
fn a_step_out_of_tries_is_stuck_and_resolve_retry_runs_a_new_try() {
    let dir = folder("stuck-retry");
    let (code, status) = run(&dir, "k");
    assert_eq!(code, Some(4), "{status}");
    assert_eq!(status["status"], "needs-intervention");
    let invocation = &status["invocation"];
    assert_eq!(invocation["state"], "stuck");
    assert_eq!(invocation["attempt"], 2);
    assert_eq!(invocation["maxAttempts"], 2);
    assert_eq!(invocation["error"]["code"], "command.exit");
    assert_eq!(invocation["error"]["nodeId"], "work");

    let run_id = status["runId"].as_str().unwrap();
    let shown = dir.run(&args(&["--json", "status", run_id])).json();
    assert_eq!(shown["invocation"], status["invocation"]);
    let text = dir.run(&args(&["status", run_id])).stdout;
    assert!(text.contains("attempt 2 of 2 stuck"), "{text}");
    assert!(text.contains("error      command.exit: "), "{text}");

    std::fs::write(dir.file("ok"), "").unwrap();
    let resolve = dir.run(&args(&[
        "--json", "resolve", run_id, "--retry", "--id", "r1",
    ]));
    assert_eq!(resolve.json()["disposition"], "applied");
    assert_eq!(
        resolve.json()["invocationId"],
        invocation["invocationId"],
        "without an ID, resolve answers the outstanding invocation"
    );
    let (code, status) = run(&dir, "k");
    assert_eq!(code, Some(0), "{status}");
    assert_eq!(status["result"]["output"], "ok");

    let effects = dir.effects();
    let attempts: Vec<u32> = effects.iter().map(|e| e.attempt).collect();
    assert_eq!(
        attempts,
        [1, 2, 3],
        "the retry is a third try, past maxAttempts"
    );
    assert!(
        effects.iter().all(|e| e.key == effects[0].key),
        "every try has the same effect key: {effects:?}"
    );
}

#[test]
fn resolve_fail_ends_the_run_with_the_kept_error() {
    let dir = folder("stuck-fail");
    let (code, status) = run(&dir, "k");
    assert_eq!(code, Some(4), "{status}");
    let run_id = status["runId"].as_str().unwrap();
    let kept = status["invocation"]["error"].clone();

    let resolve = dir.run(&args(&["--json", "resolve", run_id, "--fail", "giving up"]));
    assert_eq!(resolve.json()["disposition"], "applied");
    let (code, status) = run(&dir, "k");
    assert_eq!(code, Some(1), "{status}");
    assert_eq!(status["result"]["error"], kept);
    assert_eq!(dir.effects().len(), 2);

    let late = dir.run(&args(&["--json", "resolve", run_id, "--retry"]));
    assert_eq!(
        late.code(),
        Some(2),
        "a finished run has no step to resolve"
    );
    assert_eq!(late.json()["error"]["code"], "run.no-invocation");
}

#[test]
fn resolve_complete_takes_the_given_outcome() {
    let dir = folder("stuck-complete");
    let (_, status) = run(&dir, "k");
    let run_id = status["runId"].as_str().unwrap();
    dir.run(&args(&["resolve", run_id, "--complete", "ok"]));
    let (code, status) = run(&dir, "k");
    assert_eq!(code, Some(0), "{status}");
    assert_eq!(status["result"]["output"], "ok");
}
