//! What `status` says a run waits for and how to move it on (spec/cli.md, `waiting`,
//! `intervention` and `running`): at a signal, the signal, its outcomes, their schemas
//! and the `spore signal` commands; at a stuck or unknown step, why, its last error,
//! its attempts and the `spore resolve` commands; at a running step, its attempt and
//! how long it has run. Each with the time the run came to stand so.

mod common;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{effect_argv, Dir};
use serde_json::{json, Value};

/// The heartbeat renews the lease well before it runs out on a busy machine.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

const DOT: &str = r#"digraph "status" {
  dialect = "runspore/0.2";
  start = "work";
  approve [kind="await-signal"];
  done [kind=complete];
  work -> approve;
  approve -> done [on="approved"];
  approve -> work [on="rejected"];
}
"#;

/// `work` runs `then` with `effect`, two tries allowed; `approve` waits for
/// `approval`, whose `rejected` needs a reason.
fn folder(name: &str, effect: &str, then: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let argv = effect_argv("work", then);
    let settings = format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "actions": {{
    "work": {{"kind": "command", "effect": "{effect}", "argv": {argv},
             "retry": {{"maxAttempts": 2, "backoffMs": "10"}}}}
  }},
  "nodes": {{
    "work": {{"action": "work"}},
    "approve": {{"signal": "approval", "outputSchemas": {{"rejected": {{
      "type": "object", "properties": {{"reason": {{"type": "string"}}}},
      "required": ["reason"]}}}}}},
    "done": {{"output": {{"$get": ["nodes", "approve", "outcome"]}}}}
  }}
}}
"#
    );
    std::fs::write(flow.join("settings.json"), settings).unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn now_ms() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(since.as_millis()).unwrap()
}

/// `sinceMs` and `forMs` of a detail, checked to be decimal strings that end now.
fn since(detail: &Value, before: u64) -> u64 {
    let ms = |key: &str| {
        detail[key]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(|| panic!("{key} is not a decimal string: {detail}"))
    };
    let (since, lasted) = (ms("sinceMs"), ms("forMs"));
    assert!(since >= before && since <= now_ms(), "{detail}");
    assert!(since + lasted <= now_ms() + 1, "{detail}");
    since
}

