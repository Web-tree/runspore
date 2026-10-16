//! H37: the reference loop in `examples/coding-loop/`, installed as a project's
//! workflow and driven through the compiled binary by `continue --wait`, killed at
//! every row of spec/cli.md §Coming back after a kill. A fake `claude` stands in for
//! the implement step and a fake `npm` for the tests; either hangs on the calls listed
//! in its `.hang` file, so a kill lands inside a try.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{Dir, Out};
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use runspore_types::digest::ids;
use serde_json::{json, Value};

const FAST: [&str; 9] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "1000",
    "--heartbeat-ms",
    "100",
    "--stop-grace-ms",
    "500",
];

/// Call `n` (from 0) logs its stdin and NUL-separated argv as `calls/<n>.stdin` and
/// `calls/<n>.argv`, appends `call <n>` to `work.txt` in its working folder (its edit
/// on disk), hangs when `claude.hang` lists `n`, then prints an envelope shaped as
/// `claude -p --output-format json` prints it, with `answer.json` as the structured
/// output.
const CLAUDE: &str = r#"#!/bin/sh
calls="$FAKE_DIR/calls"
mkdir -p "$calls"
n=$(ls "$calls" | grep -c '\.argv$')
cat > "$calls/$n.stdin"
echo "call $n" >> work.txt
for a in "$@"; do printf '%s\0' "$a"; done > "$calls/next"
mv "$calls/next" "$calls/$n.argv"
if grep -qx "$n" "$FAKE_DIR/claude.hang" 2>/dev/null; then sleep 30; fi
printf '{"type":"result","is_error":false,"structured_output":%s}' "$(cat "$FAKE_DIR/answer.json")"
"#;

/// Run `n` (from 0) appends a line to `npm.log`, hangs when `npm.hang` lists `n`, and
/// is red while `red` exists.
const NPM: &str = r#"#!/bin/sh
log="$FAKE_DIR/npm.log"
n=$(cat "$log" 2>/dev/null | wc -l | tr -d ' ')
echo "$* $n" >> "$log"
if grep -qx "$n" "$FAKE_DIR/npm.hang" 2>/dev/null; then sleep 30; fi
if [ -f "$FAKE_DIR/red" ]; then echo "1 failing: add (run $n)"; echo "AssertionError" >&2; exit 1; fi
echo "all passing"
"#;

const TASK: &str = r#"{"task": "Add an add function."}"#;

/// A project folder with the example installed as `.spore/workflows/coding-loop`, and
/// the two fakes in `bin/`.
fn project(name: &str) -> Dir {
    let dir = Dir::new(name);
    let init = dir.run(&["init"]);
    assert_eq!(init.code(), Some(0), "{}", init.stderr);
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/coding-loop");
    let folder = dir.file(".spore/workflows/coding-loop");
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
    dir.write(
        "answer.json",
        &json!({"outcome": "ok", "output": {"summary": "Added add()."}}),
    );
    dir
}

/// Makes the fake `name` hang on these calls.
fn hang(dir: &Dir, name: &str, calls: &[usize]) {
    let lines: String = calls.iter().map(|n| format!("{n}\n")).collect();
    std::fs::write(dir.file(&format!("{name}.hang")), lines).unwrap();
}

