//! What a run needs, through the compiled binary (spec/cli.md, What a run needs):
//! `list --unfinished`, `continue [<runId>]` and `step show [<runId>]`.

mod common;

use std::path::Path;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

/// `build` runs a command, then `approve` waits for a person's decision.
const LOOP_DOT: &str = r#"digraph "loop" {
  dialect = "runspore/0.2";
  start = "build";
  approve [kind="await-signal"];
  done [kind=complete];
  build -> approve;
  approve -> done [on="approved"];
  approve -> build [on="rejected"];
}
"#;

const APPROVE: &str = "Read the build and approve it.\n";

/// `review`: an external, `unsafe` step for actor `reviewer`.
const REVIEW_DOT: &str = r#"digraph "review" {
  dialect = "runspore/0.2";
  start = "review";
  done [kind=complete];
  rework [kind=fail];
  review -> done [on="approved"];
  review -> rework [on="changes"];
}
"#;

/// Writes the `loop` workflow folder at `path`, its `build` step running `argv`.
fn loop_folder(path: &Path, argv: Value) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(path.join("workflow.dot"), LOOP_DOT).unwrap();
    std::fs::write(path.join("approve.md"), APPROVE).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {"build": {"kind": "command", "effect": "idempotent", "argv": argv}},
        "nodes": {
            "build": {"action": "build"},
            "approve": {"signal": "approval", "instructions": "approve.md",
                        "outputSchemas": {"rejected": {"type": "object"}}},
            "done": {}
        }
    });
    std::fs::write(path.join("settings.json"), settings.to_string()).unwrap();
}

/// Writes the `review` workflow folder at `path`, its step due within `timeout_ms`.
fn review_folder(path: &Path, timeout_ms: &str) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(path.join("workflow.dot"), REVIEW_DOT).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "unsafe",
                       "outcomes": ["approved", "changes"],
                       "async": {"timeoutMs": timeout_ms}}
        },
        "nodes": {"review": {"action": "review"}, "done": {},
                  "rework": {"error": {"code": "review.changes", "message": "no"}}}
    });
    std::fs::write(path.join("settings.json"), settings.to_string()).unwrap();
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

/// `spore start <workflow> --key <key>`: the run ID.
fn start(dir: &Dir, workflow: &str, key: &str) -> String {
    let (code, started) = json(dir, &["start", workflow, "--key", key]);
    assert_eq!(code, Some(0), "{started}");
    started["runId"].as_str().unwrap().to_string()
}

fn ok_loop(name: &str) -> Dir {
    let dir = Dir::new(name);
    loop_folder(&dir.file("loop"), json!(["true"]));
    dir
}

fn review(name: &str, timeout_ms: &str) -> Dir {
    let dir = Dir::new(name);
    review_folder(&dir.file("review"), timeout_ms);
    dir
}

fn unfinished(dir: &Dir) -> Vec<Value> {
    let (code, listed) = json(dir, &["list", "--unfinished"]);
    assert_eq!(code, Some(0), "{listed}");
    listed["runs"].as_array().unwrap().clone()
}

#[test]
fn continue_with_nothing_unfinished_names_the_workflows_and_creates_no_database() {
    let dir = Dir::new("continue-none");
    assert_eq!(dir.run(&["init"]).code(), Some(0));
    loop_folder(&dir.file(".spore/workflows/loop"), json!(["true"]));
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["runs"], json!([]));
    assert_eq!(out["workflows"][0]["name"], "loop", "{out}");
    let folder = out["workflows"][0]["folder"].as_str().unwrap();
    assert!(folder.ends_with(".spore/workflows/loop"), "{folder}");
    assert!(!dir.file("runspore.db").exists());

    let text = dir.run(&args(&["continue"]));
    assert_eq!(text.code(), Some(0));
    assert!(
        text.stdout.contains("nothing to continue"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("loop"), "{}", text.stdout);
}

