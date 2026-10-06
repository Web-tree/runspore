//! H6: the real binary, crashed at every failpoint and at SIGKILL points through a
//! run, recovers with every completed step run once and the unsafe step run once.
//! The kernel is called natively here: a debug build compiles the component for
//! seconds per process, and this suite starts thousands of processes.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::ExitStatusExt;
use std::time::{Duration, Instant};

use common::{effect_argv, Dir, Out};
use serde_json::{json, Value};

const KEY: &str = "crash";
const TUNING: [&str; 7] = [
    "--native-kernel",
    "--lease-ms",
    "400",
    "--heartbeat-ms",
    "100",
    "--poll-ms",
    "20",
];
const FAILPOINTS: [&str; 19] = [
    "host.turn.after-reduce",
    "host.turn.after-commit",
    "host.attempt.after-claim",
    "host.attempt.after-effect",
    "host.attempt.after-finish",
    "store.create-run.before-commit",
    "store.create-run.after-commit",
    "store.append-event.before-commit",
    "store.append-event.after-commit",
    "store.commit-turn.before-commit",
    "store.commit-turn.after-commit",
    "store.claim-attempt.before-commit",
    "store.claim-attempt.after-commit",
    "store.heartbeat.before-commit",
    "store.heartbeat.after-commit",
    "store.finish-attempt.before-commit",
    "store.finish-attempt.after-commit",
    "store.expire-attempt.before-commit",
    "store.expire-attempt.after-commit",
];