fn red(dir: &Dir, on: bool) {
    if on {
        std::fs::write(dir.file("red"), "").unwrap();
    } else {
        std::fs::remove_file(dir.file("red")).unwrap();
    }
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

/// `spore --json …`: the exit code and the JSON it printed.
fn json(dir: &Dir, extra: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["--json"];
    all.extend_from_slice(extra);
    let out = spore(dir, &all);
    (out.code(), out.json())
}

/// `spore --json …` aborted by `failpoint`.
fn crash(dir: &Dir, failpoint: &str, extra: &[&str]) {
    let mut all = vec!["--json"];
    all.extend_from_slice(extra);
    let mut cmd = command(dir, &all);
    cmd.env("RUNSPORE_FAILPOINTS", failpoint);
    let out = common::run(cmd);
    assert_eq!(
        out.code(),
        None,
        "{failpoint}: {}{}",
        out.stdout,
        out.stderr
    );
    assert!(out.stderr.contains(failpoint), "{}", out.stderr);
}

fn start(dir: &Dir) -> String {
    let (code, started) = json(
        dir,
        &["start", "coding-loop", "--key", "k", "--input", TASK],
    );
    assert_eq!(code, Some(0), "{started}");
    started["runId"].as_str().unwrap().to_string()
}

/// `continue <run> --wait` to its end: what the person must do next, with the run's
/// exit code (`--exit-status`).
fn follow(dir: &Dir, run: &str) -> (Option<i32>, Value) {
    json(dir, &["continue", run, "--wait", "--exit-status"])
}

/// `continue <run> --wait` in the background, in a process group of its own, as a
/// session's follower is.
fn follower(dir: &Dir, run: &str) -> Child {
    command(dir, &["--json", "continue", run, "--wait"])
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// The session dies: its follower's process group is killed outright.
fn kill_session(mut follower: Child) {
    killpg(Pid::from_raw(follower.id() as i32), Signal::SIGKILL).unwrap();
    assert_eq!(follower.wait().unwrap().code(), None);
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until `claude` call `n` has started.
fn claude_call(dir: &Dir, n: usize) {
    let argv = dir.file(&format!("calls/{n}.argv"));
    wait_until(&format!("claude call {n}"), || argv.exists());
}

/// Waits until test run `n` has started.
fn test_run(dir: &Dir, n: usize) {
    wait_until(&format!("test run {n}"), || tests_run(dir) > n);
}

fn tests_run(dir: &Dir) -> usize {
    std::fs::read_to_string(dir.file("npm.log"))
        .unwrap_or_default()
        .lines()
        .count()
}

/// The edits the `claude` calls left in the project.
fn work(dir: &Dir) -> String {
    std::fs::read_to_string(dir.file("work.txt")).unwrap_or_default()
}

/// One `claude` call: its arguments and the JSON it read on stdin.
struct Call {
    argv: Vec<String>,
    stdin: Value,
}

impl Call {
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

/// Asserts the first call started the conversation and every later one resumed it.
fn one_conversation(dir: &Dir, run: &str) {
    let session = ids::uuid(json!([run, "implement"]).to_string().as_bytes());
    for (n, call) in calls(dir).iter().enumerate() {
        let flag = if n == 0 { "--session-id" } else { "--resume" };
        assert_eq!(call.session(), (flag, session.as_str()), "call {n}");
    }
}

fn status(dir: &Dir, run: &str) -> Value {
    let (code, status) = json(dir, &["status", run]);
    assert_eq!(code, Some(0), "{status}");
    status
}

fn unfinished(dir: &Dir) -> Vec<Value> {
    let (code, listed) = json(dir, &["list", "--unfinished"]);
    assert_eq!(code, Some(0), "{listed}");
    listed["runs"].as_array().unwrap().clone()
}

/// What `list --unfinished` says the one unfinished run needs.
fn needs(dir: &Dir) -> Value {
    let runs = unfinished(dir);
    assert_eq!(runs.len(), 1, "{runs:?}");
    runs[0]["needs"].clone()
}

/// The status of each `activity.result` event of the run, in order.
fn results(dir: &Dir, run: &str) -> Vec<String> {
    let (code, events) = json(dir, &["events", run]);
    assert_eq!(code, Some(0), "{events}");
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "activity.result")
        .map(|e| e["payload"]["status"].as_str().unwrap().to_string())
        .collect()
}

fn signal(dir: &Dir, run: &str, name: &str, outcome: &str, id: &str, data: Option<&str>) -> Value {
    let mut extra = vec!["signal", run, name, "--outcome", outcome, "--id", id];
    if let Some(data) = data {
        extra.extend(["--data", data]);
    }
    let (code, out) = json(dir, &extra);
    assert_eq!(code, Some(0), "{out}");
    out
}

/// Asserts the run waits at `activation`, entered from `from` on `outcome`.
fn waits_at(out: &(Option<i32>, Value), activation: &str, from: &str, outcome: &str) {
    let (code, status) = out;
    assert_eq!(*code, Some(3), "{status}");
    assert_eq!(status["status"], "waiting", "{status}");
    assert_eq!(status["needs"], "signal", "{status}");
    assert_eq!(status["position"]["activationId"], activation, "{status}");
    assert_eq!(
        status["position"]["from"],
        json!({"nodeId": from, "outcome": outcome}),
        "{status}"
    );
}

#[test]
fn killed_before_start_returned() {
    let dir = project("back-before-start");
    // Before the run was committed: there is none.
    crash(
        &dir,
        "store.create-run.before-commit",
        &["start", "coding-loop", "--key", "k", "--input", TASK],
    );
    assert!(unfinished(&dir).is_empty());
    let (code, out) = json(&dir, &["continue"]);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out["runs"], json!([]));
    assert_eq!(out["workflows"][0]["name"], "coding-loop", "{out}");

    // After it was committed, before `start` printed it: a run with no turn taken.
    crash(
        &dir,
        "store.create-run.after-commit",
        &["start", "coding-loop", "--key", "k", "--input", TASK],
    );
    assert_eq!(needs(&dir), "working");
    let run = start(&dir);
    assert_eq!(
        unfinished(&dir)[0]["runId"],
        run.as_str(),
        "the same key, the same run"
    );
    assert!(calls(&dir).is_empty());

    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    assert_eq!(calls(&dir).len(), 1);
    assert_eq!(tests_run(&dir), 1);
    one_conversation(&dir, &run);
}

#[test]
fn a_dead_session_records_nothing_and_its_lease_runs_out() {
    let dir = project("back-implement-kill");
    hang(&dir, "claude", &[0]);
    let run = start(&dir);
    let session = follower(&dir, &run);
    claude_call(&dir, 0);
    kill_session(session);

    let status = status(&dir, &run);
    assert_eq!(status["status"], "running", "{status}");
    assert_eq!(status["position"]["activationId"], "implement/1");
    let running = &status["running"];
    assert_eq!(
        running["state"], "cut-off",
        "the dead try still holds its lease, and nothing works it: {status}"
    );
    assert!(
        matches!(
            running["cutOff"]["reason"].as_str(),
            Some("process-gone" | "worker-gone")
        ),
        "{status}"
    );
    assert_eq!(
        running["cutOff"]["commands"],
        json!([format!("spore continue {run}")])
    );
    assert_eq!(needs(&dir), "driver");
    let text = spore(&dir, &["status", &run]).stdout;
    assert!(
        text.contains("running    implement attempt 1 of 3 was cut off: ")
            && text.contains(&format!("; spore continue {run} runs it again\n")),
        "{text}"
    );
    let listed = spore(&dir, &["list", "--unfinished"]).stdout;
    assert!(listed.contains(" driver "), "{listed}");
    assert!(results(&dir, &run).is_empty(), "nothing recorded");

    let back = Instant::now();
    let out = follow(&dir, &run);
    waits_at(&out, "approve/1", "test", "ok");
    assert!(
        back.elapsed() >= Duration::from_millis(300),
        "the lease ran out first"
    );
    let seen = calls(&dir);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].stdin, seen[0].stdin, "the same message again");
    one_conversation(&dir, &run);
    assert_eq!(
        work(&dir),
        "call 0\ncall 1\n",
        "the cut-off try's edit is still there"
    );
    assert_eq!(tests_run(&dir), 1);
    // implement's dead try, its next try, the tests.
    assert_eq!(results(&dir, &run), ["expired", "success", "success"]);
}

