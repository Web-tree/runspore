//! A step's `params` (spec/dot.md §4): only an activity step takes them, they are
//! checked as mappings like its input, and `$resultSchema` there becomes the schema of
//! a result-mode answer built from the step's `outputSchemas`.

use std::path::PathBuf;

use runspore_build::{check, Checked, Machine};
use serde_json::{json, Value};

const DOT: &str = "digraph \"release\" {\n  dialect = \"runspore/0.2\";\n  start   = \"build\";\n\n  build [label=\"Build\"];\n  wait  [kind=\"await-signal\"];\n  done  [kind=\"complete\"];\n\n  build -> wait [on=\"ok\"];\n  build -> done [on=\"red\"];\n  wait -> done;\n}\n";

/// A folder whose `build` step and `wait` step have the given settings.
struct Folder(PathBuf);

impl Folder {
    fn new(build: Value, wait: Value) -> Self {
        Self::with(DOT, &["ok", "red"], build, wait)
    }

    /// A folder whose `build` action declares `outcomes`, routed by `dot`.
    fn with(dot: &str, outcomes: &[&str], build: Value, wait: Value) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "runspore-params-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let settings = json!({
            "format": "runspore.workflow/0.2",
            "actions": {"build": {"kind": "command", "argv": ["make"], "effect": "idempotent",
                                  "outcomes": outcomes}},
            "nodes": {"build": build, "wait": wait},
        });
        std::fs::write(dir.join("workflow.dot"), dot).unwrap();
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

fn wait() -> Value {
    json!({"signal": "approval"})
}

fn package(checked: &Checked) -> &Value {
    match &checked.built {
        Some(built) => &built.package,
        None => panic!("not built: {}", checked.report.short()),
    }
}

#[test]
fn an_activity_step_s_params_go_into_the_package_as_written() {
    let params = json!({
        "tools": ["Read", "Edit"],
        "mode": {"$if": [{"$get": ["input", "fresh"]}, "clean", "incremental"]},
        "session": {"$uuid": [{"$get": ["run", "id"]}, "build"]},
    });
    let checked = Folder::new(json!({"action": "build", "params": params}), wait()).check();
    assert_eq!(package(&checked)["nodes"]["build"]["params"], params);
}

#[test]
fn params_on_another_kind_of_step_is_refused_with_a_fix() {
    let checked = Folder::new(
        json!({"action": "build"}),
        json!({"signal": "approval", "params": {"tools": []}}),
    )
    .check();
    assert_eq!(checked.report.codes(), ["node.field-not-allowed"]);
    let resolved = checked.report.resolved();
    let (_, settings) = checked.report.with_fix(&resolved[0].fixes[0]).unwrap();
    assert!(!settings.contains("params"), "{settings}");
}

#[test]
fn params_are_checked_as_mappings() {
    for (params, code) in [
        (
            json!({"a": {"$get": ["nodes"], "x": 1}}),
            "mapping.get-extra-key",
        ),
        (json!({"a": {"$uuid": []}}), "mapping.operator-shape"),
        (
            json!({"a": {"$json": 1, "x": 1}}),
            "mapping.operator-extra-key",
        ),
        (
            json!({"a": {"$jion": [[], ","]}}),
            "mapping.unknown-operator",
        ),
        (
            json!({"a": {"$if": [{"$get": ["step"]}, 1, 2]}}),
            "mapping.path-root",
        ),
    ] {
        let checked = Folder::new(json!({"action": "build", "params": params}), wait()).check();
        assert_eq!(checked.report.codes(), [code], "{params}");
    }
}

#[test]
fn result_schema_becomes_the_schema_of_a_result_mode_answer() {
    let changes = json!({"type": "object", "properties": {"files": {"type": "array"}},
                         "required": ["files"]});
    let build = json!({
        "action": "build",
        "outputSchemas": {"ok": changes},
        "params": {"schema": {"$resultSchema": {}},
                   "text": {"$json": {"$resultSchema": {}}},
                   "schemas": {"$outputSchemas": {}}},
        "input": {"$resultSchema": {}},
    });
    let checked = Folder::new(build, wait()).check();
    let node = &package(&checked)["nodes"]["build"];
    let schema = json!({"type": "object",
         "properties": {"outcome": {"enum": ["ok", "red"]},
                        "output": {"anyOf": [changes, {}]}},
         "required": ["outcome", "output"], "additionalProperties": false});
    assert_eq!(node["params"]["schema"], json!({"$literal": schema}));
    assert_eq!(
        node["params"]["text"],
        json!({"$json": {"$literal": schema}})
    );
    assert_eq!(
        node["params"]["schemas"],
        json!({"$literal": {"ok": changes}})
    );
    assert_eq!(node["input"], json!({"$literal": schema}));
    // A schema runspore itself can check answers against.
    runspore_schema::check(&schema, 1).unwrap();
}

#[test]
fn result_schema_of_a_single_outcome_takes_its_schema_as_the_output() {
    let changes = json!({"type": "object", "properties": {"files": {"type": "array"}},
                         "required": ["files"]});
    let build = json!({
        "action": "build",
        "outputSchemas": {"ok": changes},
        "params": {"schema": {"$resultSchema": {}}},
    });
    let dot = DOT.replace("  build -> done [on=\"red\"];\n", "");
    let checked = Folder::with(&dot, &["ok"], build, wait()).check();
    let schema = &package(&checked)["nodes"]["build"]["params"]["schema"]["$literal"];
    assert_eq!(
        *schema,
        json!({"type": "object",
               "properties": {"outcome": {"enum": ["ok"]}, "output": changes},
               "required": ["outcome", "output"], "additionalProperties": false})
    );
}

#[test]
fn result_schema_is_an_object_at_the_top_level() {
    // Model APIs refuse a schema whose top level is anyOf, oneOf or allOf.
    let build = json!({
        "action": "build",
        "outputSchemas": {"ok": {"type": "object"}, "red": {"type": "object"}},
        "params": {"schema": {"$resultSchema": {}}},
    });
    let checked = Folder::new(build, wait()).check();
    let schema = &package(&checked)["nodes"]["build"]["params"]["schema"]["$literal"];
    assert_eq!(schema["type"], "object");
    for keyword in ["anyOf", "oneOf", "allOf"] {
        assert!(schema.get(keyword).is_none(), "{schema}");
    }
}

#[test]
fn result_schema_on_a_step_without_output_schemas_is_refused() {
    let checked = Folder::new(
        json!({"action": "build", "params": {"schema": {"$resultSchema": {}}}}),
        wait(),
    )
    .check();
    assert_eq!(checked.report.codes(), ["input.marker-no-schemas"]);
    let checked = Folder::new(
        json!({"action": "build", "outputSchemas": {"ok": {"type": "object"}},
               "params": {"schema": {"$resultSchema": true}}}),
        wait(),
    )
    .check();
    assert_eq!(checked.report.codes(), ["input.marker-shape"]);
}
