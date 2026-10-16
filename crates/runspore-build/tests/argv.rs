//! A command's `argv` items may be mappings (spec/host.md §3): `spore validate` checks
//! them as mappings whose paths start with `input`, `params`, `run` or `step`, in the
//! action's `argv` and in its probe's, watch's and stop's. A path into `input` or
//! `params` is checked against what each step that runs the action gives it.

use std::path::PathBuf;

use runspore_build::{check, Checked, Machine};
use serde_json::{json, Value};

const DOT: &str = "digraph \"ask\" {\n  dialect = \"runspore/0.2\";\n  start   = \"ask\";\n\n  done [kind=\"complete\"];\n\n  ask -> done;\n}\n";

/// A folder whose one step runs the action `ask`.
struct Folder(PathBuf);

impl Folder {
    fn new(action: Value) -> Self {
        Self::with_step(
            action,
            json!({"action": "ask", "params": {"tools": ["Read"], "program": "claude"}}),
        )
    }

    /// A folder whose step `ask` has the settings `step`.
    fn with_step(action: Value, step: Value) -> Self {
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
            "nodes": {"ask": step},
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
        // An external action: this spore runs no `command` async (spec/host.md §2, The
        // handshake).
        let mut action = json!({"kind": "external", "effect": "unsafe"});
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

/// An async command's `async` has the shapes the host takes (spec/dot.md §4, host.md §3,
/// Async commands): tagged programs, the `store` channel, a lease shorter than the
/// deadline. A component program and a channel this spore does not offer are refused
/// by the handshake (host.md §2).
#[test]
fn an_async_command_takes_tagged_programs_and_the_store_channel() {
    let mut action = command(json!(["start"]));
    action["async"] = json!({"timeoutMs": "60000", "leaseMs": "5000", "channels": ["store"],
                             "probe": {"command": {"argv": ["check"], "timeoutMs": "500"}},
                             "watch": {"command": {"argv": ["wait"], "env": {"A": "1"}}}});
    let checked = Folder::new(action.clone()).check();
    assert!(checked.built.is_some(), "{}", checked.report.short());

    for (field, value, code) in [
        (
            "probe",
            json!({"component": {"ref": "./p.wasm", "export": "probe"}}),
            "action.component-unsupported",
        ),
        (
            "probe",
            json!({"commnd": {"argv": ["check"]}}),
            "settings.typo-key",
        ),
        ("stop", json!({}), "settings.empty"),
        ("watch", json!({"command": {"argv": []}}), "settings.empty"),
        ("channels", json!(["nats"]), "action.no-result-channel"),
        ("channels", json!([]), "settings.empty"),
        ("leaseMs", json!("60000"), "action.lease-too-long"),
    ] {
        let mut bad = action.clone();
        bad["async"][field] = value;
        let checked = Folder::new(bad).check();
        assert!(
            checked.report.codes().iter().any(|c| c == code),
            "{field}: {:?}",
            checked.report.codes()
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

/// A `$get` in `argv` that names nothing the step gives fails the step every time it
/// starts (`mapping.missing-path`), so `spore validate` reports it with the step and
/// the path.
#[test]
fn an_argv_path_the_step_does_not_give_is_reported_with_the_step() {
    let item = json!({"$get": ["input", "x"]});
    let checked =
        Folder::with_step(command(json!(["make", item])), json!({"action": "ask"})).check();
    assert!(checked.built.is_none(), "built");
    assert_eq!(checked.report.codes(), ["mapping.missing-path"]);
    let resolved = checked.report.resolved();
    assert_eq!(
        resolved[0].path.as_deref(),
        Some("/actions/ask/argv/1/$get/1")
    );
    for words in [r#"step "ask""#, r#"action "ask""#, r#"["input", "x"]"#] {
        assert!(
            resolved[0].message.contains(words),
            "{words}: {}",
            resolved[0].message
        );
    }
    // The fix maps the key from the run's input.
    let (_, settings) = checked.report.with_fix(&resolved[0].fixes[0]).unwrap();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    assert_eq!(
        settings["nodes"]["ask"]["input"],
        json!({"x": {"$get": ["input", "x"]}})
    );
}

#[test]
fn argv_paths_are_checked_against_the_steps_input_and_params() {
    let reported = |step: &Value, item: &Value| {
        let checked = Folder::with_step(command(json!(["make", item])), step.clone()).check();
        checked
            .report
            .codes()
            .iter()
            .any(|c| c == "mapping.missing-path")
    };
    let missing = [
        (json!({"input": {"y": 1}}), json!({"$get": ["input", "x"]})),
        (
            json!({"input": {"x": {"y": 1}}}),
            json!({"$get": ["input", "x", "z"]}),
        ),
        (
            json!({"input": {"x": [1]}}),
            json!({"$get": ["input", "x", 1]}),
        ),
        (
            json!({"input": {"x": "a"}}),
            json!({"$get": ["input", "x", "y"]}),
        ),
        (
            json!({"input": {"x": {"$literal": {"y": 1}}}}),
            json!({"$get": ["input", "x", "z"]}),
        ),
        (
            json!({"input": {"x": {"$json": {"y": 1}}}}),
            json!({"$get": ["input", "x", "y"]}),
        ),
        (json!({}), json!({"$get": ["params", "tools"]})),
        (
            json!({"params": {"t": 1}}),
            json!({"$join": [{"$get": ["params", "tools"]}, ","]}),
        ),
        // The condition of `$if` is always evaluated.
        (
            json!({}),
            json!({"$if": [{"$get": ["input", "resume"]}, "-r", "-n"]}),
        ),
    ];
    let given = [
        (json!({"input": {"x": 1}}), json!({"$get": ["input", "x"]})),
        (json!({}), json!({"$get": ["input"]})),
        (json!({}), json!({"$get": ["input", "x"], "default": "."})),
        // What another mapping gives is known only at run time.
        (
            json!({"input": {"$get": ["nodes", "a"]}}),
            json!({"$get": ["input", "x"]}),
        ),
        (
            json!({"input": {"x": {"$get": ["input", "x"]}}}),
            json!({"$get": ["input", "x", "y"]}),
        ),
        (
            json!({"input": {"x": {"$if": [true, {"y": 1}, {}]}}}),
            json!({"$get": ["input", "x", "y"]}),
        ),
        (
            json!({"input": {"x": {"$literal": {"y": 1}}}}),
            json!({"$get": ["input", "x", "y"]}),
        ),
        (
            json!({"input": {"x": ["a", "b"]}}),
            json!({"$get": ["input", "x", 1]}),
        ),
        // A branch of `$if` is evaluated only when its condition picks it.
        (
            json!({"params": {}}),
            json!({"$if": [true, {"$get": ["params", "a"]}, "b"]}),
        ),
        (json!({}), json!({"$get": ["step", "attempt"]})),
    ];
    for (mut step, item) in missing {
        step["action"] = json!("ask");
        assert!(reported(&step, &item), "{step} {item}: not reported");
    }
    for (mut step, item) in given {
        step["action"] = json!("ask");
        assert!(!reported(&step, &item), "{step} {item}: reported");
    }
}

#[test]
fn probe_watch_and_stop_argv_paths_are_checked_against_the_step() {
    for program in ["probe", "watch", "stop"] {
        let mut action = json!({"kind": "external", "effect": "unsafe"});
        action["async"] = json!({
            program: {"command": {"argv": ["check", {"$get": ["input", "id"]}]}}
        });
        let checked = Folder::with_step(action, json!({"action": "ask"})).check();
        assert_eq!(
            checked.report.codes(),
            ["mapping.missing-path"],
            "{program}"
        );
        let resolved = checked.report.resolved();
        assert_eq!(
            resolved[0].path,
            Some(format!(
                "/actions/ask/async/{program}/command/argv/1/$get/1"
            ))
        );
    }
}