#[test]
fn a_closed_terminal_records_the_try_stopped() {
    let dir = project("back-implement-hup");
    hang(&dir, "claude", &[0]);
    let run = start(&dir);
    let mut session = follower(&dir, &run);
    claude_call(&dir, 0);
    kill(Pid::from_raw(session.id() as i32), Signal::SIGHUP).unwrap();
    assert_eq!(session.wait().unwrap().code(), Some(130));

    let status = status(&dir, &run);
    assert_eq!(status["status"], "running", "{status}");
    assert_eq!(
        status["running"]["state"], "queued",
        "the next try is due at once"
    );
    assert_eq!(needs(&dir), "working");
    assert_eq!(results(&dir, &run), ["unknown"]);

    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    let seen = calls(&dir);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].stdin, seen[0].stdin);
    one_conversation(&dir, &run);
}

/// Kills the session during three tries in a row, each started once `started(n)`
/// returns; then `continue` finds the step stuck, and `resolve --retry` runs a fourth.
fn three_cut_off_tries(dir: &Dir, run: &str, node: &str, started: impl Fn(usize)) {
    for n in 0..3 {
        let session = follower(dir, run);
        started(n);
        kill_session(session);
    }
    assert_eq!(needs(dir), "driver", "the last try was cut off");
    let (code, stuck) = json(dir, &["continue", run]);
    assert_eq!(code, Some(4), "{stuck}");
    assert_eq!(stuck["status"], "needs-intervention");
    assert_eq!(stuck["needs"], "stuck");
    assert_eq!(stuck["position"]["nodeId"], node, "{stuck}");
    let invocation = &stuck["invocation"];
    assert_eq!(invocation["state"], "stuck", "{stuck}");
    assert_eq!(invocation["attempt"], 3);
    assert_eq!(invocation["error"]["code"], "activity.attempts-exhausted");
    assert_eq!(stuck["commands"][0], format!("spore resolve {run} --retry"));
    assert_eq!(
        stuck["commands"].as_array().unwrap().last().unwrap(),
        &json!(format!("spore resolve {run} --fail <message>"))
    );
    assert_eq!(needs(dir), "stuck");

    let (code, resolved) = json(dir, &["resolve", run, "--retry", "--id", "retry-1"]);
    assert_eq!(code, Some(0), "{resolved}");
    waits_at(&follow(dir, run), "approve/1", "test", "ok");
}