#[test]
fn continue_with_several_unfinished_runs_lists_them_and_exits_2() {
    let dir = ok_loop("continue-several");
    let first = start(&dir, "loop", "a");
    let second = start(&dir, "loop", "b");
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(code, Some(2), "{out}");
    assert_eq!(out["error"]["code"], "run.several");
    let runs: Vec<&str> = out["error"]["details"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["runId"].as_str().unwrap())
        .collect();
    assert_eq!(runs, [second.as_str(), first.as_str()], "newest first");
    // It changed nothing: neither run took a turn.
    for run in [&first, &second] {
        let (_, status) = json(&dir, &["status", run]);
        assert_eq!(status["status"], "created", "{status}");
    }
    let text = dir.run(&args(&["continue"]));
    assert_eq!(text.code(), Some(2));
    assert!(text.stderr.contains(&first) && text.stderr.contains(&second));
    assert!(
        text.stderr.contains("spore continue <runId>"),
        "{}",
        text.stderr
    );
}

#[test]
fn continue_works_the_one_run_and_prints_the_signal_to_send() {
    let dir = ok_loop("continue-signal");
    let run = start(&dir, "loop", "k");
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(code, Some(3), "{out}");
    assert_eq!(out["status"], "waiting");
    assert_eq!(out["needs"], "signal");
    assert_eq!(
        out["commands"],
        json!([
            format!("spore step show {run}"),
            format!("spore signal {run} approval --outcome approved"),
            format!("spore signal {run} approval --outcome rejected --data <json>"),
        ])
    );
    let text = dir.run(&args(&["continue", &run]));
    assert_eq!(text.code(), Some(3));
    assert!(
        text.stdout.contains("waits at approve for signal approval"),
        "{}",
        text.stdout
    );
    assert!(text
        .stdout
        .contains(&format!("spore signal {run} approval --outcome approved")));

    // A decision already sent makes the run working until it is applied.
    let sent = dir.run(&["signal", &run, "approval", "--outcome", "approved"]);
    assert_eq!(sent.code(), Some(0), "{}", sent.stderr);
    assert_eq!(unfinished(&dir)[0]["needs"], "working");
    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["status"], "completed");
    assert_eq!(out["needs"], Value::Null);
    assert_eq!(out["commands"], json!([]));
    assert!(unfinished(&dir).is_empty());
}

#[test]
fn continue_works_only_its_run_and_spends_no_try_of_another() {
    let dir = Dir::new("continue-only-its-run");
    let ran = dir.file("ran.log");
    let line = format!("echo \"$RUNSPORE_RUN_ID\" >> '{}'", ran.display());
    loop_folder(&dir.file("loop"), json!(["sh", "-c", line]));
    let other = start(&dir, "loop", "other");
    let mine = start(&dir, "loop", "mine");
    let out = dir.run(&args(&["continue", &mine]));
    assert_eq!(out.code(), Some(3), "{}", out.stderr);
    assert!(!out.stderr.contains("other"), "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(&ran).unwrap(), format!("{mine}\n"));
    let (_, status) = json(&dir, &["status", &other, "--no-probe"]);
    assert_eq!(status["status"], "created", "{status}");

    let (code, out) = json(&dir, &["continue", &other]);
    assert_eq!(code, Some(3), "{out}");
    assert_eq!(
        std::fs::read_to_string(&ran).unwrap(),
        format!("{mine}\n{other}\n")
    );
}

#[test]
fn continue_at_a_stuck_step_prints_the_three_resolve_forms() {
    let dir = Dir::new("continue-stuck");
    loop_folder(&dir.file("loop"), json!(["false"]));
    let run = start(&dir, "loop", "k");
    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(4), "{out}");
    assert_eq!(out["needs"], "stuck");
    assert_eq!(
        out["commands"],
        json!([
            format!("spore resolve {run} --retry"),
            format!("spore resolve {run} --complete ok"),
            format!("spore resolve {run} --fail <message>"),
        ])
    );
    let text = dir.run(&args(&["continue", &run]));
    assert_eq!(text.code(), Some(4));
    assert!(text.stdout.contains("command.exit"), "{}", text.stdout);
    assert_eq!(unfinished(&dir)[0]["needs"], "stuck");
}

