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
        "needs      a person: step work it gave up and has no failed arrow since ".to_string(),
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
    since(running, before);
    let text = dir.run(&args(&["status", &run])).stdout;
    assert!(
        text.contains("running    work attempt 1 of 2, claimed, due since "),
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
        text.contains("needs      a person: step work its last attempt's lease ran out"),
        "{text}"
    );
}
