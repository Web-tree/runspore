//! A command that fails keeps the end of both streams in its error (spec/host.md §3,
//! `command`), and `status` and `events` show them: `claude -p --output-format json`
//! prints its error on stdout and leaves stderr empty.

mod common;

use common::Dir;
use serde_json::{json, Value};

const SAID: &str = r#"{"is_error":true,"result":"Credit balance is too low"}"#;

fn flow() -> Value {
    let script = format!("printf '%s\\n' '{SAID}'; echo 'retry later' >&2; exit 1");
    json!({
        "format": "runspore.workflow/0.2", "name": "agent", "start": "agent",
        "actions": {"agent": {
            "kind": "command", "effect": "read-only", "argv": ["sh", "-c", script],
            "output": "json"
        }},
        "nodes": {
            "agent": {"kind": "activity", "action": "agent", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

/// The error the run kept for the failed command, from `status --json`.
fn kept_error(status: &Value) -> &Value {
    let invocation = &status["invocation"]["error"];
    if invocation.is_null() {
        &status["result"]["error"]
    } else {
        invocation
    }
}

#[test]
fn status_and_events_show_the_end_of_stdout_and_stderr() {
    let dir = Dir::new("command-failure");
    dir.folder("flow", &flow());
    let out = dir.run(&["--json", "run", "flow"]);
    let status = out.json();
    let run = status["runId"].as_str().unwrap().to_string();
    let error = kept_error(&status);
    assert_eq!(error["code"], "command.exit", "{status}");
    assert_eq!(
        error["details"],
        json!({"exitCode": 1, "stdoutTail": format!("{SAID}\n"), "stderrTail": "retry later\n"}),
        "{status}"
    );

    let text = dir.run(&["status", &run]).stdout;
    assert!(text.contains(&format!("stdout     {SAID}\n")), "{text}");
    assert!(text.contains("stderr     retry later\n"), "{text}");

    let events = dir.run(&["--json", "events", &run]).json();
    let failed = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "activity.result")
        .expect("an activity result");
    assert_eq!(failed["payload"]["error"]["details"], error["details"]);

    let text = dir.run(&["events", &run]).stdout;
    assert!(
        text.contains(&format!("      stdout     {SAID}\n")),
        "{text}"
    );
    assert!(text.contains("      stderr     retry later"), "{text}");
}
