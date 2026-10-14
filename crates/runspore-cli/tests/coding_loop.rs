//! The reference loop in `examples/coding-loop/`, run end to end through the compiled
//! binary with a fake `claude` that records its argv and stdin, and a fake `npm`
//! (docs/guide/claude-step.md): the first try starts the session, a stopped try, a red
//! test and a rejection each resume it, and an answer outside the schema is refused.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use common::{Dir, Out};
use runspore_types::digest::ids;
use serde_json::{json, Value};

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "5000",
    "--heartbeat-ms",
    "100",
];

/// Logs its stdin and NUL-separated argv as `calls/<n>.stdin` and `calls/<n>.argv`,
/// hangs once when `hang` exists, then prints an envelope shaped as
/// `claude -p --output-format json` prints it, cost fraction included, with
/// `answer.json` as the structured output.
const CLAUDE: &str = r#"#!/bin/sh
calls="$FAKE_DIR/calls"
mkdir -p "$calls"
n=$(ls "$calls" | grep -c '\.argv$')
cat > "$calls/$n.stdin"
hang=no
if [ -f "$FAKE_DIR/hang" ]; then rm "$FAKE_DIR/hang"; hang=yes; fi
for a in "$@"; do printf '%s\0' "$a"; done > "$calls/next"
mv "$calls/next" "$calls/$n.argv"
if [ "$hang" = yes ]; then sleep 30; fi
printf '{"type":"result","is_error":false,"total_cost_usd":0.0123,"structured_output":%s}' "$(cat "$FAKE_DIR/answer.json")"
"#;

/// Logs to `npm.log`; red once when `red` exists, green otherwise.
const NPM: &str = r#"#!/bin/sh
echo "$*" >> "$FAKE_DIR/npm.log"
if [ -f "$FAKE_DIR/red" ]; then rm "$FAKE_DIR/red"; echo "1 failing: add"; exit 1; fi
echo "all passing"
"#;

/// A directory holding a copy of the example and the two fakes in `bin/`.
fn project(name: &str) -> Dir {
    let dir = Dir::new(name);
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/coding-loop");
    let folder = dir.file("coding-loop");
    std::fs::create_dir_all(&folder).unwrap();
    for entry in std::fs::read_dir(&example).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    let bin = dir.file("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for (name, script) in [("claude", CLAUDE), ("npm", NPM)] {
        let path = bin.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    answer(
        &dir,
        json!({"outcome": "ok", "output": {"summary": "Added add()."}}),
    );
    dir
}

fn answer(dir: &Dir, structured: Value) {
    dir.write("answer.json", &structured);
}

/// `spore` with the fakes first on `PATH`.
fn command(dir: &Dir, extra: &[&str]) -> Command {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    let mut cmd = dir.cmd(&all);
    let path = format!(
        "{}:{}",
        dir.file("bin").display(),
        std::env::var("PATH").unwrap()
    );
    cmd.env("PATH", path).env("FAKE_DIR", &dir.path);
    cmd
}

fn spore(dir: &Dir, extra: &[&str]) -> Out {
    common::run(command(dir, extra))
}

const TASK: &str = r#"{"task": "Add an add function."}"#;

fn run_loop(dir: &Dir) -> Out {
    spore(
        dir,
        &[
            "--json",
            "run",
            "coding-loop",
            "--key",
            "k",
            "--input",
            TASK,
        ],
    )
}

/// One `claude` call: its arguments and the JSON it read on stdin.
struct Call {
    argv: Vec<String>,
    stdin: Value,
}

impl Call {
    /// The argument after `flag`.
    fn after(&self, flag: &str) -> &str {
        let at = self.argv.iter().position(|a| a == flag).unwrap();
        &self.argv[at + 1]
    }

    /// The session flag and the session ID.
    fn session(&self) -> (&str, &str) {
        (&self.argv[1], &self.argv[2])
    }
}

fn calls(dir: &Dir) -> Vec<Call> {
    let calls = dir.file("calls");
    let count = std::fs::read_dir(&calls)
        .map(|entries| {
            entries
                .filter(|e| e.as_ref().unwrap().path().extension() == Some("argv".as_ref()))
                .count()
        })
        .unwrap_or(0);
    (0..count)
        .map(|n| {
            let argv = std::fs::read_to_string(calls.join(format!("{n}.argv"))).unwrap();
            let stdin = std::fs::read(calls.join(format!("{n}.stdin"))).unwrap();
            Call {
                argv: argv
                    .trim_end_matches('\0')
                    .split('\0')
                    .map(String::from)
                    .collect(),
                stdin: serde_json::from_slice(&stdin).unwrap(),
            }
        })
        .collect()
}

fn session(run: &str) -> String {
    ids::uuid(json!([run, "implement"]).to_string().as_bytes())
}

fn signal(dir: &Dir, run: &str, outcome: &str, id: &str, data: Option<&str>) {
    let mut extra = vec!["signal", run, "approval", "--outcome", outcome, "--id", id];
    if let Some(data) = data {
        extra.extend(["--data", data]);
    }
    let out = spore(dir, &extra);
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
}

#[test]
fn the_example_folder_holds_two_files_and_validates() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/coding-loop");
    let mut names: Vec<String> = std::fs::read_dir(&example)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["settings.json", "workflow.dot"]);
    let dir = project("coding-loop-valid");
    let out = spore(&dir, &["validate", "coding-loop"]);
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
}