#[test]
fn three_cut_off_tries_of_implement_make_it_stuck() {
    let dir = project("back-implement-stuck");
    hang(&dir, "claude", &[0, 1, 2]);
    let run = start(&dir);
    three_cut_off_tries(&dir, &run, "implement", |n| claude_call(&dir, n));
    assert_eq!(calls(&dir).len(), 4, "the retry is a fourth try");
    one_conversation(&dir, &run);
    assert_eq!(tests_run(&dir), 1);
}

#[test]
fn three_cut_off_test_runs_make_test_stuck() {
    let dir = project("back-test-stuck");
    hang(&dir, "npm", &[0, 1, 2]);
    let run = start(&dir);
    three_cut_off_tries(&dir, &run, "test", |n| test_run(&dir, n));
    assert_eq!(tests_run(&dir), 4);
    assert_eq!(calls(&dir).len(), 1, "implement's result stands");
}

#[test]
fn killed_between_steps_runs_nothing_again() {
    let dir = project("back-between");
    let run = start(&dir);
    crash(
        &dir,
        "host.attempt.after-finish",
        &["continue", &run, "--wait"],
    );
    assert_eq!(results(&dir, &run), ["success"], "implement's answer");
    assert_eq!(status(&dir, &run)["status"], "running");
    assert_eq!(needs(&dir), "working");
    assert_eq!(tests_run(&dir), 0);

    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    assert_eq!(calls(&dir).len(), 1);
    assert_eq!(tests_run(&dir), 1);
}

#[test]
fn killed_after_the_command_returned_runs_it_again() {
    let dir = project("back-after-effect");
    let run = start(&dir);
    crash(
        &dir,
        "host.attempt.after-effect",
        &["continue", &run, "--wait"],
    );
    assert!(
        results(&dir, &run).is_empty(),
        "the answer was not recorded"
    );
    assert_eq!(needs(&dir), "working");

    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    let seen = calls(&dir);
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].stdin, seen[0].stdin);
    one_conversation(&dir, &run);
    assert_eq!(work(&dir), "call 0\ncall 1\n", "it finds its finished edit");
}

