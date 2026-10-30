//! The needs-person mark through the compiled binary (spec/cli.md, Steps outside the
//! worker; spec/host.md H45): the holder of an async step marks it as waiting on a
//! person with `spore step needs-person`, `status`, `list` and `continue` show the mark,
//! a repeated ID or a second clear changes nothing, the mark changes nothing the run
//! reads, and it ends with its attempt. `continue --wait` returns at a mark set while
//! it follows, never at one it found when it began.

mod common;

use std::io::Read;
use std::process::Child;
use std::time::{Duration, Instant};

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

const PROMPT: &str = "Claude needs your permission to use Bash";

/// `review`: an external step for actor `reviewer`, tried at most twice, then `done`.
fn external(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "idempotent",
                       "outcomes": ["approved"], "async": {"timeoutMs": "60000"},
                       "retry": {"maxAttempts": 2, "backoffMs": "10"}}
        },
        "nodes": {"review": {"action": "review"}, "done": {}}
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

/// Starts `flow` and claims its review step; the run and the token.
fn claimed(dir: &Dir) -> (String, String) {
    let (code, started) = json(dir, &["start", "flow", "--key", "k"]);
    assert_eq!(code, Some(0), "{started}");
    let run = started["runId"].as_str().unwrap().to_string();
    let (code, ready) = json(dir, &["continue", &run]);
    assert_eq!(code, Some(6), "{ready}");
    let (code, claim) = json(dir, &["step", "claim", &run, "--actor", "reviewer"]);
    assert_eq!(code, Some(0), "{claim}");
    (run, claim["token"].as_str().unwrap().to_string())
}

fn mark(dir: &Dir, token: &str, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["step", "needs-person", token];
    all.extend_from_slice(extra);
    json(dir, &all)
}

/// The run's events, as `events --json` lists them.
fn events(dir: &Dir, run: &str) -> Value {
    json(dir, &["events", run]).1["events"].clone()
}

/// Waits for `child` to exit; its exit code, and stdout parsed as JSON.
fn finish(mut child: Child) -> (Option<i32>, Value) {
    let status = child.wait().unwrap();
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let json = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("not one JSON object ({e}): {stdout}"));
    (status.code(), json)
}

#[test]
fn a_mark_shows_in_status_list_and_continue_and_moves_nothing() {
    let dir = external("np-shown");
    let (run, token) = claimed(&dir);
    let (_, before) = json(&dir, &["status", &run]);
    assert_eq!(before["step"]["needsPerson"], Value::Null, "{before}");
    assert_eq!(before["step"].get("waiting"), None, "{before}");
    let events_before = events(&dir, &run);

    let (code, set) = mark(&dir, &token, &["--reason", PROMPT, "--id", "n1"]);
    assert_eq!(code, Some(0), "{set}");
    assert_eq!(
        (&set["disposition"], &set["nodeId"], &set["requestId"]),
        (&json!("applied"), &json!("review"), &json!("n1"))
    );
    assert_eq!((&set["runId"], &set["attempt"]), (&json!(run), &json!(1)));
    assert_eq!(set["needsPerson"]["reason"], PROMPT, "{set}");
    let since = set["needsPerson"]["sinceMs"].clone();
    assert!(since.is_string(), "{set}");

    let (_, status) = json(&dir, &["status", &run]);
    let step = &status["step"];
    assert_eq!(
        (
            &step["state"],
            &step["needsPerson"]["reason"],
            &step["needsPerson"]["sinceMs"]
        ),
        (&json!("held"), &json!(PROMPT), &since),
        "{status}"
    );
    assert!(step["needsPerson"]["forMs"].is_string(), "{status}");
    assert_eq!(
        (&step["waiting"], &step["message"]),
        (&json!("person"), &json!(PROMPT))
    );
    assert_eq!(status["revision"], before["revision"]);
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("waiting    for a person since ") && text.contains(&format!(": {PROMPT}")),
        "{text}"
    );

    let (_, list) = json(&dir, &["list", "--unfinished"]);
    let row = &list["runs"][0];
    assert_eq!(
        (&row["needs"], &row["needsPerson"]["reason"]),
        (&json!("held"), &json!(PROMPT)),
        "{list}"
    );
    let (_, tree) = json(&dir, &["list"]);
    assert_eq!(tree["runs"][0]["needsPerson"]["sinceMs"], since, "{tree}");
    let text = dir.run(&args(&["list", "--unfinished"])).stdout;
    assert!(
        text.contains("needs a person for ") && text.contains(PROMPT),
        "{text}"
    );

    let out = dir.run(&args(&["continue", &run]));
    assert_eq!(out.code(), Some(6), "{}", out.stderr);
    assert!(
        out.stdout.contains("; it waits for a person since ") && out.stdout.contains(PROMPT),
        "{}",
        out.stdout
    );
    let (code, continued) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(6), "{continued}");
    assert_eq!(continued["step"]["needsPerson"]["reason"], PROMPT);

    // A worker that comes and goes leaves it, and nothing the run reads changed.
    let (code, worked) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(6), "{worked}");
    assert_eq!(worked["step"]["needsPerson"]["sinceMs"], since, "{worked}");
    assert_eq!(events(&dir, &run), events_before);
    let (_, after) = json(&dir, &["status", &run]);
    assert_eq!(after["revision"], before["revision"]);
    assert_eq!(
        after["step"]["leaseUntilMs"],
        before["step"]["leaseUntilMs"]
    );
}

