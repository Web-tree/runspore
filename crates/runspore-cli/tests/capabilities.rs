//! `spore capabilities` and the handshake (spec/cli.md §Capabilities, spec/host.md §2,
//! The handshake): what this spore offers, a workflow that asks for more, and a run at
//! a step this spore cannot act on.

mod common;

use std::sync::Arc;

use common::Dir;
use runspore_host::{
    ActivityOutput, ActivityRegistry, CommandRunner, Engine, EngineConfig, NativeRunner,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use serde_json::{json, Value};

/// `spore --json --native-kernel …`: the exit code and the JSON it printed.
fn json(dir: &Dir, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json", "--native-kernel", "--poll-ms", "20"];
    all.extend_from_slice(extra);
    let out = dir.run(&all);
    (out.code(), out.json())
}

/// `git <args>` in this repository, trimmed.
fn git(args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// The version this checkout builds and the date `spore --version` adds to it: the
/// release on the commit its tag names, else a pre-release of the next minor version
/// named by its commit and dated by it.
fn built() -> (String, Option<String>) {
    let release = env!("CARGO_PKG_VERSION");
    let tag = format!("v{release}");
    if git(&["tag", "--points-at", "HEAD"])
        .lines()
        .any(|t| t == tag)
    {
        return (release.to_string(), None);
    }
    let mut parts = release.split('.');
    let (major, minor): (u64, u64) = (
        parts.next().unwrap().parse().unwrap(),
        parts.next().unwrap().parse().unwrap(),
    );
    let commit = git(&["rev-parse", "--short=7", "HEAD"]);
    let date = git(&["log", "-1", "--format=%cs", "HEAD"]);
    (format!("{major}.{}.0-dev+{commit}", minor + 1), Some(date))
}

#[test]
fn a_build_that_is_not_a_release_says_so_with_its_commit_and_date() {
    let dir = Dir::new("version");
    let (version, date) = built();
    let text = match &date {
        Some(date) => format!("{version} ({date})"),
        None => version.clone(),
    };
    assert_eq!(
        dir.run(&["--version"]).stdout.trim(),
        format!("spore {text}")
    );
    let (code, out) = json(&dir, &["capabilities"]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["version"], version);
    assert_eq!(out.get("commitDate"), date.map(Value::from).as_ref());
    let human = dir.run(&["capabilities"]).stdout;
    assert!(
        human.starts_with(&format!("version          {text}\n")),
        "{human}"
    );
}

/// The `spore` the database at `path` records as its writer.
fn recorded_writer(path: &std::path::Path) -> String {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT value FROM meta WHERE key = 'writer'", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn a_database_records_and_its_messages_name_the_version_spore_version_prints() {
    let dir = Dir::new("writer");
    let path = dir.file("runspore.db");
    let (version, _) = built();
    let this = format!("spore {version}");
    drop(SqliteStore::open(&path).unwrap());
    assert_eq!(recorded_writer(&path), version);

    // As spore 0.3.0 left it.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE workers; PRAGMA user_version = 11;
             UPDATE meta SET value = '0.3.0' WHERE key = 'writer';",
        )
        .unwrap();
    let (code, out) = json(&dir, &["list"]);
    assert_eq!(code, Some(10), "{out}");
    assert_eq!(out["error"]["code"], "schema.needs-migrate");
    let message = out["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("older than {this} (")),
        "{message}"
    );

    let text = dir.run(&["migrate"]).stdout;
    assert!(
        text.contains("from schema version 11 (spore 0.3.0) to "),
        "{text}"
    );
    assert!(text.contains(&format!("({this})")), "{text}");
    assert_eq!(recorded_writer(&path), version);
    let (code, out) = json(&dir, &["migrate"]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(
        (&out["fromWriter"], &out["writer"]),
        (&json!(version), &json!(version))
    );

    // As a newer spore leaves it.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 1000;")
        .unwrap();
    let (code, out) = json(&dir, &["list"]);
    assert_eq!(code, Some(10), "{out}");
    assert_eq!(out["error"]["code"], "schema.too-new");
    let message = out["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("written by {this}, newer than {this} (")),
        "{message}"
    );
}

#[test]
fn capabilities_lists_the_version_the_contracts_and_the_set_without_a_database() {
    let dir = Dir::new("capabilities");
    let (code, out) = json(&dir, &["capabilities"]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["version"], built().0);
    assert_eq!(
        out["contracts"],
        json!({"artifact": "0.1", "cli": "0.2", "dot": "0.2", "host": "0.2",
               "kernel": "0.2", "schema": "0.2", "store": "0.2"})
    );
    assert_eq!(
        out["actionKinds"],
        json!(["command", "component", "external", "native"])
    );
    assert_eq!(
        out["asyncKinds"],
        json!(["command", "component", "external"])
    );
    assert_eq!(out["resultChannels"], json!(["store"]));
    assert_eq!(
        out["componentWorlds"],
        json!(["runspore:runner/runner@0.1.0"])
    );
    assert!(!dir.file("runspore.db").exists());

    let text = dir.run(&["capabilities"]).stdout;
    assert!(
        text.contains("async kinds      command, component, external\n"),
        "{text}"
    );
    assert!(text.contains("result channels  store\n"), "{text}");
    assert!(
        text.contains("component worlds runspore:runner/runner@0.1.0\n"),
        "{text}"
    );
}

#[test]
fn validate_and_start_refuse_a_result_channel_this_spore_does_not_offer() {
    let dir = Dir::new("no-channel");
    let flow = dir.folder(
        "review",
        &json!({
            "format": "runspore.workflow/0.2", "name": "review", "start": "review",
            "actions": {"review": {"kind": "external", "effect": "unsafe",
                                   "async": {"channels": ["nats"]}}},
            "nodes": {
                "review": {"kind": "activity", "action": "review", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"}
            }
        }),
    );
    let flow = flow.to_str().unwrap();
    let (code, out) = json(&dir, &["validate", flow]);
    assert_eq!(code, Some(2), "{out}");
    assert_eq!(out["diagnostics"][0]["code"], "action.no-result-channel");
    let (code, out) = json(&dir, &["start", flow]);
    assert_eq!(code, Some(2), "{out}");
    assert_eq!(out["error"]["code"], "workflow.invalid");
    assert_eq!(
        out["error"]["details"]["diagnostics"][0]["code"],
        "action.no-result-channel"
    );
}

/// A run of a `native` step, started and coordinated by engines in this process: one
/// that has the function starts it, one with no `native` runner moves it to the step.
/// `spore` has no such function, so the step is one it cannot act on.
fn at_a_native_step(dir: &Dir) -> String {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let path = dir.file("runspore.db");
        let engine = |activities| {
            Engine::new(
                Arc::new(SqliteStore::open(&path).unwrap()),
                Arc::new(NativeReducer),
                activities,
                EngineConfig::default(),
            )
        };
        let native = NativeRunner::new().function("work", |_, _| async {
            ActivityOutput::Success {
                outcome: "ok".into(),
                output: Value::Null,
            }
        });
        let workflow = serde_json::to_vec(&json!({
            "format": "runspore.workflow/0.2", "name": "work", "start": "work",
            "actions": {"work": {"kind": "native", "function": "work", "effect": "pure"}},
            "nodes": {
                "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"}
            }
        }))
        .unwrap();
        let with = engine(ActivityRegistry::with_builtins(native));
        let key = with.start(&workflow, json!({}), "k").await.unwrap().key;
        let without = engine(ActivityRegistry::new().with("command", Arc::new(CommandRunner)));
        without.run_until_parked(&key).await.unwrap();
        key.run
    })
}

#[test]
fn status_and_worker_show_a_step_this_spore_cannot_act_on_and_leave_it_untouched() {
    let dir = Dir::new("capability-missing");
    let run = at_a_native_step(&dir);

    let (code, out) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(6), "{out}");
    assert_eq!(out["status"], "running");
    let step = &out["step"];
    assert_eq!(
        (&step["kind"], &step["state"], &step["attempt"]),
        (&json!("native"), &json!("capability-missing"), &json!(1))
    );
    let missing = step["missing"].as_str().unwrap();
    assert!(
        missing.contains("no native function `work` is registered"),
        "{missing}"
    );
    assert_eq!(out["running"]["state"], "queued");
    assert_eq!(out["running"]["claimed"], false);

    let text = dir.run(&["status", &run]).stdout;
    assert!(
        text.contains("step       native capability-missing, attempt 1\n"),
        "{text}"
    );
    assert!(text.contains(&format!("missing    {missing}\n")), "{text}");
    assert!(
        text.contains("queued for a spore that can run it"),
        "{text}"
    );
    assert!(!text.contains("slots "), "{text}");
}
