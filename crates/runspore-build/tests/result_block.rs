//! A command's `result` block (spec/host.md §3, `command`): it goes with
//! `"output": "json"` only, and its two mappings read `stdout`, `input`, `params`, `run`
//! and `step`.

use std::path::PathBuf;

use runspore_build::{check, Checked, Machine};
use serde_json::{json, Value};

const DOT: &str = "digraph \"review\" {\n  dialect = \"runspore/0.2\";\n  start   = \"review\";\n\n  review [label=\"Review\"];\n  done   [kind=\"complete\"];\n\n  review -> done [on=\"approved\"];\n  review -> done [on=\"changes\"];\n}\n";

/// A folder whose one action, `review`, has the given fields beside its kind, argv,
/// effect and outcomes.
struct Folder(PathBuf);

impl Folder {
    fn new(fields: Value) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "runspore-result-block-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut action = json!({"kind": "command", "argv": ["make"], "effect": "read-only",
                                "outcomes": ["approved", "changes"]});
        for (key, value) in fields.as_object().unwrap() {
            action[key] = value.clone();
        }
        let settings = json!({
            "format": "runspore.workflow/0.2",
            "actions": {"review": action},
            "nodes": {"review": {"action": "review"}},
        });
        std::fs::write(dir.join("workflow.dot"), DOT).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();
        Self(dir)
    }

    fn check(&self) -> Checked {
        check(&self.0, &Machine::examples()).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn block(outcome: Value, output: Value) -> Value {
    json!({"outcome": outcome, "output": output})
}

#[test]
fn a_result_block_in_json_mode_goes_into_the_package_as_written() {
    let result = block(
        json!({"$get": ["stdout", "structured_output", "outcome"]}),
        json!({"text": {"$get": ["stdout", "result"]}, "task": {"$get": ["input", "task"]},
               "tools": {"$get": ["params", "tools"]}, "run": {"$get": ["run", "id"]},
               "visit": {"$get": ["step", "visit"]}}),
    );
    let checked = Folder::new(json!({"output": "json", "result": result})).check();
    let Some(built) = &checked.built else {
        panic!("not built: {}", checked.report.short());
    };
    assert_eq!(built.package["actions"]["review"]["result"], result);
}

#[test]
fn a_result_block_outside_json_mode_is_refused_with_a_fix() {
    let result = block(json!({"$get": ["stdout", "verdict"]}), json!(null));
    for output in [None, Some("text"), Some("result"), Some("none")] {
        let mut fields = json!({ "result": result });
        if let Some(output) = output {
            fields["output"] = json!(output);
        }
        let checked = Folder::new(fields).check();
        assert_eq!(
            checked.report.codes(),
            ["command.result-not-json"],
            "{output:?}"
        );
        let resolved = checked.report.resolved();
        let (_, settings) = checked.report.with_fix(&resolved[0].fixes[0]).unwrap();
        let settings: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            settings["actions"]["review"]["output"], "json",
            "{output:?}"
        );
    }
}

#[test]
fn a_result_block_is_checked_as_mappings_over_its_own_roots() {
    for (outcome, output, code) in [
        (
            json!({"$get": ["nodes", "review", "outcome"]}),
            json!(null),
            "mapping.path-root",
        ),
        (
            json!("approved"),
            json!({"$get": ["from"]}),
            "mapping.path-root",
        ),
        (
            json!("approved"),
            json!({"$get": "stdout.x"}),
            "mapping.path-not-list",
        ),
        (
            json!({"$jion": [[], ","]}),
            json!(null),
            "mapping.unknown-operator",
        ),
        (
            json!("approved"),
            json!({"$outputSchemas": {}}),
            "mapping.unknown-operator",
        ),
    ] {
        let fields = json!({"output": "json", "result": block(outcome.clone(), output.clone())});
        let checked = Folder::new(fields).check();
        assert_eq!(checked.report.codes(), [code], "{outcome} {output}");
    }
    let fields = json!({"output": "json",
                        "result": block(json!({"$get": ["nodes", "x"]}), json!(null))});
    let text = Folder::new(fields).check().report.text("validate");
    assert!(
        text.contains(r#"The "$get" path of action "review" starts with "nodes""#),
        "{text}"
    );
    assert!(
        text.contains(r#""stdout" | "input" | "params" | "run" | "step""#),
        "{text}"
    );
}

#[test]
fn a_result_block_reads_exit_zero_so_exit_outcomes_cannot_map_it() {
    let fields = json!({"output": "json", "exitOutcomes": {"0": "approved"},
                        "result": block(json!("approved"), json!(null))});
    let checked = Folder::new(fields).check();
    assert_eq!(checked.report.codes(), ["command.result-exit-zero"]);
}

#[test]
fn a_result_block_needs_both_mappings() {
    for result in [
        json!({"outcome": "approved"}),
        json!({"outcome": "approved", "output": 1, "extra": 2}),
    ] {
        let fields = json!({"output": "json", "result": result});
        let checked = Folder::new(fields).check();
        assert!(checked.built.is_none(), "{result}");
    }
}
