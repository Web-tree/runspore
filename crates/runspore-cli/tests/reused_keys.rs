//! A start key, signal ID or resolve ID used again for a different request: the error
//! names the run it already names and the ways on (spec/cli.md, Keys used again).

mod common;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

/// Parks at once at `approve`; `label` changes the workflow without changing its shape.
fn flow(label: &str) -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "reuse", "start": "approve",
        "nodes": {
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done"}},
            "done": {"kind": "complete", "output": label}
        }
    })
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn run_id(dir: &Dir, input: &str) -> String {
    let out = dir.run(&args(&[
        "--json", "run", "flow", "--key", "k", "--input", input,
    ]));
    assert_eq!(out.code(), Some(3), "{}", out.stdout);
    out.json()["runId"].as_str().unwrap().to_string()
}

#[test]
fn a_start_key_with_another_input_names_its_run_and_the_ways_on() {
    let dir = Dir::new("key-input");
    dir.folder("flow", &flow("done"));
    let run = run_id(&dir, r#"{"ticket": "it's 1"}"#);

    let out = dir.run(&args(&[
        "run",
        "flow",
        "--key",
        "k",
        "--input",
        r#"{"ticket": 2}"#,
    ]));
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    let stderr = &out.stderr;
    assert!(stderr.starts_with("spore: run.key-reused: "), "{stderr}");
    assert!(
        stderr.contains(&format!("run {run}")) && stderr.contains("another input"),
        "{stderr}"
    );
    assert!(
        stderr.contains(r#"spore run flow --key k --input '{"ticket":"it'\''s 1"}'"#),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("spore worker --run {run} --until-parked")),
        "{stderr}"
    );
    assert!(!stderr.contains("digest-mismatch"), "{stderr}");

    let json = dir
        .run(&args(&[
            "--json", "start", "flow", "--key", "k", "--input", "{}",
        ]))
        .json();
    let error = &json["error"];
    assert_eq!(error["code"], "run.key-reused");
    assert_eq!(
        error["details"],
        json!({
            "runId": run, "startKey": "k", "differs": ["input"],
            "input": {"ticket": "it's 1"},
            "commands": [
                r#"spore start flow --key k --input '{"ticket":"it'\''s 1"}'"#,
                format!("spore worker --run {run} --until-parked"),
            ]
        })
    );

    // Both ways on work.
    let again = dir.run(&args(&[
        "--json",
        "run",
        "flow",
        "--key",
        "k",
        "--input",
        r#"{"ticket":"it's 1"}"#,
    ]));
    assert_eq!(
        (again.code(), again.json()["runId"].clone()),
        (Some(3), json!(run))
    );
    let worker = dir.run(&args(&["worker", "--run", &run, "--until-parked"]));
    assert_eq!(worker.code(), Some(3), "{}", worker.stderr);
}

#[test]
fn a_start_key_with_a_changed_workflow_points_at_the_worker() {
    let dir = Dir::new("key-workflow");
    dir.folder("flow", &flow("done"));
    let run = run_id(&dir, "{}");
    dir.folder("flow", &flow("changed"));

    let out = dir.run(&args(&[
        "--json", "run", "flow", "--key", "k", "--input", "{}",
    ]));
    assert_eq!(out.code(), Some(2), "{}", out.stdout);
    let error = &out.json()["error"];
    assert_eq!(error["code"], "run.key-reused");
    assert_eq!(error["details"]["differs"], json!(["workflow"]));
    assert_eq!(
        error["details"]["commands"],
        json!([format!("spore worker --run {run} --until-parked")])
    );
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("another version of the workflow"),
        "{message}"
    );

    let both = dir.run(&args(&[
        "--json", "run", "flow", "--key", "k", "--input", "[1]",
    ]));
    assert_eq!(
        both.json()["error"]["details"]["differs"],
        json!(["input", "workflow"])
    );
}

#[test]
fn a_signal_or_resolve_id_used_for_another_request_says_so() {
    let dir = Dir::new("ids");
    dir.folder("flow", &flow("done"));
    let run = run_id(&dir, "{}");
    let signal = |data: &str| {
        dir.run(&args(&[
            "--json",
            "signal",
            &run,
            "approval",
            "--outcome",
            "approved",
            "--data",
            data,
            "--id",
            "m1",
        ]))
    };
    assert_eq!(signal("1").code(), Some(0));
    let out = signal("2");
    assert_eq!(out.code(), Some(2), "{}", out.stdout);
    let error = &out.json()["error"];
    assert_eq!(error["code"], "signal.id-reused");
    assert_eq!(error["details"], json!({"runId": run, "id": "m1"}));
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("run {run}")) && message.contains("--id"),
        "{message}"
    );

    let resolve = |how: &[&str]| {
        let mut all = args(&["--json", "resolve", &run, "inv_x", "--id", "r1"]);
        all.extend_from_slice(how);
        dir.run(&all)
    };
    assert_eq!(resolve(&["--retry"]).code(), Some(0));
    let out = resolve(&["--fail", "no"]);
    assert_eq!(out.code(), Some(2), "{}", out.stdout);
    let error = &out.json()["error"];
    assert_eq!(error["code"], "resolve.id-reused");
    assert_eq!(error["details"], json!({"runId": run, "id": "r1"}));
}
