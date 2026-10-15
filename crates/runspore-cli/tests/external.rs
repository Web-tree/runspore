//! External steps through the compiled binary (spec/cli.md, Steps outside the worker):
//! `run` stops at one with exit 6, an actor claims it with `spore step claim`, and
//! reports with `step complete` or `step fail`; a person may `step abandon` it.

mod common;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

const DOT: &str = r#"digraph "review" {
  dialect = "runspore/0.2";
  start = "review";
  done [kind=complete];
  rework [kind=fail];
  review -> done [on="approved"];
  review -> rework [on="changes"];
}
"#;

const INSTRUCTIONS: &str = "Read the change and approve it, or list what to change.\n";

/// `review`: an external, `unsafe` step for actor `reviewer` with the given deadline,
/// instructions, and a schema for `changes`.
fn folder(name: &str, timeout_ms: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    std::fs::write(flow.join("review.md"), INSTRUCTIONS).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "unsafe",
                       "outcomes": ["approved", "changes"],
                       "async": {"timeoutMs": timeout_ms}}
        },
        "nodes": {
            "review": {"action": "review", "instructions": "review.md",
                       "input": {"$get": ["input"]},
                       "outputSchemas": {"changes": {"type": "object", "required": ["list"],
                                                     "properties": {"list": {"type": "array"}}}}},
            "done": {"output": {"$get": ["nodes", "review", "output"]}},
            "rework": {"error": {"code": "review.changes", "message": "changes asked for"}}
        }
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

/// Starts `flow` under `key` and works it until it parks at the review step.
fn at_review(dir: &Dir, key: &str) -> String {
    let (code, status) = json(
        dir,
        &["run", "flow", "--input", r#"{"change": 7}"#, "--key", key],
    );
    assert_eq!(code, Some(6), "{status}");
    status["runId"].as_str().unwrap().to_string()
}

#[test]
fn run_stops_at_an_external_step_with_exit_6_and_status_shows_it() {
    let dir = folder("ext-exit6", "60000");
    let run = at_review(&dir, "k");
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["status"], "running");
    assert_eq!(
        status["step"],
        json!({"kind": "external", "actor": "reviewer", "state": "ready", "attempt": 1,
               "holder": null, "deadlineMs": null, "leaseUntilMs": null, "missing": null})
    );
    let (code, worked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(6), "{worked}");
    let text = dir.run(&args(&["status", &run]));
    assert!(text
        .stdout
        .contains("step       external ready for reviewer, attempt 1"));
    // It waits for an actor, not for a worker's slot.
    assert_eq!(status["running"]["state"], "queued", "{status}");
    assert!(
        text.stdout
            .contains("running    review attempt 1 of 1, queued for an actor to claim it since "),
        "{}",
        text.stdout
    );
    assert!(!text.stdout.contains("slots "), "{}", text.stdout);
}