#[test]
fn a_repeat_is_a_no_op_and_a_clear_is_harmless() {
    let dir = external("np-repeat");
    let (run, token) = claimed(&dir);
    let (_, first) = mark(&dir, &token, &["--reason", PROMPT, "--id", "n1"]);
    let since = first["needsPerson"]["sinceMs"].clone();
    let (code, again) = mark(&dir, &token, &["--reason", PROMPT, "--id", "n1"]);
    assert_eq!(code, Some(0), "{again}");
    assert_eq!(again["disposition"], "duplicate");
    assert_eq!(again["needsPerson"]["sinceMs"], since);
    // The same reason without an ID changes nothing either.
    let (_, same) = mark(&dir, &token, &["--reason", PROMPT]);
    assert_eq!(same["disposition"], "duplicate", "{same}");
    std::thread::sleep(Duration::from_millis(20));
    // Another reason replaces the reason and keeps when it was set.
    let (_, other) = mark(&dir, &token, &["--reason", "Claude asks a question"]);
    assert_eq!(other["disposition"], "applied", "{other}");
    assert_eq!(
        (
            &other["needsPerson"]["reason"],
            &other["needsPerson"]["sinceMs"]
        ),
        (&json!("Claude asks a question"), &since)
    );
    let (code, reused) = mark(&dir, &token, &["--clear", "--id", "n1"]);
    assert_eq!(
        (code, &reused["error"]["code"]),
        (Some(2), &json!("step.id-reused")),
        "{reused}"
    );
    assert_eq!(
        reused["error"]["details"],
        json!({"runId": run, "id": "n1"})
    );

    let (code, cleared) = mark(&dir, &token, &["--clear", "--id", "c1"]);
    assert_eq!(code, Some(0), "{cleared}");
    assert_eq!(
        (&cleared["disposition"], &cleared["needsPerson"]),
        (&json!("applied"), &Value::Null)
    );
    let (code, twice) = mark(&dir, &token, &["--clear"]);
    assert_eq!(code, Some(0), "{twice}");
    assert_eq!(twice["disposition"], "duplicate");
    let (code, replay) = mark(&dir, &token, &["--clear", "--id", "c1"]);
    assert_eq!(
        (code, &replay["disposition"]),
        (Some(0), &json!("duplicate"))
    );
    let text = dir.run(&args(&["step", "needs-person", &token, "--clear"]));
    assert_eq!(text.stdout.trim(), "step review attempt 1 needs no person");

    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["step"]["needsPerson"], Value::Null, "{status}");
    assert_eq!(status["step"].get("waiting"), None, "{status}");
}

#[test]
fn bad_requests_are_refused_and_record_nothing() {
    let dir = external("np-refused");
    let (run, token) = claimed(&dir);
    let long = "x".repeat(1025);
    for reason in ["", long.as_str()] {
        let (code, refused) = mark(&dir, &token, &["--reason", reason]);
        assert_eq!(
            (code, &refused["error"]["code"]),
            (Some(2), &json!("step.reason-invalid")),
            "{refused}"
        );
    }
    let (code, bad) = mark(&dir, "not-a-token", &["--reason", PROMPT]);
    assert_eq!(
        (code, &bad["error"]["code"]),
        (Some(2), &json!("step.token-invalid"))
    );
    let neither = dir.run(&args(&["step", "needs-person", &token]));
    assert_eq!(neither.code(), Some(2), "{}", neither.stderr);
    let both = dir.run(&args(&[
        "step",
        "needs-person",
        &token,
        "--reason",
        PROMPT,
        "--clear",
    ]));
    assert_eq!(both.code(), Some(2), "{}", both.stderr);
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["step"]["needsPerson"], Value::Null, "{status}");
}

