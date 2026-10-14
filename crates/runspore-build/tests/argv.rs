//! A command's `argv` items may be mappings (spec/host.md §3): `spore validate` checks
//! them as mappings whose paths start with `input`, `params`, `run` or `step`, in the
//! action's `argv` and in its probe's, watch's and stop's.

use std::path::PathBuf;

use runspore_build::{check, Checked, Machine};
use serde_json::{json, Value};

const DOT: &str = "digraph \"ask\" {\n  dialect = \"runspore/0.2\";\n  start   = \"ask\";\n\n  done [kind=\"complete\"];\n\n  ask -> done;\n}\n";

/// A folder whose one step runs the action `ask`.
struct Folder(PathBuf);

impl Folder {
    fn new(action: Value) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "runspore-argv-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let settings = json!({
            "format": "runspore.workflow/0.2",
            "actions": {"ask": action},
            "nodes": {"ask": {"action": "ask", "params": {"tools": ["Read"]}}},
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

fn command(argv: Value) -> Value {
    json!({"kind": "command", "argv": argv, "effect": "idempotent"})
}

#[test]
fn a_mapped_argv_goes_into_the_package_as_written() {
    let argv = json!([
        "make",
        {"$if": [{"$get": ["step", "resumed"]}, "--resume", "--session-id"]},
        {"$uuid": [{"$get": ["run", "id"]}, {"$get": ["step", "node"]}]},
        {"$join": [{"$get": ["params", "tools"]}, ","]},
        {"$get": ["input", "repo"], "default": "."},
    ]);
    let checked = Folder::new(command(argv.clone())).check();
    let Some(built) = &checked.built else {
        panic!("not built: {}", checked.report.short());
    };
    assert_eq!(built.package["actions"]["ask"]["argv"], argv);
}

#[test]
fn argv_mappings_are_checked_with_the_action_roots() {
    for (item, code) in [
        (json!({"$get": ["nodes", "a"]}), "mapping.path-root"),
        (json!({"$get": ["from", "node"]}), "mapping.path-root"),
        (json!({"$get": "step"}), "mapping.path-not-list"),
        (json!({"$get": ["step"], "x": 1}), "mapping.get-extra-key"),
        (json!({"$if": [true, "a"]}), "mapping.operator-shape"),
        (json!({"$jion": [[], ","]}), "mapping.unknown-operator"),
    ] {
        let checked = Folder::new(command(json!(["make", item]))).check();
        assert_eq!(checked.report.codes(), [code], "{item}");
        let resolved = checked.report.resolved();
        assert!(
            resolved[0].message.contains("action \"ask\""),
            "{item}: {}",
            resolved[0].message
        );
    }
    let checked = Folder::new(command(json!(["make", {"$get": ["nodes", "a"]}]))).check();
    let resolved = checked.report.resolved();
    assert_eq!(
        resolved[0].path.as_deref(),
        Some("/actions/ask/argv/1/$get/0")
    );
    assert!(
        resolved[0]
            .message
            .contains("\"input\" | \"params\" | \"run\" | \"step\""),
        "{}",
        resolved[0].message
    );
}

#[test]
fn an_argv_item_is_a_string_or_a_mapping() {
    for item in [json!(7), json!(["a"]), json!(null), json!(true)] {
        let checked = Folder::new(command(json!(["make", item]))).check();
        assert!(checked.built.is_none(), "{item}: built");
        let resolved = checked.report.resolved();
        assert_eq!(
            resolved[0].path.as_deref(),
            Some("/actions/ask/argv/1"),
            "{item}"
        );
    }
}

#[test]
fn probe_watch_and_stop_argv_mappings_are_checked_too() {
    for program in ["probe", "watch", "stop"] {
        let mut action = command(json!(["make"]));
        action["async"] = json!({
            program: {"command": {"argv": ["check", {"$get": ["step", "attempt"]}]}}
        });
        let checked = Folder::new(action.clone()).check();
        assert!(
            checked.built.is_some(),
            "{program}: {}",
            checked.report.short()
        );
        action["async"][program]["command"]["argv"][1] = json!({"$get": ["nodes", "ask"]});
        let checked = Folder::new(action).check();
        assert_eq!(checked.report.codes(), ["mapping.path-root"], "{program}");
        let resolved = checked.report.resolved();
        assert_eq!(
            resolved[0].path,
            Some(format!(
                "/actions/ask/async/{program}/command/argv/1/$get/0"
            ))
        );
    }
}

#[test]
fn a_mapped_program_is_not_looked_up_and_a_split_keeps_the_mappings() {
    let checked = Folder::new(command(json!([{"$get": ["params", "program"]}]))).check();
    assert!(checked.built.is_some(), "{}", checked.report.short());
    assert!(
        checked.report.codes().is_empty(),
        "{:?}",
        checked.report.codes()
    );

    let resume = json!({"$if": [{"$get": ["step", "resumed"]}, "-r", "-n"]});
    let checked = Folder::new(command(json!(["make -k", resume]))).check();
    assert_eq!(checked.report.codes(), ["command.argv-spaces"]);
    let resolved = checked.report.resolved();
    let (_, settings) = checked.report.with_fix(&resolved[0].fixes[0]).unwrap();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(
        settings["actions"]["ask"]["argv"],
        json!(["make", "-k", resume])
    );
}
