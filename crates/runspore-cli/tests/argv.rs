//! A command's `argv` from mappings through the compiled binary (spec/host.md §3):
//! `step.resumed` picks `--session-id` on the first try of the first visit and
//! `--resume` after a failed, timed-out or stopped try and a return to the step; an item that
//! gives no string leaves the step stuck without starting it.

mod common;

use common::Dir;
use runspore_types::digest::ids;
use serde_json::{json, Value};

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

/// The `ask` action: logs `<visit> <attempt> <flag> <session>` to the effects log,
/// then runs `then`. The flag and the session come from mappings, as a `claude -p`
/// step would take them.
fn ask(then: &str) -> Value {
    let script =
        format!("echo \"$RUNSPORE_VISIT $RUNSPORE_ATTEMPT $1 $2\" >> \"$EFFECTS_LOG\"; {then}");
    json!({
        "kind": "command", "effect": "idempotent",
        "retry": {"maxAttempts": 2, "backoffMs": "10"},
        "argv": [
            "/bin/sh", "-c", script, "sh",
            {"$if": [{"$get": ["step", "resumed"]}, "--resume", "--session-id"]},
            {"$uuid": [{"$get": ["run", "id"]}, {"$get": ["step", "node"]}]},
        ],
    })
}

/// A workflow whose `ask` step runs `action`, then `done`.
fn flow(action: Value) -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "ask", "start": "ask",
        "actions": {"ask": action},
        "nodes": {
            "ask": {"kind": "activity", "action": "ask", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"},
        },
    })
}

/// Runs `package` to its end and returns the run's ID and the lines `ask` logged.
fn run(name: &str, package: &Value) -> (String, Vec<String>) {
    let dir = Dir::new(name);
    dir.folder("flow", package);
    let out = dir.run(&args(&["--json", "run", "flow"]));
    let status = out.json();
    assert_eq!(out.code(), Some(0), "{name}: {status}{}", out.stderr);
    assert_eq!(status["status"], json!("completed"), "{name}");
    let log = std::fs::read_to_string(dir.file("effects.log")).unwrap();
    let run = status["runId"].as_str().unwrap().to_string();
    (run, log.lines().map(String::from).collect())
}

fn session(run: &str) -> String {
    ids::uuid(json!([run, "ask"]).to_string().as_bytes())
}

#[test]
fn the_first_try_of_the_first_visit_starts_a_session() {
    let (run, log) = run("argv-first", &flow(ask("true")));
    assert_eq!(log, [format!("1 1 --session-id {}", session(&run))]);
}

#[test]
fn a_try_after_a_failed_try_resumes() {
    let then = "[ \"$RUNSPORE_ATTEMPT\" = 1 ] && exit 1; true";
    let (run, log) = run("argv-failed", &flow(ask(then)));
    let session = session(&run);
    assert_eq!(
        log,
        [
            format!("1 1 --session-id {session}"),
            format!("1 2 --resume {session}")
        ]
    );
}

#[test]
fn a_try_after_a_timed_out_try_resumes() {
    let mut action = ask("[ \"$RUNSPORE_ATTEMPT\" = 1 ] && sleep 5; true");
    action["timeoutMs"] = json!("1000");
    let (run, log) = run("argv-timed-out", &flow(action));
    let session = session(&run);
    assert_eq!(
        log,
        [
            format!("1 1 --session-id {session}"),
            format!("1 2 --resume {session}")
        ]
    );
}

#[test]
fn a_try_after_a_stopped_try_resumes() {
    // The first worker is interrupted during the first try; another does the next.
    let dir = Dir::new("argv-stopped");
    dir.folder(
        "flow",
        &flow(ask("[ \"$RUNSPORE_ATTEMPT\" = 1 ] && sleep 30; true")),
    );
    let run = dir.run(&args(&["--json", "start", "flow"])).json()["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut worker = dir.spawn(&args(&["--grace-ms", "300", "worker"]));
    let log = dir.file("effects.log");
    while !log.exists() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pid = nix::unistd::Pid::from_raw(worker.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::SIGINT).unwrap();
    assert_eq!(worker.wait().unwrap().code(), Some(130));
    let out = dir.run(&args(&[
        "--json",
        "worker",
        "--run",
        &run,
        "--until-parked",
    ]));
    assert_eq!(out.json()["status"], json!("completed"), "{}", out.stderr);
    let session = session(&run);
    let lines: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(
        lines,
        [
            format!("1 1 --session-id {session}"),
            format!("1 2 --resume {session}")
        ]
    );
}

#[test]
fn a_return_to_the_step_resumes() {
    // `check` is red once, which sends the run back to `ask`.
    let mut package = flow(ask("true"));
    package["actions"]["check"] = json!({
        "kind": "command", "effect": "idempotent",
        "argv": ["/bin/sh", "-c", "test -f checked || { touch checked; exit 1; }"],
        "outcomes": ["ok", "red"], "exitOutcomes": {"1": "red"},
    });
    package["nodes"]["ask"]["outcomes"] = json!({"ok": "check"});
    package["nodes"]["check"] = json!({"kind": "activity", "action": "check",
                                       "outcomes": {"ok": "done", "red": "ask"}});
    let (run, log) = run("argv-return", &package);
    let session = session(&run);
    assert_eq!(
        log,
        [
            format!("1 1 --session-id {session}"),
            format!("2 1 --resume {session}")
        ]
    );
}

#[test]
fn an_item_that_gives_no_string_leaves_the_step_stuck_unstarted() {
    let dir = Dir::new("argv-invalid");
    let mut action = ask("true");
    action["argv"][5] = json!({"$get": ["step", "attempt"]});
    dir.folder("flow", &flow(action));
    let out = dir.run(&args(&["--json", "run", "flow"]));
    let status = out.json();
    assert_eq!(out.code(), Some(4), "{status}{}", out.stderr);
    assert_eq!(status["status"], json!("needs-intervention"));
    let invocation = &status["invocation"];
    assert_eq!(invocation["state"], json!("stuck"), "{status}");
    assert_eq!(invocation["attempt"], json!(1), "not retried");
    assert_eq!(invocation["error"]["code"], json!("command.argv-invalid"));
    assert_eq!(
        invocation["error"]["details"],
        json!({"index": 5, "value": 1})
    );
    assert!(!dir.file("effects.log").exists(), "the step ran");
}

#[test]
fn validate_refuses_a_path_root_an_action_does_not_have() {
    let dir = Dir::new("argv-root");
    let mut action = ask("true");
    action["argv"][5] = json!({"$get": ["nodes", "ask", "outcome"]});
    dir.folder("flow", &flow(action));
    let out = dir.run(&["--json", "validate", "flow"]);
    assert_eq!(out.code(), Some(2), "{}{}", out.stdout, out.stderr);
    let report = out.json();
    let diagnostic = &report["diagnostics"][0];
    assert_eq!(diagnostic["code"], json!("mapping.path-root"), "{report}");
    assert_eq!(
        diagnostic["path"],
        json!("/actions/ask/argv/5/$get/0"),
        "{report}"
    );
}