#[test]
fn continue_at_an_external_step_says_how_to_claim_or_abandon_it() {
    let dir = review("continue-external", "60000");
    let run = start(&dir, "review", "k");
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(code, Some(6), "{out}");
    assert_eq!(out["needs"], "external-ready");
    assert_eq!(
        out["commands"],
        json!([format!("spore step claim {run} --actor reviewer")])
    );
    let listed = unfinished(&dir);
    assert_eq!(listed[0]["needs"], "external-ready");
    assert_eq!(listed[0]["actor"], "reviewer");

    let holder = r#"{"pid":42}"#;
    let claim = dir.run(&[
        "step", "claim", &run, "--actor", "reviewer", "--holder", holder,
    ]);
    assert_eq!(claim.code(), Some(0), "{}", claim.stderr);
    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(6), "{out}");
    assert_eq!(out["needs"], "held");
    assert_eq!(out["step"]["holder"], json!({"pid": 42}));
    assert_eq!(out["step"]["actor"], "reviewer");
    assert!(out["step"]["deadlineMs"].is_string(), "{out}");
    assert_eq!(
        out["commands"],
        json!([format!("spore step abandon {run}")])
    );
    let text = dir.run(&args(&["continue", &run]));
    assert_eq!(text.code(), Some(6));
    assert!(text.stdout.contains("held by reviewer"), "{}", text.stdout);
    assert!(text.stdout.contains(r#"{"pid":42}"#), "{}", text.stdout);
    let listed = unfinished(&dir);
    assert_eq!(
        (&listed[0]["needs"], &listed[0]["actor"]),
        (&json!("held"), &json!("reviewer"))
    );
}

#[test]
fn continue_expires_a_held_step_past_its_deadline_and_goes_on() {
    let dir = review("continue-expired", "300");
    let run = start(&dir, "review", "k");
    assert_eq!(json(&dir, &["continue", &run]).0, Some(6));
    let claim = dir.run(&["step", "claim", &run, "--actor", "reviewer"]);
    assert_eq!(claim.code(), Some(0), "{}", claim.stderr);
    std::thread::sleep(std::time::Duration::from_millis(400));
    // The step is `unsafe`: once expired, its outcome is unknown and a person decides.
    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(4), "{out}");
    assert_eq!(out["needs"], "stuck");
    assert_eq!(out["intervention"]["attemptStatus"], "expired", "{out}");
}

#[test]
fn continue_on_a_quarantined_run_works_nothing_and_names_release() {
    let dir = ok_loop("continue-quarantined");
    let run = start(&dir, "loop", "k");
    let db = rusqlite::Connection::open(dir.file("runspore.db")).unwrap();
    db.execute("DELETE FROM artifacts", []).unwrap();
    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(5), "{out}");
    assert_eq!(out["needs"], "quarantined");
    assert_eq!(out["commands"], json!([format!("spore release {run}")]));
    let revision = out["revision"].clone();
    let (code, again) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(5), "{again}");
    assert_eq!(
        again["revision"], revision,
        "a quarantined run is not worked"
    );
    let text = dir.run(&args(&["continue", &run]));
    assert!(
        text.stdout.contains("artifact.unreadable"),
        "{}",
        text.stdout
    );
    assert_eq!(unfinished(&dir)[0]["needs"], "quarantined");
}

