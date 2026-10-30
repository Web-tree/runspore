//! A held step marked as needing a person (spec/cli.md, Steps outside the worker): its
//! holder sets and clears the mark with `spore step needs-person`; `status`, `list
//! --unfinished` and `continue` show it, and it ends with the attempt.

mod common;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

const DOT: &str = r#"digraph "review" {
  dialect = "runspore/0.2";
  start = "review";
  done [kind=complete];
  review -> done [on="approved"];
}
"#;

/// `review`: an external, `unsafe` step for actor `reviewer`.
fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "unsafe",
                       "outcomes": ["approved"], "async": {"timeoutMs": "600000"}}
        },
        "nodes": {"review": {"action": "review"}}
    });
    std::fs::write(flow.join("settings.json"), settings.to_string()).unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

/// `spore --json …`: the exit code and the JSON it printed.
fn json(dir: &Dir, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(extra);
    let out = dir.run(&args(&all));
    (out.code(), out.json())
}

/// Starts `flow`, works it to the review step and claims it: the run and the token.
fn claimed(dir: &Dir) -> (String, String) {
    let (code, status) = json(dir, &["run", "flow", "--key", "k"]);
    assert_eq!(code, Some(6), "{status}");
    let run = status["runId"].as_str().unwrap().to_string();
    let (code, claim) = json(dir, &["step", "claim", &run, "--actor", "reviewer"]);
    assert_eq!(code, Some(0), "{claim}");
    (run, claim["token"].as_str().unwrap().to_string())
}

fn mark(dir: &Dir, token: &str, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["step", "needs-person", token];
    all.extend_from_slice(extra);
    json(dir, &all)
}

#[test]
fn a_marked_step_shows_in_status_list_and_continue_until_cleared() {
    let dir = folder("person-mark");
    let (run, token) = claimed(&dir);

    let (code, set) = mark(
        &dir,
        &token,
        &["--reason", "permission prompt", "--id", "m1"],
    );
    assert_eq!(code, Some(0), "{set}");
    assert_eq!(
        (
            &set["nodeId"],
            &set["attempt"],
            &set["held"],
            &set["disposition"]
        ),
        (&json!("review"), &json!(1), &json!(true), &json!("applied"))
    );
    assert_eq!(set["needsPerson"]["reason"], "permission prompt");
    let since = set["needsPerson"]["sinceMs"].clone();

    // A worker coming and going leaves the mark: it lives in the store.
    let (code, worked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(6), "{worked}");

    let (_, status) = json(&dir, &["status", &run]);
    let step = &status["step"];
    assert_eq!(
        (&step["state"], &step["waiting"], &step["message"]),
        (
            &json!("held"),
            &json!("person"),
            &json!("permission prompt")
        ),
        "{status}"
    );
    assert_eq!(step["waitingSinceMs"], since);
    assert!(step["waitingForMs"].is_string(), "{status}");
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("waiting    for a person since ") && text.contains(": permission prompt"),
        "{text}"
    );

    let (_, list) = json(&dir, &["list", "--unfinished"]);
    let row = &list["runs"][0];
    assert_eq!(
        (
            &row["needs"],
            &row["waiting"],
            &row["message"],
            &row["waitingSinceMs"]
        ),
        (
            &json!("held"),
            &json!("person"),
            &json!("permission prompt"),
            &since
        ),
        "{list}"
    );
    let text = dir.run(&args(&["list", "--unfinished"])).stdout;
    assert!(
        text.contains("needs a person for ") && text.contains(": permission prompt"),
        "{text}"
    );

    let (code, shown) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(6), "{shown}");
    assert_eq!(shown["step"]["waiting"], "person", "{shown}");
    let text = dir.run(&args(&["continue", &run])).stdout;
    assert!(
        text.contains("; it waits for a person since ") && text.contains(": permission prompt"),
        "{text}"
    );
    // A follower returns at once: the step needs the person, not a driver.
    let (code, followed) = json(&dir, &["continue", &run, "--wait"]);
    assert_eq!(code, Some(0), "{followed}");
    assert_eq!(
        (&followed["needs"], &followed["step"]["waiting"]),
        (&json!("held"), &json!("person")),
        "{followed}"
    );

    // A new reason keeps the time it was first marked.
    let (_, again) = mark(&dir, &token, &["--reason", "idle with a question"]);
    assert_eq!(again["needsPerson"]["sinceMs"], since, "{again}");

    for id in ["c1", "c2"] {
        let (code, cleared) = mark(&dir, &token, &["--clear", "--id", id]);
        assert_eq!(code, Some(0), "{cleared}");
        assert_eq!(
            (&cleared["held"], &cleared["needsPerson"]),
            (&json!(true), &Value::Null)
        );
    }
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["step"]["waiting"], Value::Null, "{status}");

    // The same message again changes nothing.
    let (code, replay) = mark(
        &dir,
        &token,
        &["--reason", "permission prompt", "--id", "m1"],
    );
    assert_eq!(code, Some(0), "{replay}");
    assert_eq!(replay["disposition"], "duplicate");
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["step"]["waiting"], Value::Null, "{status}");
    let (_, list) = json(&dir, &["list", "--unfinished"]);
    assert_eq!(list["runs"][0]["waiting"], Value::Null, "{list}");
}

#[test]
fn the_mark_ends_with_the_step() {
    let dir = folder("person-end");
    let (run, token) = claimed(&dir);
    let (code, _) = mark(&dir, &token, &["--reason", "permission prompt"]);
    assert_eq!(code, Some(0));
    let (code, done) = json(&dir, &["step", "complete", &token, "--outcome", "approved"]);
    assert_eq!(code, Some(0), "{done}");
    let (code, finished) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(0), "{finished}");
    assert_eq!(finished["status"], "completed");

    // A late hook may clear on every call; a late mark is refused.
    let (code, cleared) = mark(&dir, &token, &["--clear"]);
    assert_eq!(code, Some(0), "{cleared}");
    assert_eq!(
        (&cleared["held"], &cleared["needsPerson"]),
        (&json!(false), &Value::Null)
    );
    let (code, refused) = mark(&dir, &token, &["--reason", "idle"]);
    assert_eq!(code, Some(2), "{refused}");
    assert_eq!(refused["error"]["code"], "lease.lost", "{refused}");
}

#[test]
fn a_mark_needs_a_reason_or_clear_and_a_short_reason() {
    let dir = folder("person-usage");
    let (_, token) = claimed(&dir);
    let out = dir.run(&args(&["step", "needs-person", &token]));
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    let out = dir.run(&args(&[
        "step",
        "needs-person",
        &token,
        "--reason",
        "x",
        "--clear",
    ]));
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    let long = "x".repeat(1025);
    let (code, refused) = mark(&dir, &token, &["--reason", &long]);
    assert_eq!(code, Some(2), "{refused}");
    assert_eq!(refused["error"]["code"], "step.reason-too-long");
}
