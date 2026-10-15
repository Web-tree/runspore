//! Async commands through the compiled binary (spec/cli.md, Steps outside the worker;
//! spec/host.md §3, H33, H34): a start command detaches a job that reports with
//! `spore step complete`, `run` stops at the held step with exit 6, `status` probes it
//! unless `--no-probe`, and the worker's death leaves a detached job running and a
//! running start command stopped before the next try.

mod common;

use std::time::{Duration, Instant};

use common::{Dir, BIN};
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};

const FAST: [&str; 3] = ["--native-kernel", "--poll-ms", "20"];

/// A one-step flow: `work`, an async command running `start` with `async`, then `done`.
fn flow(start: &str, effect: &str, async_spec: Value) -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "detached", "start": "work",
        "actions": {"work": {"kind": "command", "effect": effect,
                             "retry": {"maxAttempts": 3, "backoffMs": "10"},
                             "argv": ["/bin/sh", "-c", start], "async": async_spec}},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}}
        }
    })
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

fn read(dir: &Dir, name: &str) -> String {
    std::fs::read_to_string(dir.file(name)).unwrap_or_default()
}

/// Waits up to 30 s for `name` to exist and returns what it holds.
fn wait_for(dir: &Dir, name: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.file(name).exists() {
        assert!(Instant::now() < deadline, "{name} never appeared");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    read(dir, name)
}

fn alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH)
}

fn group_alive(group: i32) -> bool {
    killpg(Pid::from_raw(group), None) != Err(Errno::ESRCH)
}

/// The start of H33: detaches a job in a session and process group of its own
/// (`setsid`), with its streams away from the start's, and prints the job's handle.
/// The job waits for `go`, then reports through `spore step complete` with the
/// attempt's token. Perl's `setsid` stands in for the `setsid` tool, which macOS
/// lacks; `set -m` is no substitute, as dash turns job control off without a tty.
/// The start waits for the job's `job.pid`, written after `setsid`: ending sooner
/// would let the guard kill the start's group with the job still in it.
fn detaching_start() -> String {
    format!(
        "printf %s \"$RUNSPORE_RUN_ID\" > run_id; \
         perl -MPOSIX -e 'POSIX::setsid() or die; exec @ARGV' /bin/sh -c \
           'echo $$ > job.pid.tmp; mv job.pid.tmp job.pid; \
            while [ ! -f go ]; do sleep 0.05; done; \
            \"$0\" --native-kernel --json step complete \"$RUNSPORE_TOKEN\" --outcome ok \
               --output \"{{\\\"from\\\": \\\"job\\\"}}\" > reported.tmp; \
            mv reported.tmp reported.json' '{BIN}' \
           < /dev/null > /dev/null 2>&1 & \
         while [ ! -f job.pid ]; do sleep 0.01; done; \
         echo \"{{\\\"pid\\\": $(cat job.pid)}}\""
    )
}

#[test]
fn h33_a_detached_job_outlives_its_killed_worker_and_its_report_completes_the_run() {
    let dir = Dir::new("h33");
    // A watch that never ends keeps the worker at the held step until it is killed.
    let watch = json!({"command": {"argv": ["/bin/sh", "-c", "exec sleep 600"]}});
    let spec = json!({"timeoutMs": "60000", "watch": watch});
    dir.folder("flow", &flow(&detaching_start(), "idempotent", spec));
    let mut worker = dir.spawn(&args(&["run", "flow", "--key", "h33"]));
    let job: i32 = wait_for(&dir, "job.pid").trim().parse().unwrap();
    let run = read(&dir, "run_id");
    // The start has handed over once the step shows held.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (_, status) = json(&dir, &["status", &run, "--no-probe"]);
        if status["step"]["state"] == "held" {
            assert_eq!(status["step"]["kind"], "command");
            assert_eq!(status["step"]["holder"], json!({"pid": job}));
            break;
        }
        assert!(Instant::now() < deadline, "never held: {status}");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    kill(Pid::from_raw(worker.id() as i32), Signal::SIGKILL).unwrap();
    let _ = worker.wait();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(job), "the job went with its worker");

    std::fs::write(dir.file("go"), "").unwrap();
    let reported: Value = serde_json::from_str(&wait_for(&dir, "reported.json")).unwrap();
    assert_eq!(reported["disposition"], "applied", "{reported}");
    let (code, done) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(0), "{done}");
    assert_eq!(done["result"]["output"], json!({"from": "job"}), "{done}");
}