#[test]
fn the_mark_ends_with_its_attempt_and_the_next_attempt_starts_unmarked() {
    let dir = external("np-ends");
    let (run, token) = claimed(&dir);
    mark(&dir, &token, &["--reason", PROMPT]);
    let fail = [
        "step",
        "fail",
        &token,
        "--error",
        "pane closed",
        "--retryable",
    ];
    let (code, failed) = json(&dir, &fail);
    assert_eq!(code, Some(0), "{failed}");
    // The ended attempt carries no mark: a clear changes nothing, a set is too late.
    let (code, cleared) = mark(&dir, &token, &["--clear"]);
    assert_eq!(
        (code, &cleared["disposition"]),
        (Some(0), &json!("duplicate"))
    );
    let (code, late) = mark(&dir, &token, &["--reason", PROMPT]);
    assert_eq!(
        (code, &late["error"]["code"]),
        (Some(2), &json!("lease.lost")),
        "{late}"
    );

    let (code, retried) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(6), "{retried}");
    let (_, claim) = json(&dir, &["step", "claim", &run, "--actor", "reviewer"]);
    assert_eq!(claim["attempt"], 2, "{claim}");
    let second = claim["token"].as_str().unwrap().to_string();
    let (_, status) = json(&dir, &["status", &run]);
    assert_eq!(status["step"]["needsPerson"], Value::Null, "{status}");

    // An abandon ends it too; the run then needs a person to resolve the step.
    mark(&dir, &second, &["--reason", PROMPT]);
    let (code, abandoned) = json(&dir, &["step", "abandon", &run]);
    assert_eq!(code, Some(0), "{abandoned}");
    let (code, late) = mark(&dir, &second, &["--reason", PROMPT]);
    assert_eq!(
        (code, &late["error"]["code"]),
        (Some(2), &json!("lease.lost"))
    );
    let (_, list) = json(&dir, &["list"]);
    assert_eq!(list["runs"][0]["needsPerson"], Value::Null, "{list}");
}

#[test]
fn a_completed_step_takes_its_mark_with_it() {
    let dir = external("np-completed");
    let (run, token) = claimed(&dir);
    mark(&dir, &token, &["--reason", PROMPT]);
    let (code, done) = json(&dir, &["step", "complete", &token, "--outcome", "approved"]);
    assert_eq!(code, Some(0), "{done}");
    // A hook that clears after the step has ended exits 0.
    let (code, cleared) = mark(&dir, &token, &["--clear"]);
    assert_eq!(
        (code, &cleared["disposition"]),
        (Some(0), &json!("duplicate"))
    );
    let (code, finished) = json(&dir, &["continue", &run]);
    assert_eq!((code, &finished["status"]), (Some(0), &json!("completed")));
    assert_eq!(finished["step"], Value::Null, "{finished}");
}

#[test]
fn the_work_an_async_command_started_marks_its_step_with_its_token() {
    let dir = Dir::new("np-async");
    let start = "printf %s \"$RUNSPORE_TOKEN\" > token; echo '{\"job\": 1}'";
    dir.folder(
        "flow",
        &json!({
            "format": "runspore.workflow/0.2", "name": "detached", "start": "work",
            "actions": {"work": {"kind": "command", "effect": "idempotent",
                                 "argv": ["/bin/sh", "-c", start],
                                 "async": {"timeoutMs": "60000"}}},
            "nodes": {
                "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"}
            }
        }),
    );
    let (code, held) = json(&dir, &["run", "flow", "--key", "k"]);
    assert_eq!(
        (code, &held["step"]["state"]),
        (Some(6), &json!("held")),
        "{held}"
    );
    let run = held["runId"].as_str().unwrap().to_string();
    let token = std::fs::read_to_string(dir.file("token")).unwrap();
    let (code, set) = mark(&dir, &token, &["--reason", PROMPT]);
    assert_eq!(code, Some(0), "{set}");
    let (_, status) = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(
        (
            &status["step"]["kind"],
            &status["step"]["needsPerson"]["reason"]
        ),
        (&json!("command"), &json!(PROMPT)),
        "{status}"
    );
    let (code, done) = json(&dir, &["step", "complete", &token, "--outcome", "ok"]);
    assert_eq!(code, Some(0), "{done}");
    let (code, finished) = json(&dir, &["continue", &run]);
    assert_eq!((code, &finished["status"]), (Some(0), &json!("completed")));
}

#[test]
fn continue_wait_returns_at_a_mark_set_while_it_follows_only() {
    let dir = external("np-wait");
    let (run, token) = claimed(&dir);
    let follower = dir.spawn(&args(&["--json", "continue", &run, "--wait"]));
    std::thread::sleep(Duration::from_millis(500));
    mark(&dir, &token, &["--reason", PROMPT]);
    let marked = Instant::now();
    let (code, told) = finish(follower);
    assert!(
        marked.elapsed() < Duration::from_secs(5),
        "took {:?}",
        marked.elapsed()
    );
    assert_eq!(code, Some(0), "{told}");
    assert_eq!(told["needs"], "held", "{told}");
    assert_eq!(
        (&told["step"]["waiting"], &told["step"]["message"]),
        (&json!("person"), &json!(PROMPT)),
        "{told}"
    );
    // Following again while the same mark stands waits on, through another reason.
    let mut again = dir.spawn(&args(&["--json", "continue", &run, "--wait"]));
    std::thread::sleep(Duration::from_millis(500));
    mark(&dir, &token, &["--reason", "Claude asks a question"]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        again.try_wait().unwrap().is_none(),
        "--wait returned at the mark it began with"
    );
    // Cleared and set again, the step waits for the person anew.
    mark(&dir, &token, &["--clear"]);
    std::thread::sleep(Duration::from_millis(100));
    mark(&dir, &token, &["--reason", PROMPT]);
    let (code, anew) = finish(again);
    assert_eq!(code, Some(0), "{anew}");
    assert_eq!(anew["step"]["needsPerson"]["reason"], PROMPT, "{anew}");
}
