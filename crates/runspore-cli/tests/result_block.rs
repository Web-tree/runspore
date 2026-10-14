//! A command's `result` block through the compiled binary (spec/host.md §3,
//! `command`): the outcome and output come from nested keys of JSON stdout, the output
//! is checked against the step's `outputSchemas`, and a mapping that does not resolve,
//! a non-string outcome or an undeclared one leave the step stuck with the end of
//! stdout in the error.

mod common;

use common::Dir;
use serde_json::{json, Value};

/// `review` prints `answer.json`, shaped as `claude -p --output-format json` prints:
/// the answer under `structured_output`.
fn flow() -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "review", "start": "review",
        "actions": {"review": {
            "kind": "command", "effect": "read-only", "argv": ["sh", "-c", "cat answer.json"],
            "outcomes": ["approved", "changes"], "output": "json",
            "result": {
                "outcome": {"$get": ["stdout", "structured_output", "outcome"]},
                "output": {"$get": ["stdout", "structured_output", "output"]}
            }
        }},
        "nodes": {
            "review": {"kind": "activity", "action": "review",
                       "outcomes": {"approved": "done", "changes": "done"},
                       "outputSchemas": {"approved": {
                           "type": "object", "properties": {"summary": {"type": "string"}},
                           "required": ["summary"], "additionalProperties": false}}},
            "done": {"kind": "complete", "output": {
                "outcome": {"$get": ["nodes", "review", "outcome"]},
                "output": {"$get": ["nodes", "review", "output"]}}}
        }
    })
}

/// Runs the flow with `stdout` as what the command prints.
fn run(name: &str, stdout: &Value) -> (Option<i32>, Value) {
    let dir = Dir::new(name);
    dir.folder("flow", &flow());
    dir.write("answer.json", stdout);
    let out = dir.run(&["--json", "run", "flow"]);
    (out.code(), out.json())
}

fn claude(answer: Value) -> Value {
    json!({"type": "result", "is_error": false, "session_id": "s-1",
           "structured_output": answer})
}

#[test]
fn the_outcome_and_output_come_from_a_nested_key_of_stdout() {
    let (code, status) = run(
        "result-ok",
        &claude(json!({"outcome": "approved", "output": {"summary": "fine"}})),
    );
    assert_eq!(code, Some(0), "{status}");
    assert_eq!(
        status["result"]["output"],
        json!({"outcome": "approved", "output": {"summary": "fine"}})
    );
}

#[test]
fn the_mapped_output_is_checked_against_output_schemas() {
    let (code, status) = run(
        "result-schema",
        &claude(json!({"outcome": "approved", "output": {"summary": 12}})),
    );
    assert_eq!(code, Some(4), "{status}");
    let error = &status["invocation"]["error"];
    assert_eq!(error["code"], "output.invalid", "{status}");
    assert_eq!(error["details"]["outcome"], "approved", "{status}");
}

#[test]
fn a_result_that_does_not_resolve_is_stuck_with_the_end_of_stdout() {
    let cases = [
        (
            "result-missing",
            claude(json!({"verdict": "approved"})),
            "command.result-invalid",
            json!({"field": "outcome", "path": ["stdout", "structured_output", "outcome"]}),
        ),
        (
            "result-not-string",
            claude(json!({"outcome": true, "output": null})),
            "command.result-invalid",
            json!({"field": "outcome", "value": true}),
        ),
        (
            "result-undeclared",
            claude(json!({"outcome": "maybe", "output": null})),
            "command.undeclared-outcome",
            json!({"outcome": "maybe"}),
        ),
    ];
    for (name, stdout, code, mut details) in cases {
        let (exit, status) = run(name, &stdout);
        assert_eq!(exit, Some(4), "{name}: {status}");
        let invocation = &status["invocation"];
        assert_eq!(invocation["state"], "stuck", "{name}");
        assert_eq!(invocation["attempt"], 1, "{name}: not retried");
        assert_eq!(invocation["error"]["code"], code, "{name}");
        details["stdoutTail"] = json!(serde_json::to_string_pretty(&stdout).unwrap());
        assert_eq!(invocation["error"]["details"], details, "{name}");
    }
}