/// Four effectful steps: two idempotent (retryable), an approval, an unsafe deploy.
fn workflow() -> Value {
    json!({
        "format": "runspore.workflow/0.1",
        "name": "crash",
        "start": "prep",
        "actions": {
            "prep": {"kind": "command", "effect": "idempotent", "output": "json",
                     "retry": {"maxAttempts": 3, "backoffMs": "10"},
                     "argv": effect_argv("prep", "sleep 0.1; printf '{\"n\": 1}'")},
            "build": {"kind": "command", "effect": "idempotent", "output": "json",
                      "retry": {"maxAttempts": 3, "backoffMs": "10"},
                      "argv": effect_argv("build", "sleep 0.1; cat")},
            "deploy": {"kind": "command", "effect": "unsafe", "output": "none",
                       "argv": effect_argv("deploy", "sleep 0.1")}
        },
        "nodes": {
            "prep": {"kind": "activity", "action": "prep", "outcomes": {"ok": "build"}},
            "build": {"kind": "activity", "action": "build",
                      "input": {"$get": ["nodes", "prep", "output"]},
                      "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "deploy"}},
            "deploy": {"kind": "activity", "action": "deploy", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete",
                     "output": {"built": {"$get": ["nodes", "build", "output"]},
                                "approval": {"$get": ["nodes", "approve", "outcome"]}}}
        }
    })
}

fn invoke(dir: &Dir, failpoint: Option<&str>, args: &[&str]) -> Out {
    let mut all: Vec<&str> = TUNING.to_vec();
    all.push("--json");
    all.extend_from_slice(args);
    let mut cmd = dir.cmd(&all);
    if let Some(fp) = failpoint {
        cmd.env("RUNSPORE_FAILPOINTS", fp);
    }
    common::run(cmd)
}

fn run_id(dir: &Dir) -> String {
    invoke(dir, None, &["start", "workflow.json", "--key", KEY]).json()["runId"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Drives the run with `run`, delivering the signal on exit 3 and resolving on exit 4
/// from the effects log. Only invocation number `stage` (runs, signals and resolutions
/// in order) gets `failpoint`. Returns `Ok(true)` if a process aborted, `Ok(false)` once
/// the run completed.
fn drive(dir: &Dir, failpoint: Option<&str>, stage: usize) -> Result<bool, String> {
    let mut calls = 0..;
    let mut armed = || failpoint.filter(|_| calls.next() == Some(stage));
    for _ in 0..20 {
        let out = invoke(dir, armed(), &["run", "workflow.json", "--key", KEY]);
        if aborted(&out) {
            return Ok(true);
        }
        let follow = match out.code() {
            Some(0) => return Ok(false),
            Some(3) => {
                let run = out.json()["runId"].as_str().unwrap().to_string();
                invoke(
                    dir,
                    armed(),
                    &[
                        "signal",
                        &run,
                        "approval",
                        "--outcome",
                        "approved",
                        "--id",
                        "approve-1",
                    ],
                )
            }
            Some(4) => {
                let status = out.json();
                let run = status["runId"].as_str().unwrap();
                let invocation = status["invocation"]["invocationId"].as_str().unwrap();
                let attempt = status["invocation"]["attempt"].to_string();
                let deployed = dir.effects().iter().any(|e| e.node == "deploy");
                let decision = if deployed {
                    ["--complete", "ok"]
                } else {
                    ["--retry", ""]
                };
                let id = format!("resolve-{attempt}");
                let mut args = vec!["resolve", run, invocation, decision[0]];
                if deployed {
                    args.push(decision[1]);
                }
                args.extend(["--id", &id]);
                invoke(dir, armed(), &args)
            }
            code => return Err(format!("run exited {code:?}: {}", out.stdout.trim())),
        };
        if aborted(&follow) {
            return Ok(true);
        }
        if follow.code() != Some(0) {
            return Err(format!(
                "follow-up exited {:?}: {}",
                follow.code(),
                follow.stdout
            ));
        }
    }
    Err("run did not complete within 20 invocations".into())
}

fn aborted(out: &Out) -> bool {
    out.status.signal() == Some(6) || out.status.signal() == Some(9)
}

struct Reference {
    result: Value,
    nodes: Value,
    kinds: Vec<String>,
}

fn reference() -> Reference {
    let dir = Dir::new("reference");
    dir.write("workflow.json", &workflow());
    assert_eq!(drive(&dir, None, 0), Ok(false));
    let status = invoke(&dir, None, &["status", &run_id(&dir)]).json();
    let effects = dir.effects();
    let nodes: Vec<&str> = effects.iter().map(|e| e.node.as_str()).collect();
    assert_eq!(nodes, ["prep", "build", "deploy"]);
    Reference {
        result: status["result"].clone(),
        nodes: status["nodes"].clone(),
        kinds: events(&dir)
            .iter()
            .map(|e| e["kind"].as_str().unwrap().into())
            .collect(),
    }
}

fn events(dir: &Dir) -> Vec<Value> {
    let out = invoke(dir, None, &["events", &run_id(dir)]);
    out.json()["events"].as_array().unwrap().clone()
}

/// The five H6 assertions for a recovered run; `accepted` holds the nodes whose result
/// was in the snapshot when the process died.
fn check(dir: &Dir, reference: &Reference, accepted: &BTreeSet<String>) -> Result<(), String> {
    let status = invoke(dir, None, &["status", &run_id(dir)]).json();
    if status["status"] != "completed"
        || status["result"] != reference.result
        || status["nodes"] != reference.nodes
    {
        return Err(format!("final state differs: {status}"));
    }
    let mut by_node: BTreeMap<String, Vec<common::Effect>> = BTreeMap::new();
    for effect in dir.effects() {
        by_node.entry(effect.node.clone()).or_default().push(effect);
    }
    for node in accepted.iter().filter(|n| *n != "approve") {
        if by_node.get(node).map_or(0, Vec::len) != 1 {
            return Err(format!(
                "{node} had an accepted result and ran again: {by_node:?}"
            ));
        }
    }
    for node in ["prep", "build"] {
        let runs = by_node.get(node).cloned().unwrap_or_default();
        let attempts: BTreeSet<u32> = runs.iter().map(|e| e.attempt).collect();
        let keys: BTreeSet<&str> = runs.iter().map(|e| e.key.as_str()).collect();
        if runs.is_empty() || runs.len() > 3 || attempts.len() != runs.len() || keys.len() != 1 {
            return Err(format!(
                "{node} executions break the retry contract: {runs:?}"
            ));
        }
    }
    if by_node.get("deploy").map_or(0, Vec::len) != 1 {
        return Err(format!(
            "unsafe deploy did not run exactly once: {by_node:?}"
        ));
    }
    let events = events(dir);
    let mut revisions = Vec::new();
    for (i, event) in events.iter().enumerate() {
        if event["sequence"] != (i + 1).to_string() {
            return Err(format!("sequence gap at {i}: {event}"));
        }
        match event["consumedRevision"].as_str() {
            Some(r) => revisions.push(r.parse::<u64>().unwrap()),
            None => return Err(format!("event never consumed: {event}")),
        }
    }
    revisions.sort_unstable();
    revisions.dedup();
    let last = status["revision"].as_str().unwrap().parse::<u64>().unwrap();
    if revisions != (1..=last).collect::<Vec<_>>() {
        return Err(format!("revisions not gapless 1..={last}: {revisions:?}"));
    }
    Ok(())
}

fn accepted_nodes(dir: &Dir) -> BTreeSet<String> {
    let out = invoke(dir, None, &["status", &run_id(dir)]);
    match out.code() {
        Some(0) => out.json()["nodes"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
        _ => BTreeSet::new(),
    }
}

/// Crash with `failpoint` at hit `n`. `None` when the failpoint did not fire. Lease
/// expiry happens only after a crash, so the expire failpoints first leave a claimed
/// attempt behind.
fn failpoint_scenario(
    reference: &Reference,
    name: &str,
    stage: usize,
    n: u32,
) -> Option<Result<(), String>> {
    let dir = Dir::new("fp");
    dir.write("workflow.json", &workflow());
    let spec = format!("{name}:{n}");
    if name.starts_with("store.expire-attempt") {
        let held = invoke(
            &dir,
            Some("host.attempt.after-claim"),
            &["run", "workflow.json", "--key", KEY],
        );
        if !aborted(&held) {
            return Some(Err("no lease left behind to expire".into()));
        }
    }
    match drive(&dir, Some(&spec), stage) {
        Ok(false) => return None,
        Ok(true) => {}
        Err(e) => return Some(Err(format!("before the crash: {e}"))),
    }
    let accepted = accepted_nodes(&dir);
    Some(drive(&dir, None, 0).and_then(|crashed| {
        if crashed {
            return Err("aborted without a failpoint".into());
        }
        check(&dir, reference, &accepted)
    }))
}

#[test]
fn every_failpoint_at_every_hit_recovers() {
    let reference = reference();
    assert_eq!(
        reference.kinds.first().map(String::as_str),
        Some("run.started")
    );
    let rows: Vec<(String, u32, u32, Vec<String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = FAILPOINTS
            .iter()
            .map(|name| {
                let reference = &reference;
                s.spawn(move || {
                    let (mut hits, mut passed, mut failures) = (0, 0, Vec::new());
                    let stages = if name.starts_with("store.expire-attempt") {
                        1
                    } else {
                        3
                    };
                    for stage in 0..stages {
                        for n in 1..=40 {
                            let Some(result) = failpoint_scenario(reference, name, stage, n) else {
                                break;
                            };
                            hits += 1;
                            match result {
                                Ok(()) => passed += 1,
                                Err(e) => failures.push(format!("{name}:{n} stage {stage}: {e}")),
                            }
                        }
                    }
                    (name.to_string(), hits, passed, failures)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    println!("\n{:<36} {:>5} {:>7}", "failpoint", "hits", "passed");
    for (name, hits, passed, _) in &rows {
        println!("{name:<36} {hits:>5} {passed:>7}");
    }
    let failures: Vec<&String> = rows.iter().flat_map(|r| &r.3).collect();
    for name in FAILPOINTS {
        assert!(
            rows.iter().any(|r| r.0 == name && r.1 > 0),
            "{name} never fired"
        );
    }
    assert!(failures.is_empty(), "failed scenarios:\n{failures:#?}");
}

#[test]
fn sigkill_at_every_point_of_a_run_recovers() {
    let reference = reference();
    let timed = Dir::new("timing");
    timed.write("workflow.json", &workflow());
    let run = run_id(&timed);
    invoke(
        &timed,
        None,
        &["signal", &run, "approval", "--outcome", "approved"],
    );
    let begin = Instant::now();
    assert_eq!(
        invoke(&timed, None, &["run", "workflow.json", "--key", KEY]).code(),
        Some(0)
    );
    let duration = begin.elapsed();
    let points: u32 = 40;
    let results: Vec<Result<(), String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..points)
            .map(|i| {
                let reference = &reference;
                let delay = duration * (i + 1) / points;
                s.spawn(move || kill_scenario(reference, delay))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let failures: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    println!(
        "\nSIGKILL sweep over {duration:?}: {points} kill points, {} passed",
        results.len() - failures.len()
    );
    assert!(failures.is_empty(), "failed kill points:\n{failures:#?}");
}

fn kill_scenario(reference: &Reference, delay: Duration) -> Result<(), String> {
    let dir = Dir::new("kill");
    dir.write("workflow.json", &workflow());
    let run = run_id(&dir);
    invoke(
        &dir,
        None,
        &[
            "signal",
            &run,
            "approval",
            "--outcome",
            "approved",
            "--id",
            "approve-1",
        ],
    );
    let mut args: Vec<&str> = TUNING.to_vec();
    args.extend(["run", "workflow.json", "--key", KEY]);
    let mut child = dir.spawn(&args);
    std::thread::sleep(delay);
    let _ = child.kill();
    let _ = child.wait();
    let accepted = accepted_nodes(&dir);
    match drive(&dir, None, 0) {
        Ok(false) => {
            check(&dir, reference, &accepted).map_err(|e| format!("kill after {delay:?}: {e}"))
        }
        other => Err(format!("kill after {delay:?}: recovery {other:?}")),
    }
}