#[test]
fn a_stuck_step_then_a_signal_say_what_they_wait_for_and_how_to_answer() {
    let dir = folder(
        "status-stuck",
        "idempotent",
        "test -f ok || { echo out-line; echo err-line >&2; exit 3; }",
    );
    let before = now_ms();
    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(parked.code(), Some(4), "{}", parked.stdout);
    let status = parked.json();
    let run = status["runId"].as_str().unwrap().to_string();
    let intervention = &status["intervention"];
    assert_eq!(
        (&status["waiting"], &status["running"]),
        (&Value::Null, &Value::Null)
    );
    assert_eq!(intervention["reason"], "stuck", "{status}");
    assert_eq!(intervention["nodeId"], "work");
    assert_eq!(
        (
            intervention["attempt"].clone(),
            intervention["maxAttempts"].clone()
        ),
        (json!(2), json!(2))
    );
    assert_eq!(intervention["attemptStatus"], "failure");
    assert_eq!(intervention["error"], status["invocation"]["error"]);
    assert_eq!(intervention["error"]["code"], "command.exit");
    assert_eq!(intervention["error"]["details"]["stdoutTail"], "out-line\n");
    assert_eq!(intervention["error"]["details"]["stderrTail"], "err-line\n");
    assert_eq!(
        intervention["commands"],
        json!([
            format!("spore resolve {run} --retry"),
            format!("spore resolve {run} --complete ok"),
            format!("spore resolve {run} --fail <message>"),
        ])
    );
    let stuck_since = since(intervention, before);

    let text = dir.run(&args(&["status", &run])).stdout;
    for line in [
        "needs      a person: step work gave up and has no failed arrow since ".to_string(),
        "stderr     err-line\n".to_string(),
        format!("resolve    spore resolve {run} --retry\n"),
        format!("           spore resolve {run} --fail <message>\n"),
    ] {
        assert!(text.contains(&line), "{line:?} missing from:\n{text}");
    }

    std::fs::write(dir.file("ok"), "").unwrap();
    let retry: Vec<String> = intervention["commands"][0]
        .as_str()
        .unwrap()
        .split(' ')
        .skip(1)
        .map(String::from)
        .collect();
    let retry: Vec<&str> = retry.iter().map(String::as_str).collect();
    assert_eq!(
        dir.run(&args(&retry)).code(),
        Some(0),
        "the printed command"
    );
    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(parked.code(), Some(3), "{}", parked.stdout);
    let status = dir.run(&args(&["--json", "status", &run])).json();
    let waiting = &status["waiting"];
    assert_eq!(status["intervention"], Value::Null);
    assert_eq!(waiting["nodeId"], "approve", "{status}");
    assert_eq!(waiting["signal"], "approval");
    assert_eq!(waiting["outcomes"], json!(["approved", "rejected"]));
    assert_eq!(
        waiting["outputSchemas"]["rejected"]["required"],
        json!(["reason"])
    );
    assert_eq!(waiting["outputSchemas"].get("approved"), None);
    assert_eq!(
        waiting["commands"],
        json!([
            format!("spore signal {run} approval --outcome approved"),
            format!("spore signal {run} approval --outcome rejected --data <json>"),
        ])
    );
    assert!(since(waiting, stuck_since) >= stuck_since);

    let text = dir.run(&args(&["status", &run])).stdout;
    for line in [
        "waiting    for signal approval since ".to_string(),
        "outcomes   approved\n".to_string(),
        "           rejected, with data that fits {".to_string(),
        format!("answer     spore signal {run} approval --outcome approved\n"),
    ] {
        assert!(text.contains(&line), "{line:?} missing from:\n{text}");
    }

    let approve: Vec<&str> = waiting["commands"][0]
        .as_str()
        .unwrap()
        .split(' ')
        .skip(1)
        .collect();
    assert_eq!(
        dir.run(&args(&approve)).code(),
        Some(0),
        "the printed command"
    );
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(done.code(), Some(0), "{}", done.stdout);
    let done = done.json();
    assert_eq!(
        (&done["waiting"], &done["intervention"], &done["running"]),
        (&Value::Null, &Value::Null, &Value::Null)
    );
}

#[test]
fn a_running_step_shows_its_try_and_an_unknown_outcome_its_lost_lease() {
    let dir = folder("status-unknown", "unsafe", "sleep 5");
    let before = now_ms();
    let mut child = dir.spawn(&args(&["run", "flow", "--key", "u"]));
    while dir.effects().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let list = dir.run(&args(&["--json", "list"])).json();
    let run = list["runs"][0]["runId"].as_str().unwrap().to_string();
    let status = dir.run(&args(&["--json", "status", &run])).json();
    let running = &status["running"];
    assert_eq!(status["status"], "running", "{status}");
    assert_eq!(
        (
            &running["nodeId"],
            &running["actionId"],
            &running["claimed"]
        ),
        (&json!("work"), &json!("work"), &json!(true)),
        "{status}"
    );
    assert_eq!(
        (running["attempt"].clone(), running["maxAttempts"].clone()),
        (json!(1), json!(2))
    );
    assert_eq!(
        running["invocationId"],
        status["invocation"]["invocationId"]
    );
    assert_eq!(running["state"], "started", "{status}");
    assert_eq!(running["pid"], status["invocation"]["process"]["pgid"]);
    let worker = running["worker"].as_str().expect("worker").to_string();
    since(running, before);
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains(&format!(
            "running    work attempt 1 of 2, started by {worker} since "
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(", pid {}\n", running["pid"])),
        "{text}"
    );
    assert!(
        text.contains(&format!("slots      1 of 4 in use by {worker}\n")),
        "{text}"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "u"]));
    assert_eq!(parked.code(), Some(4), "{}", parked.stdout);
    let status = parked.json();
    let intervention = &status["intervention"];
    assert_eq!(status["running"], Value::Null);
    assert_eq!(intervention["reason"], "unknown", "{status}");
    assert_eq!(intervention["attemptStatus"], "expired");
    assert_eq!(intervention["error"], Value::Null);
    assert_eq!(
        intervention["commands"][1],
        format!("spore resolve {run} --complete ok")
    );
    since(intervention, before);
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("needs      a person: step work lost its last attempt's lease"),
        "{text}"
    );
}