#[test]
fn h34_a_start_command_left_by_a_dead_worker_and_guard_is_stopped_before_the_next_try() {
    let dir = Dir::new("h34");
    let start = "echo \"$RUNSPORE_ATTEMPT $$ $PPID\" >> tries.log; \
                 if [ \"$RUNSPORE_ATTEMPT\" = 1 ]; then sleep 30 & wait; fi; \
                 echo '{\"job\": 2}'";
    dir.folder("flow", &flow(start, "idempotent", json!({})));
    let flags = ["--lease-ms", "2000", "--stop-grace-ms", "500"];
    let mut argv = args(&flags);
    argv.extend_from_slice(&["run", "flow", "--key", "h34"]);
    let mut worker = dir.spawn(&argv);
    let tries = |dir: &Dir| -> Vec<Vec<i32>> {
        read(dir, "tries.log")
            .lines()
            .map(|l| l.split(' ').filter_map(|w| w.parse().ok()).collect())
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while tries(&dir).is_empty() {
        assert!(Instant::now() < deadline, "try 1 never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (group, guard) = (tries(&dir)[0][1], tries(&dir)[0][2]);
    std::thread::sleep(Duration::from_millis(200));
    kill(Pid::from_raw(guard), Signal::SIGKILL).unwrap();
    kill(Pid::from_raw(worker.id() as i32), Signal::SIGKILL).unwrap();
    let _ = worker.wait();
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        group_alive(group),
        "the start went with its worker and guard"
    );

    let mut next = dir.spawn(&argv);
    let mut gone = None;
    let deadline = Instant::now() + Duration::from_secs(15);
    while tries(&dir).len() < 2 {
        assert!(Instant::now() < deadline, "try 2 never started");
        if gone.is_none() && !group_alive(group) {
            gone = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = next.wait().unwrap();
    assert!(gone.is_some(), "try 2 started while try 1's start ran");
    assert_eq!(status.code(), Some(6), "the next try hands over and parks");
}

#[test]
fn status_probes_a_held_step_unless_told_not_to() {
    let dir = Dir::new("async-status");
    let probe = "cat > /dev/null; cat answer";
    let spec = json!({"probe": {"command": {"argv": ["/bin/sh", "-c", probe]}}});
    dir.folder("flow", &flow("echo '{\"job\": 1}'", "unsafe", spec));
    std::fs::write(dir.file("answer"), r#"{"holder": "alive"}"#).unwrap();
    let (code, parked) = json(&dir, &["run", "flow", "--key", "s"]);
    assert_eq!(code, Some(6), "{parked}");
    assert_eq!(
        (&parked["step"]["kind"], &parked["step"]["state"]),
        (&json!("command"), &json!("held"))
    );
    assert_eq!(parked["step"]["holder"], json!({"job": 1}));
    assert_eq!(
        parked["slots"]["held"],
        json!([]),
        "a held step takes no slot"
    );
    let run = parked["runId"].as_str().unwrap().to_string();

    std::fs::write(dir.file("answer"), r#"{"holder": "dead"}"#).unwrap();
    let (_, unprobed) = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(unprobed["step"]["state"], "held", "{unprobed}");
    let (_, probed) = json(&dir, &["status", &run]);
    assert_eq!(probed["step"], Value::Null, "{probed}");
    let (code, after) = json(&dir, &["worker", "--run", &run, "--until-parked"]);
    assert_eq!(code, Some(4), "an unsafe step whose holder died: {after}");
    assert_eq!(after["intervention"]["attemptStatus"], "expired");
}
