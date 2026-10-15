//! `spore continue --wait` through the compiled binary (spec/cli.md, What a run needs):
//! it follows a run past a held async step until the run needs a person or has ended. A
//! report from elsewhere is picked up within one poll, a watch's `needs` line makes it
//! return 6 with the watch's message, and an interrupt leaves the attempt held.

mod common;

use std::io::Read;
use std::process::Child;
use std::time::{Duration, Instant};

use common::Dir;
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

const DOT: &str = r#"digraph "review" {
  dialect = "runspore/0.2";
  start = "review";
  done [kind=complete];
  review -> done [on="approved"];
}
"#;

/// `review`: an external step for actor `reviewer`, then `done`.
fn external(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "review": {"kind": "external", "actor": "reviewer", "effect": "unsafe",
                       "outcomes": ["approved"], "async": {"timeoutMs": "60000"}}
        },
        "nodes": {"review": {"action": "review"},
                  "done": {"output": {"$get": ["nodes", "review", "output"]}}}
    });
    std::fs::write(flow.join("settings.json"), settings.to_string()).unwrap();
    dir
}

/// `work`: an async command whose start hands over `{"job": 1}` and whose action
/// watches it with `watch`, then `done`.
fn watched(name: &str, watch: &str) -> Dir {
    let dir = Dir::new(name);
    let watch = json!({"command": {"argv": ["/bin/sh", "-c", watch]}});
    dir.folder(
        "flow",
        &json!({
            "format": "runspore.workflow/0.2", "name": "watched", "start": "work",
            "actions": {"work": {"kind": "command", "effect": "unsafe",
                                 "argv": ["/bin/sh", "-c", "echo '{\"job\": 1}'"],
                                 "async": {"timeoutMs": "60000", "watch": watch}}},
            "nodes": {
                "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
                "done": {"kind": "complete"}
            }
        }),
    );
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

fn start(dir: &Dir) -> String {
    let (code, started) = json(dir, &["start", "flow", "--key", "k"]);
    assert_eq!(code, Some(0), "{started}");
    started["runId"].as_str().unwrap().to_string()
}

/// Waits up to 30 s for `name` to exist and returns what it holds.
fn wait_for(dir: &Dir, name: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.file(name).exists() {
        assert!(Instant::now() < deadline, "{name} never appeared");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    std::fs::read_to_string(dir.file(name)).unwrap()
}

fn alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH)
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
fn a_report_from_elsewhere_is_picked_up_within_one_poll() {
    let dir = external("wait-report");
    let run = start(&dir);
    // A ready external step needs an actor: it returns at once.
    let (code, ready) = json(&dir, &["continue", &run, "--wait"]);
    assert_eq!(code, Some(6), "{ready}");
    assert_eq!(ready["needs"], "external-ready", "{ready}");
    let (_, claim) = json(&dir, &["step", "claim", &run, "--actor", "reviewer"]);
    let token = claim["token"].as_str().unwrap().to_string();

    // Held by the actor, `continue` alone stops there; `--wait` follows the run.
    let (code, held) = json(&dir, &["continue", &run]);
    assert_eq!((code, &held["needs"]), (Some(6), &json!("held")), "{held}");
    let mut follower = dir.spawn(&args(&["--json", "continue", &run, "--wait"]));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        follower.try_wait().unwrap().is_none(),
        "--wait returned at a held step"
    );

    let (code, report) = json(
        &dir,
        &[
            "step",
            "complete",
            &token,
            "--outcome",
            "approved",
            "--output",
            r#"{"ok": 1}"#,
        ],
    );
    assert_eq!(code, Some(0), "{report}");
    let reported = Instant::now();
    let (code, done) = finish(follower);
    assert!(
        reported.elapsed() < Duration::from_secs(5),
        "took {:?}",
        reported.elapsed()
    );
    assert_eq!(code, Some(0), "{done}");
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["needs"], Value::Null, "{done}");
    assert_eq!(done["result"]["output"], json!({"ok": 1}), "{done}");
}

#[test]
fn a_watch_that_says_its_holder_waits_for_a_person_returns_6_with_its_message() {
    let watch = "cat > /dev/null; echo $$ >> watch.pids; \
                 echo '{\"needs\": \"person\", \"message\": \"answer in pane 2\"}'; \
                 exec sleep 600";
    let dir = watched("wait-needs", watch);
    let run = start(&dir);
    let (code, waits) = json(
        &dir,
        &["--stop-grace-ms", "500", "continue", &run, "--wait"],
    );
    assert_eq!(code, Some(6), "{waits}");
    assert_eq!(waits["needs"], "held", "{waits}");
    assert_eq!(waits["step"]["state"], "held", "{waits}");
    assert_eq!(waits["step"]["waiting"], "person", "{waits}");
    assert_eq!(waits["step"]["message"], "answer in pane 2", "{waits}");
    assert_eq!(
        waits["commands"],
        json!([format!("spore step abandon {run}")])
    );
    let first: i32 = wait_for(&dir, "watch.pids").trim().parse().unwrap();
    assert!(!alive(first), "the watch outlived continue --wait");

    // The attempt stays held, and the next driver starts the watch again.
    let (_, status) = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(
        (&status["step"]["state"], &status["step"]["attempt"]),
        (&json!("held"), &json!(1)),
        "{status}"
    );
    assert_eq!(status["step"].get("waiting"), None, "{status}");
    let again = dir.run(&args(&[
        "--stop-grace-ms",
        "500",
        "continue",
        &run,
        "--wait",
    ]));
    assert_eq!(again.code(), Some(6), "{}", again.stdout);
    assert!(
        again
            .stdout
            .contains("; it waits for a person: answer in pane 2"),
        "{}",
        again.stdout
    );
    assert_eq!(
        std::fs::read_to_string(dir.file("watch.pids"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn an_interrupt_stops_the_watch_and_leaves_the_attempt_held() {
    let watch = "cat > /dev/null; echo $$ > watch.pid.tmp; mv watch.pid.tmp watch.pid; \
                 exec sleep 600";
    let dir = watched("wait-interrupt", watch);
    let run = start(&dir);
    let follower = dir.spawn(&args(&[
        "--json",
        "--grace-ms",
        "500",
        "--stop-grace-ms",
        "500",
        "continue",
        &run,
        "--wait",
    ]));
    let pid: i32 = wait_for(&dir, "watch.pid").trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    kill(Pid::from_raw(follower.id() as i32), Signal::SIGINT).unwrap();
    let (code, stopped) = finish(follower);
    assert_eq!(code, Some(130), "{stopped}");
    assert_eq!(stopped["interrupted"], true, "{stopped}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the watch outlived the interrupt"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let (_, status) = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(status["status"], "running", "{status}");
    assert_eq!(
        (&status["step"]["state"], &status["step"]["attempt"]),
        (&json!("held"), &json!(1)),
        "the attempt is held for the next driver: {status}"
    );
    assert_eq!(status["step"]["holder"], json!({"job": 1}), "{status}");
}
