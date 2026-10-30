//! `component` actions through the compiled binary (spec/host.md §3, `component`; H41,
//! H44): a step runner sealed into the workflow starts a detached process that
//! outlives its SIGKILLed worker, and a second `spore` probes and stops it through the
//! runner; a crash at each of the component failpoints leaves every completed step
//! done once.

mod common;

use std::time::{Duration, Instant};

use common::{Dir, BIN};
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "1000",
    "--heartbeat-ms",
    "100",
];

/// `prep`, a command logged to the effects log, then `job`, the test runner given
/// `input`, then `done`.
fn flow(grants: Value, input: Value) -> Value {
    json!({
        "format": "runspore.workflow/0.2", "name": "runner", "start": "prep",
        "actions": {
            "prep": {"kind": "command", "effect": "idempotent",
                     "argv": common::effect_argv("prep", "true")},
            "job": {"kind": "component", "module": "runner.wasm", "effect": "idempotent",
                    "grants": grants, "retry": {"maxAttempts": 3, "backoffMs": "10"}}
        },
        "nodes": {
            "prep": {"kind": "activity", "action": "prep", "outcomes": {"ok": "job"}},
            "job": {"kind": "activity", "action": "job", "input": {"$literal": input},
                    "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

fn folder(dir: &Dir, flow: &Value) {
    let path = dir.folder("flow", flow);
    std::fs::write(path.join("runner.wasm"), runspore_wasmtime::TEST_RUNNER).unwrap();
}

#[test]
fn validate_refuses_a_runner_outside_the_world_and_a_grant_spore_does_not_offer() {
    const CLOCK_RUNNER: &[u8] = include_bytes!(
        "../../runspore-build/catalogue/examples/component.import-unsupported/runner.wasm"
    );
    let dir = Dir::new("component-validate");
    let grants = json!({"programs": ["sleep"], "network": ["api.example.com"]});
    folder(&dir, &flow(grants, json!({})));
    let codes = |dir: &Dir| -> Vec<Value> {
        let (code, out) = spore(dir, None, &["validate", "flow"]);
        assert_eq!(code, Some(2), "{out}");
        let report: Value = serde_json::from_str(out.trim()).unwrap();
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].clone())
            .collect()
    };
    assert_eq!(codes(&dir), [json!("component.grant-unsupported")]);
    folder(&dir, &flow(json!({"programs": ["sleep"]}), json!({})));
    std::fs::write(dir.file("flow/runner.wasm"), CLOCK_RUNNER).unwrap();
    assert_eq!(codes(&dir), [json!("component.import-unsupported")]);
    std::fs::write(dir.file("flow/runner.wasm"), runspore_wasmtime::TEST_RUNNER).unwrap();
    let (code, out) = spore(&dir, None, &["validate", "flow"]);
    assert_eq!(code, Some(0), "{out}");
}

/// `spore --json …` with the fast flags: the exit code and what it printed.
fn spore(dir: &Dir, failpoint: Option<&str>, extra: &[&str]) -> (Option<i32>, String) {
    let mut all = FAST.to_vec();
    all.push("--json");
    all.extend_from_slice(extra);
    let mut cmd = dir.cmd(&all);
    if let Some(failpoint) = failpoint {
        cmd.env("RUNSPORE_FAILPOINTS", failpoint);
    }
    let out = common::run(cmd);
    (out.code(), out.stdout)
}

fn json(dir: &Dir, extra: &[&str]) -> Value {
    let (_, out) = spore(dir, None, extra);
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"))
}

fn alive(pid: i32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    match stat {
        // A zombie has ended; only its parent's reaping is left.
        Ok(stat) => !stat
            .rsplit(')')
            .next()
            .unwrap_or("")
            .trim_start()
            .starts_with('Z'),
        Err(_) => kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH) && !cfg!(target_os = "linux"),
    }
}

#[test]
fn h41_a_runner_s_process_outlives_its_killed_worker_and_a_second_spore_stops_it() {
    let dir = Dir::new("component-h41");
    folder(
        &dir,
        &flow(
            json!({"programs": ["sleep"]}),
            json!({"spawn": ["sleep", "300"]}),
        ),
    );
    let (code, started) = spore(&dir, None, &["start", "flow", "--key", "h41"]);
    assert_eq!(code, Some(0), "{started}");
    let run = serde_json::from_str::<Value>(&started).unwrap()["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let mut args = FAST.to_vec();
    args.push("worker");
    let mut worker = dir.spawn(&args);
    let deadline = Instant::now() + Duration::from_secs(60);
    let holder = loop {
        let status = json(&dir, &["status", &run, "--no-probe"]);
        if status["step"]["state"] == "held" {
            assert_eq!(status["step"]["kind"], "component", "{status}");
            break status["step"]["holder"].clone();
        }
        assert!(Instant::now() < deadline, "never held: {status}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let pid = i32::try_from(holder["process"]["pgid"].as_i64().unwrap()).unwrap();
    assert!(alive(pid), "the runner's process runs: {holder}");
    kill(Pid::from_raw(worker.id() as i32), Signal::SIGKILL).unwrap();
    let _ = worker.wait();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid), "the process went with its worker");

    // A second spore probes through the runner: the holder is alive, the step held.
    let probed = json(&dir, &["status", &run]);
    assert_eq!(probed["step"]["state"], "held", "{probed}");
    let cancelled = json(&dir, &["cancel", &run]);
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the runner's stop left {pid} running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(dir.effects().len(), 1, "prep ran once");
}

/// The test runner's `start` input that starts a job reporting the step done with the
/// attempt's token, after logging itself to the effects log.
fn reporting_job() -> Value {
    let script = "echo \"job $RUNSPORE_ATTEMPT $RUNSPORE_EFFECT_KEY\" >> \"$EFFECTS_LOG\"; \
                  \"$0\" --native-kernel --json step complete \"$RUNSPORE_TOKEN\" \
                  --outcome ok > /dev/null 2>&1";
    json!({"spawn": ["/bin/sh", "-c", script, BIN]})
}

#[test]
fn h44_a_crash_at_each_component_failpoint_runs_no_completed_step_again() {
    for failpoint in ["host.component.after-spawn", "host.component.after-start"] {
        let dir = Dir::new("component-h44");
        folder(
            &dir,
            &flow(json!({"programs": ["/bin/sh"]}), reporting_job()),
        );
        let run = json(&dir, &["start", "flow", "--key", "h44"])["runId"]
            .as_str()
            .unwrap()
            .to_string();
        let (code, out) = spore(&dir, Some(failpoint), &["run", "flow", "--key", "h44"]);
        assert_eq!(code, None, "{failpoint}: the run did not abort: {out}");
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let (code, out) = spore(&dir, None, &["worker", "--run", &run, "--until-parked"]);
            if code == Some(0) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{failpoint}: never completed: exit {code:?}, {out}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let effects = dir.effects();
        let preps = effects.iter().filter(|e| e.node == "prep").count();
        assert_eq!(preps, 1, "{failpoint}: prep ran again: {effects:?}");
        let jobs = effects.iter().filter(|e| e.node == "job").count();
        assert!(
            (1..=3).contains(&jobs),
            "{failpoint}: {jobs} jobs: {effects:?}"
        );
        let status = json(&dir, &["status", &run]);
        assert_eq!(status["status"], "completed", "{failpoint}: {status}");
    }
}

/// H46 through the binary: the work proposes a candidate answer with `spore step
/// propose`, `status` and `step show` show it, `withdraw` discards it, and once the
/// runner's probe says the work has stopped the step completes with it, exactly once
/// across a crash between the probe's answer and the commit.
#[test]
fn h46_a_candidate_answer_commits_once_when_the_work_stops_across_a_crash() {
    let dir = Dir::new("component-h46");
    let answer = dir.file("answer");
    let token = dir.file("token");
    std::fs::write(&answer, "alive").unwrap();
    let input = json!({
        "tokenTo": token.to_string_lossy(),
        "handle": json!({"answer": answer.to_string_lossy()}).to_string(),
    });
    folder(
        &dir,
        &flow(json!({"folders": ["."], "writeFolders": ["."]}), input),
    );
    let (code, out) = spore(&dir, None, &["run", "flow", "--key", "h46"]);
    assert_eq!(code, Some(6), "held: {out}");
    let run = serde_json::from_str::<Value>(out.trim()).unwrap()["runId"]
        .as_str()
        .unwrap()
        .to_string();
    let token = std::fs::read_to_string(&token).expect("the runner wrote its token");

    let (code, out) = spore(
        &dir,
        None,
        &["step", "propose", &token, "--outcome", "maybe"],
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("step.undeclared-outcome"), "{out}");
    let proposed = json(
        &dir,
        &[
            "step",
            "propose",
            &token,
            "--outcome",
            "ok",
            "--output",
            r#"{"n": 1}"#,
        ],
    );
    assert_eq!(proposed["candidate"]["outcome"], "ok", "{proposed}");
    assert_eq!(proposed["attempt"], 1);
    let status = json(&dir, &["status", &run, "--no-probe"]);
    let candidate = &status["step"]["candidate"];
    assert_eq!(candidate["outcome"], "ok", "{status}");
    assert_eq!(candidate["sinceMs"], proposed["candidate"]["sinceMs"]);
    assert!(candidate["forMs"].is_string(), "{candidate}");
    let shown = json(&dir, &["step", "show", &run]);
    assert_eq!(shown["candidate"]["outcome"], "ok", "{shown}");

    let withdrawn = json(&dir, &["step", "withdraw", &token]);
    assert_eq!(withdrawn["withdrawn"], true, "{withdrawn}");
    let status = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(status["step"]["candidate"], Value::Null, "{status}");
    json(
        &dir,
        &[
            "step",
            "propose",
            &token,
            "--outcome",
            "ok",
            "--output",
            r#"{"n": 2}"#,
        ],
    );

    // The probe says the work has stopped; the process dies before the commit.
    std::fs::write(&answer, "stopped").unwrap();
    let (code, out) = spore(
        &dir,
        Some("host.candidate.before-commit"),
        &["status", &run],
    );
    assert_eq!(code, None, "the probe did not reach the failpoint: {out}");
    let status = json(&dir, &["status", &run, "--no-probe"]);
    assert_eq!(
        status["step"]["state"], "held",
        "nothing was recorded: {status}"
    );
    assert_eq!(status["step"]["candidate"]["outcome"], "ok");

    let (code, out) = spore(&dir, None, &["continue", &run]);
    assert_eq!(code, Some(0), "{out}");
    let events = json(&dir, &["events", &run]);
    let results: Vec<&Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "activity.result")
        .collect();
    let outputs: Vec<&Value> = results
        .into_iter()
        .filter(|e| {
            e["payload"]["status"] == "success" && e["payload"]["output"] == json!({"n": 2})
        })
        .collect();
    assert_eq!(outputs.len(), 1, "committed once: {events}");
    assert_eq!(
        json(&dir, &["status", &run])["status"],
        "completed",
        "{events}"
    );
    let (code, out) = spore(&dir, None, &["step", "withdraw", &token]);
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("lease.lost"), "{out}");
}
