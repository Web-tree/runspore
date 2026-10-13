//! Output schemas through the compiled binary (spec/schema.md, spec/cli.md): the build
//! inlines schema files, `$ref`s and the `$outputSchemas` marker; a command whose output
//! does not fit is stuck with `output.invalid`; a signal or a resolution whose data does
//! not fit exits 2 and records nothing.

mod common;

use common::{effect_argv, Dir};
use serde_json::{json, Value};

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

const DOT: &str = r#"digraph "schemas" {
  dialect = "runspore/0.2";
  start = "review";
  approve [kind="await-signal"];
  done [kind=complete];
  review -> approve;
  approve -> done [on="approved"];
  approve -> review [on="rejected"];
}
"#;

const SUMMARY: &str = r##"{
  "$defs": {"text": {"type": "string", "minLength": 1}},
  "type": "object",
  "properties": {"summary": {"$ref": "#/$defs/text", "description": "What was done."}},
  "required": ["summary"],
  "additionalProperties": false
}
"##;

/// `review` saves its input to `input.json` and prints `answer.json` as its output.
fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(flow.join("schemas")).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    std::fs::write(flow.join("schemas/summary.json"), SUMMARY).unwrap();
    let argv = effect_argv("review", "cat > input.json; cat answer.json");
    let settings = format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "actions": {{
    "review": {{"kind": "command", "effect": "idempotent", "output": "json", "argv": {argv}}}
  }},
  "nodes": {{
    "review": {{"action": "review", "outputSchemas": {{"ok": "schemas/summary.json"}},
               "input": {{"schemas": {{"$outputSchemas": {{}}}},
                         "kept": {{"$literal": {{"$outputSchemas": {{}}}}}}}}}},
    "approve": {{"signal": "approval", "outputSchemas": {{"rejected": {{
      "type": "object", "properties": {{"reason": {{"type": "string", "minLength": 1}}}},
      "required": ["reason"], "additionalProperties": false}}}}}},
    "done": {{"output": {{"$get": ["nodes", "review", "output"]}}}}
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

fn answer(dir: &Dir, value: Value) {
    dir.write("answer.json", &value);
}

fn event_count(dir: &Dir, run: &str) -> usize {
    let events = dir.run(&args(&["--json", "events", run])).json();
    events["events"].as_array().unwrap().len()
}

fn summary_schema() -> Value {
    json!({"type": "object", "additionalProperties": false, "required": ["summary"],
           "properties": {"summary": {"type": "string", "minLength": 1,
                                      "description": "What was done."}}})
}

#[test]
fn every_result_is_checked_against_its_outcome_schema() {
    let dir = folder("schemas-flow");
    let started = dir.run(&args(&["--json", "start", "flow", "--key", "k"]));
    let run = started.json()["runId"].as_str().unwrap().to_string();
    let run = run.as_str();

    // A signal is checked even before the run reaches its node.
    let early = dir.run(&args(&[
        "signal",
        run,
        "approval",
        "--outcome",
        "rejected",
        "--data",
        "{\"reason\": \"\"}",
    ]));
    assert_eq!(early.code(), Some(2), "{}", early.stderr);
    assert!(
        early
            .stderr
            .contains("/reason: expected at least 1 characters, got 0"),
        "{}",
        early.stderr
    );
    assert_eq!(
        event_count(&dir, run),
        1,
        "a refused signal is not recorded"
    );

    // A command's output that does not fit: not retryable, so the step is stuck.
    answer(&dir, json!({"summary": 12}));
    let stuck = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    let status = stuck.json();
    assert_eq!(stuck.code(), Some(4), "{status}");
    let error = &status["invocation"]["error"];
    assert_eq!(error["code"], "output.invalid", "{status}");
    assert_eq!(
        error["details"],
        json!({"outcome": "ok", "errors": [
            {"path": "/summary", "message": "expected type \"string\", got integer (12)"}]})
    );
    assert_eq!(
        dir.effects().len(),
        1,
        "an output that does not fit is not retried"
    );

    // The build inlined the file and its `$ref`, and the marker became a copy of them.
    let input: Value =
        serde_json::from_slice(&std::fs::read(dir.file("input.json")).unwrap()).unwrap();
    assert_eq!(
        input,
        json!({"schemas": {"ok": summary_schema()}, "kept": {"$outputSchemas": {}}})
    );

    // A resolution is checked the same way and refused whole.
    let before = event_count(&dir, run);
    let bad = dir.run(&args(&[
        "resolve",
        run,
        "--complete",
        "ok",
        "--output",
        "{\"summary\": \"\", \"x\": 1}",
    ]));
    assert_eq!(bad.code(), Some(2), "{}", bad.stderr);
    assert_eq!(
        bad.stderr,
        "spore: output.invalid: the data for outcome \"ok\" does not fit its schema\n\
         /summary: expected at least 1 characters, got 0\n\
         /x: not allowed; allowed properties: summary\n"
    );
    assert_eq!(event_count(&dir, run), before);
    let good = dir.run(&args(&[
        "resolve",
        run,
        "--complete",
        "ok",
        "--output",
        "{\"summary\": \"by hand\"}",
    ]));
    assert_eq!(good.code(), Some(0), "{}", good.stderr);
    let waiting = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(waiting.code(), Some(3), "{}", waiting.stdout);

    // A signal without data is null, which does not fit.
    let missing = dir.run(&args(&[
        "--json",
        "signal",
        run,
        "approval",
        "--outcome",
        "rejected",
    ]));
    assert_eq!(missing.code(), Some(2));
    assert_eq!(
        missing.json()["error"]["details"],
        json!({"outcome": "rejected", "errors": [
            {"path": "", "message": "expected type \"object\", got null"}]})
    );
    let text = dir.run(&args(&["signal", run, "approval", "--outcome", "rejected"]));
    assert!(
        text.stderr
            .contains("\n(root): expected type \"object\", got null"),
        "{}",
        text.stderr
    );
    // An outcome with no schema is not checked.
    answer(&dir, json!({"summary": "second look"}));
    let rejected = dir.run(&args(&[
        "signal",
        run,
        "approval",
        "--outcome",
        "rejected",
        "--data",
        "{\"reason\": \"weak\"}",
    ]));
    assert_eq!(rejected.code(), Some(0), "{}", rejected.stderr);
    let again = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(again.code(), Some(3), "{}", again.stdout);
    let approved = dir.run(&args(&["signal", run, "approval", "--outcome", "approved"]));
    assert_eq!(approved.code(), Some(0), "{}", approved.stderr);
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(
        (done.code(), done.json()["result"]["output"].clone()),
        (Some(0), json!({"summary": "second look"}))
    );
}