#[test]
fn one_session_through_a_red_test_and_a_rejection() {
    let dir = project("coding-loop-session");
    std::fs::write(dir.file("red"), "").unwrap();
    let out = run_loop(&dir);
    assert_eq!(out.code(), Some(3), "{}{}", out.stdout, out.stderr);
    let waiting = out.json();
    let run = waiting["runId"].as_str().unwrap().to_string();
    assert_eq!(
        waiting["position"]["activationId"], "approve/1",
        "{waiting}"
    );

    let session = session(&run);
    let seen = calls(&dir);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].session(), ("--session-id", session.as_str()));
    assert_eq!(seen[1].session(), ("--resume", session.as_str()));

    // The node's params set the tools, the schema and the instructions.
    let first = &seen[0];
    assert_eq!(first.argv[0], "-p");
    let tools = "Read,Glob,Grep,Edit,Write,Bash";
    assert_eq!(first.after("--tools"), tools);
    assert_eq!(first.after("--allowedTools"), tools);
    assert_eq!(first.after("--permission-mode"), "default");
    assert_eq!(first.after("--output-format"), "json");
    assert!(first
        .after("--append-system-prompt")
        .starts_with("You are the implement step"));
    let schema: Value = serde_json::from_str(first.after("--json-schema")).unwrap();
    let branch = &schema["anyOf"][0]["properties"];
    assert_eq!(branch["outcome"], json!({"const": "ok"}), "{schema}");
    assert_eq!(branch["output"]["required"], json!(["summary"]), "{schema}");

    // The message is the node's input: the task, and why the run came back.
    assert_eq!(
        first.stdin,
        json!({"task": "Add an add function.", "cameFrom": null, "lastTest": null})
    );
    let red = &seen[1].stdin;
    assert_eq!(
        (&red["cameFrom"]["node"], &red["cameFrom"]["outcome"]),
        (&json!("test"), &json!("red"))
    );
    assert_eq!(red["cameFrom"]["output"]["stdout"], "1 failing: add\n");

    signal(
        &dir,
        &run,
        "rejected",
        "reject-1",
        Some(r#"{"reason": "Name it sum."}"#),
    );
    let out = run_loop(&dir);
    assert_eq!(out.code(), Some(3), "{}{}", out.stdout, out.stderr);
    let seen = calls(&dir);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2].session(), ("--resume", session.as_str()));
    assert_eq!(
        seen[2].stdin["cameFrom"],
        json!({"node": "approve", "outcome": "rejected", "output": {"reason": "Name it sum."}})
    );

    signal(&dir, &run, "approved", "approve-1", None);
    let out = run_loop(&dir);
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(
        out.json()["result"]["output"],
        json!({"task": "Add an add function.", "summary": "Added add()."})
    );
}

#[test]
fn a_stopped_try_resumes_the_session() {
    let dir = project("coding-loop-stopped");
    std::fs::write(dir.file("hang"), "").unwrap();
    let out = spore(
        &dir,
        &[
            "--json",
            "start",
            "coding-loop",
            "--key",
            "k",
            "--input",
            TASK,
        ],
    );
    let run = out.json()["runId"].as_str().unwrap().to_string();
    let mut worker = command(&dir, &["--grace-ms", "300", "worker"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    while !dir.file("calls/0.argv").exists() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pid = nix::unistd::Pid::from_raw(worker.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::SIGINT).unwrap();
    assert_eq!(worker.wait().unwrap().code(), Some(130));

    let out = spore(&dir, &["--json", "worker", "--run", &run, "--until-parked"]);
    assert_eq!(out.json()["status"], "waiting", "{}", out.stderr);
    let session = session(&run);
    let seen = calls(&dir);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].session(), ("--session-id", session.as_str()));
    assert_eq!(seen[1].session(), ("--resume", session.as_str()));
    assert_eq!(seen[1].stdin, seen[0].stdin, "the same message again");
}

#[test]
fn an_answer_outside_the_schema_is_refused() {
    let cases = [
        (
            json!({"outcome": "ok", "output": {"note": "no summary"}}),
            "output.invalid",
        ),
        (
            json!({"outcome": "done", "output": {"summary": "s"}}),
            "command.undeclared-outcome",
        ),
    ];
    for (structured, code) in cases {
        let dir = project("coding-loop-schema");
        answer(&dir, structured.clone());
        let out = run_loop(&dir);
        assert_eq!(out.code(), Some(4), "{structured}: {}", out.stdout);
        let status = out.json();
        assert_eq!(status["status"], "needs-intervention", "{status}");
        assert_eq!(status["position"]["activationId"], "implement/1");
        assert_eq!(status["invocation"]["error"]["code"], code, "{status}");
        assert!(!dir.file("npm.log").exists());
        assert_eq!(calls(&dir).len(), 1, "not retried: {structured}");
    }
}