#[test]
fn a_cut_off_test_run_is_not_a_red() {
    let dir = project("back-test-kill");
    red(&dir, true);
    hang(&dir, "npm", &[2]);
    let run = start(&dir);
    let session = follower(&dir, &run);
    test_run(&dir, 2);
    kill_session(session);
    let status = status(&dir, &run);
    assert_eq!(status["position"]["activationId"], "test/3", "{status}");
    assert_eq!(status["streaks"]["test"]["count"], 2, "{status}");
    assert_eq!(needs(&dir), "driver");

    // The third visit's second try is the third red, the fourth visit the exhausted one.
    let out = follow(&dir, &run);
    waits_at(&out, "tests_stuck/1", "test", "exhausted");
    assert_eq!(tests_run(&dir), 5);
    assert_eq!(calls(&dir).len(), 4);
}

#[test]
fn a_wait_and_a_stored_decision_run_nothing_again() {
    let dir = project("back-waiting");
    let run = start(&dir);
    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");

    // Coming back to a wait: the same instructions, nothing run.
    let again = json(&dir, &["continue", &run]);
    waits_at(&again, "approve/1", "test", "ok");
    assert_eq!(
        again.1["commands"],
        json!([
            format!("spore step show {run}"),
            format!("spore signal {run} approval --outcome approved"),
            format!("spore signal {run} approval --outcome rejected --data <json>"),
        ])
    );
    let (code, step) = json(&dir, &["step", "show"]);
    assert_eq!(code, Some(0), "{step}");
    assert_eq!(step["activationId"], "approve/1");
    assert_eq!(step["outcomes"], json!(["approved", "rejected"]));
    assert_eq!(step["from"], json!({"nodeId": "test", "outcome": "ok"}));

    // A decision sent and not yet applied: the run is working, the same decision again
    // a no-op.
    let sent = signal(&dir, &run, "approval", "approved", "approve/1", None);
    assert_eq!(sent["disposition"], "applied", "{sent}");
    assert_eq!(needs(&dir), "working");
    let again = signal(&dir, &run, "approval", "approved", "approve/1", None);
    assert_eq!(again["disposition"], "duplicate", "{again}");

    let (code, done) = follow(&dir, &run);
    assert_eq!(code, Some(0), "{done}");
    assert_eq!(done["status"], "completed");
    assert_eq!(
        done["result"]["output"],
        json!({"task": "Add an add function.", "summary": "Added add()."})
    );
    assert_eq!(calls(&dir).len(), 1);
    assert_eq!(tests_run(&dir), 1);

    // Ended: nothing to continue.
    assert!(unfinished(&dir).is_empty());
    let (code, none) = json(&dir, &["continue"]);
    assert_eq!((code, &none["runs"]), (Some(0), &json!([])), "{none}");
    let (code, ended) = json(&dir, &["continue", &run]);
    assert_eq!((code, &ended["status"]), (Some(0), &json!("completed")));
    assert_eq!(calls(&dir).len(), 1);
}