/// A project whose settings give a worker one slot: the run that got it shows its
/// program started, with its pid; the other shows it queued for a slot and which step
/// holds the slot.
#[test]
fn a_step_waiting_for_a_slot_shows_queued_and_who_holds_the_slots() {
    let dir = folder("status-slots", "idempotent", "sleep 5");
    std::fs::create_dir_all(dir.file(".spore")).unwrap();
    std::fs::write(
        dir.file(".spore/settings.json"),
        r#"{"worker": {"slots": 1}}"#,
    )
    .unwrap();
    let db = dir.file(".spore/runspore.db");
    let spore = |extra: &[&str]| {
        let mut cmd = dir.cmd(&args(extra));
        cmd.env("RUNSPORE_DB", &db);
        common::run(cmd)
    };
    for key in ["first", "second"] {
        assert_eq!(spore(&["start", "flow", "--key", key]).code(), Some(0));
    }
    let before = now_ms();
    let mut worker = dir
        .cmd(&args(&["worker"]))
        .env("RUNSPORE_DB", &db)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    while dir.effects().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let list = spore(&["--json", "list"]).json();
    let statuses: Vec<Value> = list["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| spore(&["--json", "status", r["runId"].as_str().unwrap()]).json())
        .collect();
    let of = |state: &str| {
        statuses
            .iter()
            .find(|s| s["running"]["state"] == state)
            .unwrap_or_else(|| panic!("no run {state}: {statuses:?}"))
    };
    let (started, queued) = (of("started"), of("queued"));
    let pid = &started["invocation"]["process"]["pgid"];
    assert!(pid.is_i64(), "{started}");
    assert_eq!(started["running"]["pid"], *pid);
    since(&started["running"], before);
    assert_eq!(
        (&queued["running"]["claimed"], &queued["running"]["pid"]),
        (&json!(false), &Value::Null)
    );
    assert_eq!(queued["running"]["queue"], json!({"place": 1, "of": 1}));
    assert_eq!(started["running"]["queue"], Value::Null);
    let slots = &queued["slots"];
    assert_eq!(slots["limit"], 1, "{queued}");
    let held = slots["held"].as_array().unwrap();
    assert_eq!(held.len(), 1, "{slots}");
    assert_eq!(
        (&held[0]["runId"], &held[0]["startKey"], &held[0]["nodeId"]),
        (&started["runId"], &started["startKey"], &json!("work"))
    );
    assert_eq!(held[0]["pid"], *pid);
    assert_eq!(held[0]["worker"], started["running"]["worker"]);
    since(&held[0], before);

    let run = queued["runId"].as_str().unwrap();
    let text = spore(&["status", run]).stdout;
    let worker_id = held[0]["worker"].as_str().unwrap();
    let start_key = started["startKey"].as_str().unwrap();
    assert!(
        text.contains("running    work attempt 1 of 2, queued for a free worker slot, 1st of 1 waiting since "),
        "{text}"
    );
    assert!(
        text.contains(&format!("slots      1 of 1 in use by {worker_id}\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "             {start_key} at work attempt 1, pid {pid} since "
        )),
        "{text}"
    );
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[test]
fn project_settings_that_spore_does_not_know_are_refused() {
    let dir = Dir::new("status-bad-settings");
    std::fs::create_dir_all(dir.file(".spore")).unwrap();
    let db = dir.file(".spore/runspore.db");
    for (settings, says) in [
        (r#"{"worker": {"slot": 2}}"#, "worker.slot is not a setting"),
        (
            r#"{"worker": {"slots": 0}}"#,
            "worker.slots: expected a whole number from 1",
        ),
        ("{", "settings.json: "),
    ] {
        std::fs::write(dir.file(".spore/settings.json"), settings).unwrap();
        let mut cmd = dir.cmd(&["--json", "worker"]);
        cmd.env("RUNSPORE_DB", &db);
        let out = common::run(cmd);
        assert_eq!(out.code(), Some(2), "{settings}: {}", out.stdout);
        let error = out.json();
        assert_eq!(
            error["error"]["code"], "project.settings-invalid",
            "{error}"
        );
        assert!(
            error["error"]["message"].as_str().unwrap().contains(says),
            "{error}"
        );
    }
}