#[test]
fn a_claim_takes_only_its_label_and_gives_the_actor_the_step() {
    let dir = folder("ext-claim", "60000");
    let first = at_review(&dir, "a");
    let second = at_review(&dir, "b");
    let (code, none) = json(&dir, &["step", "claim"]);
    assert_eq!((code, &none["claimed"]), (Some(0), &json!(false)), "{none}");
    let (code, refused) = json(&dir, &["step", "claim", &first]);
    assert_eq!(code, Some(2));
    assert_eq!(refused["error"]["code"], "step.not-ready");

    let holder = r#"{"pid": 42}"#;
    let (code, claim) = json(
        &dir,
        &["step", "claim", "--actor", "reviewer", "--holder", holder],
    );
    assert_eq!(code, Some(0), "{claim}");
    assert_eq!(
        (&claim["claimed"], &claim["runId"]),
        (&json!(true), &json!(first))
    );
    assert_eq!(claim["input"], json!({"change": 7}));
    assert_eq!(claim["outcomes"], json!(["approved", "changes"]));
    assert_eq!(
        claim["outputSchemas"]["changes"]["required"],
        json!(["list"])
    );
    assert_eq!(claim["resultChannel"], "store");
    let instructions = claim["instructions"].as_str().unwrap();
    assert_eq!(std::fs::read_to_string(instructions).unwrap(), INSTRUCTIONS);
    assert!(std::path::Path::new(claim["workDir"].as_str().unwrap()).is_dir());
    assert!(claim["token"].as_str().is_some_and(|t| !t.is_empty()));

    let (_, status) = json(&dir, &["status", &first]);
    let step = &status["step"];
    assert_eq!(
        (&step["state"], &step["holder"]),
        (&json!("held"), &json!({"pid": 42}))
    );
    assert!(step["deadlineMs"].is_string() && step["leaseUntilMs"].is_string());
    assert!(!status
        .to_string()
        .contains(claim["token"].as_str().unwrap()));

    let text = dir.run(&args(&["step", "claim", "--actor", "reviewer"]));
    assert_eq!(text.code(), Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains(&format!("of run {second}")),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("spore step complete"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains(INSTRUCTIONS.trim()), "{}", text.stdout);
}

#[test]
fn a_completion_is_checked_then_moves_the_run() {
    let dir = folder("ext-complete", "60000");
    let run = at_review(&dir, "k");
    let (_, claim) = json(&dir, &["step", "claim", &run, "--actor", "reviewer"]);
    let token = claim["token"].as_str().unwrap();
    let large = format!("\"{}\"", "x".repeat(70_000));
    for (outcome, output, code) in [
        ("typo", "null", "step.undeclared-outcome"),
        ("changes", "{}", "output.invalid"),
        ("approved", "1.5", "step.output-not-canonical"),
        ("approved", large.as_str(), "step.output-too-large"),
    ] {
        let complete = [
            "step",
            "complete",
            token,
            "--outcome",
            outcome,
            "--output",
            output,
        ];
        let (exit, refused) = json(&dir, &complete);
        assert_eq!(
            (exit, &refused["error"]["code"]),
            (Some(2), &json!(code)),
            "{refused}"
        );
    }
    let (_, events) = json(&dir, &["events", &run]);
    let results = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "activity.result")
        .count();
    assert_eq!(results, 0, "nothing was recorded");

    let complete = [
        "step",
        "complete",
        token,
        "--outcome",
        "approved",
        "--output",
        "{\"ok\": 1}",
    ];
    let (code, done) = json(&dir, &complete);
    assert_eq!(
        (code, &done["disposition"]),
        (Some(0), &json!("applied")),
        "{done}"
    );
    let (code, finished) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!((code, &finished["status"]), (Some(0), &json!("completed")));
    assert_eq!(finished["result"]["output"], json!({"ok": 1}));
}

#[test]
fn abandon_frees_the_step_and_a_late_report_is_refused() {
    let dir = folder("ext-abandon", "60000");
    let run = at_review(&dir, "k");
    let (code, none) = json(&dir, &["step", "abandon", &run]);
    assert_eq!(
        (code, &none["error"]["code"]),
        (Some(2), &json!("step.not-held"))
    );
    let (_, claim) = json(&dir, &["step", "claim", &run, "--actor", "reviewer"]);
    let token = claim["token"].as_str().unwrap();
    let (code, renewed) = json(&dir, &["step", "renew", token]);
    assert_eq!(code, Some(0), "{renewed}");
    assert_eq!(renewed["deadlineMs"], claim["deadlineMs"]);

    let (code, abandoned) = json(&dir, &["step", "abandon", &run]);
    assert_eq!(
        (code, &abandoned["disposition"]),
        (Some(0), &json!("applied"))
    );
    let (code, parked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(
        (code, &parked["status"]),
        (Some(4), &json!("needs-intervention"))
    );
    let late = ["step", "complete", token, "--outcome", "approved"];
    let (code, refused) = json(&dir, &late);
    assert_eq!(
        (code, &refused["error"]["code"]),
        (Some(2), &json!("lease.lost"))
    );
    let (code, bad) = json(&dir, &["step", "fail", "not-a-token", "--error", "x"]);
    assert_eq!(
        (code, &bad["error"]["code"]),
        (Some(2), &json!("step.token-invalid"))
    );
}

#[test]
fn a_passed_deadline_ends_the_attempt_and_the_effect_decides() {
    let dir = folder("ext-deadline", "300");
    let run = at_review(&dir, "k");
    let (code, _) = json(&dir, &["step", "claim", &run, "--actor", "reviewer"]);
    assert_eq!(code, Some(0));
    std::thread::sleep(std::time::Duration::from_millis(400));
    let (code, parked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(
        (code, &parked["status"]),
        (Some(4), &json!("needs-intervention"))
    );
    assert_eq!(parked["invocation"]["state"], "unknown", "{parked}");
}