#[test]
fn three_reds_ask_a_person_and_the_loop_carries_why_it_came_back() {
    let dir = project("back-loop");
    red(&dir, true);
    let run = start(&dir);

    // Three reds go back to implement; the fourth takes the exhausted arrow.
    waits_at(&follow(&dir, &run), "tests_stuck/1", "test", "exhausted");
    assert_eq!((calls(&dir).len(), tests_run(&dir)), (4, 4));
    let seen = calls(&dir);
    assert_eq!(seen[0].stdin["cameFrom"], Value::Null);
    for call in &seen[1..] {
        let from = &call.stdin["cameFrom"];
        assert_eq!(
            (&from["node"], &from["outcome"]),
            (&json!("test"), &json!("red"))
        );
        assert!(from["output"]["stdout"]
            .as_str()
            .unwrap()
            .starts_with("1 failing"));
        assert_eq!(from["output"]["stderr"], "AssertionError\n");
    }

    // Keep going: three reds are allowed again.
    signal(
        &dir,
        &run,
        "tests-stuck",
        "keep-going",
        "tests_stuck/1",
        Some(r#"{"hint": "Check the sign."}"#),
    );
    waits_at(&follow(&dir, &run), "tests_stuck/2", "test", "exhausted");
    assert_eq!((calls(&dir).len(), tests_run(&dir)), (8, 8));
    let back = &calls(&dir)[4].stdin;
    assert_eq!(
        back["cameFrom"],
        json!({"node": "tests_stuck", "outcome": "keep-going",
               "output": {"hint": "Check the sign."}})
    );
    assert_eq!(back["lastTest"]["visit"], 4, "{back}");
    assert_eq!(back["lastTest"]["outcome"], "red", "{back}");
    assert_eq!(
        back["lastTest"]["output"]["stdout"],
        "1 failing: add (run 3)\n"
    );

    // Green after keep-going; then a rejection carries its reason.
    red(&dir, false);
    signal(
        &dir,
        &run,
        "tests-stuck",
        "keep-going",
        "tests_stuck/2",
        Some("{}"),
    );
    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    signal(
        &dir,
        &run,
        "approval",
        "rejected",
        "approve/1",
        Some(r#"{"reason": "Name it sum."}"#),
    );
    waits_at(&follow(&dir, &run), "approve/2", "test", "ok");
    assert_eq!(
        calls(&dir)[9].stdin["cameFrom"],
        json!({"node": "approve", "outcome": "rejected", "output": {"reason": "Name it sum."}})
    );
    signal(&dir, &run, "approval", "approved", "approve/2", None);
    let (code, done) = follow(&dir, &run);
    assert_eq!(
        (code, &done["status"]),
        (Some(0), &json!("completed")),
        "{done}"
    );
    assert_eq!((calls(&dir).len(), tests_run(&dir)), (10, 10));
    one_conversation(&dir, &run);
}

#[test]
fn stop_at_tests_stuck_fails_the_run() {
    let dir = project("back-stop");
    red(&dir, true);
    let run = start(&dir);
    waits_at(&follow(&dir, &run), "tests_stuck/1", "test", "exhausted");
    signal(&dir, &run, "tests-stuck", "stop", "tests_stuck/1", None);
    let (code, failed) = follow(&dir, &run);
    assert_eq!(code, Some(1), "{failed}");
    assert_eq!(
        failed["result"]["error"]["code"], "tests.given-up",
        "{failed}"
    );
    assert!(unfinished(&dir).is_empty());
    let (code, _) = json(&dir, &["continue", &run]);
    assert_eq!(code, Some(1));
    assert_eq!(calls(&dir).len(), 4);
}

/// Cancels the run, then checks that it is left out, `continue` exits 7 and nothing
/// runs again, also once a dead try's lease has run out.
fn cancelled(dir: &Dir, run: &str) {
    let ran = (calls(dir).len(), tests_run(dir));
    let (code, cancel) = json(dir, &["cancel", run, "--reason", "not needed"]);
    assert_eq!(code, Some(0), "{cancel}");
    assert_eq!(cancel["status"], "cancelled", "{cancel}");
    assert!(unfinished(dir).is_empty());
    std::thread::sleep(Duration::from_millis(1200));
    let (code, out) = follow(dir, run);
    assert_eq!(code, Some(7), "{out}");
    assert_eq!(out["status"], "cancelled");
    assert_eq!(out["commands"], json!([]));
    assert_eq!((calls(dir).len(), tests_run(dir)), ran, "nothing ran again");
}

#[test]
fn cancel_at_each_waiting_point() {
    let dir = project("back-cancel-approve");
    let run = start(&dir);
    waits_at(&follow(&dir, &run), "approve/1", "test", "ok");
    cancelled(&dir, &run);

    let dir = project("back-cancel-stuck-tests");
    red(&dir, true);
    let run = start(&dir);
    waits_at(&follow(&dir, &run), "tests_stuck/1", "test", "exhausted");
    cancelled(&dir, &run);

    // A stuck step: an answer outside implement's schema.
    let dir = project("back-cancel-stuck");
    dir.write("answer.json", &json!({"outcome": "ok", "output": {}}));
    let run = start(&dir);
    let (code, stuck) = follow(&dir, &run);
    assert_eq!(
        (code, &stuck["needs"]),
        (Some(4), &json!("stuck")),
        "{stuck}"
    );
    cancelled(&dir, &run);

    // A dead session's try, its lease not yet run out.
    let dir = project("back-cancel-killed");
    hang(&dir, "claude", &[0]);
    let run = start(&dir);
    let session = follower(&dir, &run);
    claude_call(&dir, 0);
    kill_session(session);
    cancelled(&dir, &run);
}