/// `spore worker` runs the step: `continue` names that worker, its pid and for how
/// long, works nothing and exits 0; `continue --wait` says the same on stderr and
/// follows the run past it.
#[test]
fn continue_at_a_step_another_worker_runs_names_the_worker() {
    let dir = Dir::new("continue-elsewhere");
    loop_folder(&dir.file("loop"), common::effect_argv("build", "sleep 3"));
    let run = start(&dir, "loop", "k");
    let mut worker = dir
        .cmd(&args(&["worker"]))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    while dir.effects().is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let (code, out) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["needs"], "working");
    assert_eq!(
        out["commands"],
        json!([format!("spore continue {run} --wait")])
    );
    let running = &out["running"];
    assert_eq!(running["state"], "started", "{out}");
    let (worker_id, pid) = (running["worker"].as_str().unwrap(), &running["pid"]);
    assert!(pid.is_i64(), "{out}");
    let text = dir.run(&args(&["continue", &run]));
    assert_eq!(text.code(), Some(0), "{}", text.stderr);
    let line = format!("build attempt 1 is running on worker {worker_id} (pid {pid}, for ");
    assert!(text.stdout.starts_with(&line), "{}", text.stdout);
    assert!(
        text.stdout.contains(&format!(
            "s); waiting for it to finish\n  spore continue {run} --wait\n"
        )),
        "{}",
        text.stdout
    );
    assert_eq!(dir.effects().len(), 1, "continue ran no try of its own");

    let followed = dir.run(&args(&["continue", &run, "--wait"]));
    assert_eq!(followed.code(), Some(3), "{}", followed.stderr);
    assert!(
        followed.stderr.contains(&format!("  k  {line}")),
        "{}",
        followed.stderr
    );
    assert!(
        followed
            .stdout
            .contains("waits at approve for signal approval"),
        "{}",
        followed.stdout
    );
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[test]
fn list_unfinished_gives_workflow_node_needs_since_and_actor_newest_first() {
    let dir = ok_loop("list-unfinished");
    review_folder(&dir.file("review"), "60000");
    let done = start(&dir, "loop", "done");
    assert_eq!(json(&dir, &["continue", &done]).0, Some(3));
    let sent = dir.run(&["signal", &done, "approval", "--outcome", "approved"]);
    assert_eq!(sent.code(), Some(0));
    assert_eq!(json(&dir, &["continue", &done]).0, Some(0));
    let waiting = start(&dir, "loop", "waiting");
    assert_eq!(json(&dir, &["continue", &waiting]).0, Some(3));
    let external = start(&dir, "review", "external");
    assert_eq!(json(&dir, &["continue", &external]).0, Some(6));
    // `continue` works every due run as `run` does, so this one is started last.
    let fresh = start(&dir, "loop", "fresh");

    let listed = unfinished(&dir);
    let ids: Vec<&str> = listed
        .iter()
        .map(|r| r["runId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [&fresh, &external, &waiting], "{listed:?}");
    let row = |i: usize| {
        let r = &listed[i];
        (
            r["workflow"].clone(),
            r["nodeId"].clone(),
            r["needs"].clone(),
            r["actor"].clone(),
        )
    };
    assert_eq!(
        row(0),
        (json!("loop"), Value::Null, json!("working"), Value::Null)
    );
    assert_eq!(
        row(1),
        (
            json!("review"),
            json!("review"),
            json!("external-ready"),
            json!("reviewer")
        )
    );
    assert_eq!(
        row(2),
        (
            json!("loop"),
            json!("approve"),
            json!("signal"),
            Value::Null
        )
    );
    for r in &listed {
        let since: u64 = r["since"].as_str().unwrap().parse().unwrap();
        assert!(since > 0, "{r}");
        assert_eq!(r["depth"], 0);
    }

    // The tree lists every run, each with the same new fields.
    let (_, all) = json(&dir, &["list"]);
    let finished = all["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["runId"] == done.as_str())
        .unwrap()
        .clone();
    assert_eq!(finished["needs"], Value::Null);
    assert_eq!(finished["workflow"], "loop");
    assert!(finished["since"].is_string());

    let text = dir.run(&args(&["list", "--unfinished"]));
    let lines: Vec<&str> = text.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", text.stdout);
    assert!(lines[0].starts_with(&fresh), "{}", text.stdout);
    assert!(
        lines[1].contains("external-ready") && lines[1].contains("actor reviewer"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].contains("approve") && lines[2].contains("signal"),
        "{}",
        lines[2]
    );
}

#[test]
fn step_show_describes_a_decision_without_changing_the_run() {
    let dir = ok_loop("step-show");
    let run = start(&dir, "loop", "k");
    assert_eq!(json(&dir, &["continue", &run]).0, Some(3));
    let (_, before) = json(&dir, &["status", &run]);
    let (code, shown) = json(&dir, &["step", "show"]);
    assert_eq!(code, Some(0), "{shown}");
    assert_eq!(shown["runId"], run.as_str());
    assert_eq!(shown["nodeId"], "approve");
    assert_eq!(shown["nodeKind"], "await-signal");
    assert_eq!(shown["activationId"], "approve/1");
    assert_eq!(shown["actionId"], Value::Null);
    assert_eq!(shown["signal"], "approval");
    assert_eq!(shown["outcomes"], json!(["approved", "rejected"]));
    assert_eq!(
        shown["outputSchemas"],
        json!({"rejected": {"type": "object"}})
    );
    assert_eq!(shown["from"], json!({"nodeId": "build", "outcome": "ok"}));
    let instructions = shown["instructions"].as_str().unwrap();
    assert_eq!(std::fs::read_to_string(instructions).unwrap(), APPROVE);
    assert!(instructions.starts_with(shown["files"].as_str().unwrap()));
    let (_, after) = json(&dir, &["status", &run]);
    assert_eq!(before["revision"], after["revision"]);

    let text = dir.run(&args(&["step", "show", &run]));
    assert_eq!(text.code(), Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("approve"), "{}", text.stdout);
    assert!(text.stdout.ends_with(APPROVE), "{}", text.stdout);
}

#[test]
fn step_show_refuses_a_run_at_no_step_and_several_runs() {
    let dir = ok_loop("step-show-refused");
    let (code, none) = json(&dir, &["step", "show"]);
    assert_eq!(code, Some(2), "{none}");
    assert_eq!(none["error"]["code"], "run.none-unfinished");
    assert!(!dir.file("runspore.db").exists());

    let run = start(&dir, "loop", "k");
    assert_eq!(json(&dir, &["continue", &run]).0, Some(3));
    let sent = dir.run(&["signal", &run, "approval", "--outcome", "approved"]);
    assert_eq!(sent.code(), Some(0));
    assert_eq!(json(&dir, &["continue", &run]).0, Some(0));
    let (code, ended) = json(&dir, &["step", "show", &run]);
    assert_eq!(code, Some(2), "{ended}");
    assert_eq!(ended["error"]["code"], "step.none");

    start(&dir, "loop", "a");
    start(&dir, "loop", "b");
    let (code, several) = json(&dir, &["step", "show"]);
    assert_eq!(code, Some(2), "{several}");
    assert_eq!(several["error"]["code"], "run.several");
    assert_eq!(
        several["error"]["details"]["runs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn step_show_exits_10_when_the_artifact_is_unreadable_and_leaves_the_run() {
    let dir = ok_loop("step-show-unreadable");
    let run = start(&dir, "loop", "k");
    assert_eq!(json(&dir, &["continue", &run]).0, Some(3));
    let db = rusqlite::Connection::open(dir.file("runspore.db")).unwrap();
    db.execute("DELETE FROM artifacts", []).unwrap();
    let (code, shown) = json(&dir, &["step", "show", &run]);
    assert_eq!(code, Some(10), "{shown}");
    assert_eq!(shown["error"]["code"], "artifact.unreadable");
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["quarantine"], Value::Null, "{status}");
}
