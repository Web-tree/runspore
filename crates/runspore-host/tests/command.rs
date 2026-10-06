//! H5: every row of the `command` result table.

use runspore_host::{ActivityContext, ActivityOutput, ActivityRunner, CommandRunner};
use runspore_types::store::RunKey;
use serde_json::{json, Value};
use tokio::sync::watch;

fn ctx(stop: watch::Receiver<bool>) -> ActivityContext {
    ActivityContext::new(
        RunKey::new("default", "run-1"),
        "node-a".into(),
        "act".into(),
        "inv-1".into(),
        2,
        "effect-1".into(),
        std::env::temp_dir(),
        stop,
    )
}

async fn run(action: Value, input: Value) -> ActivityOutput {
    let (_stop, rx) = watch::channel(false);
    CommandRunner
        .validate("act", &action)
        .expect("valid action");
    CommandRunner.run(&ctx(rx), &action, &input).await
}

fn sh(script: &str) -> Value {
    json!({"kind": "command", "argv": ["sh", "-c", script], "outcomes": ["ok", "red"]})
}

fn failure(output: &ActivityOutput) -> (&str, bool, &Value) {
    match output {
        ActivityOutput::Failure { error, retryable } => (&error.code, *retryable, &error.details),
        other => panic!("expected failure, got {other:?}"),
    }
}

fn unknown(output: &ActivityOutput) -> &str {
    match output {
        ActivityOutput::Unknown { error } => &error.code,
        other => panic!("expected unknown, got {other:?}"),
    }
}

#[tokio::test]
async fn spawn_failure_is_retryable() {
    let out = run(
        json!({"kind": "command", "argv": ["/no/such/program"]}),
        json!({}),
    )
    .await;
    assert_eq!(failure(&out).0, "command.spawn");
    assert!(failure(&out).1);
}

#[tokio::test]
async fn exit_zero_and_mapped_exit_codes_succeed() {
    let out = run(sh("printf hi"), json!({})).await;
    let expected = json!({"exitCode": 0, "stdout": "hi"});
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: expected
        }
    );
    let mut action = sh("exit 1");
    action["exitOutcomes"] = json!({"1": "red"});
    action["output"] = json!("none");
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "red".into(),
            output: Value::Null
        }
    );
}

#[tokio::test]
async fn other_exit_codes_fail_retryably_with_stderr_tail() {
    let out = run(sh("echo boom >&2; exit 3"), json!({})).await;
    let (code, retryable, details) = failure(&out);
    assert_eq!((code, retryable), ("command.exit", true));
    assert_eq!(details["exitCode"], json!(3));
    assert_eq!(details["stderrTail"], json!("boom\n"));
}

#[tokio::test]
async fn a_signal_the_engine_did_not_send_is_unknown() {
    let out = run(sh("kill -9 $$"), json!({})).await;
    assert_eq!(unknown(&out), "command.signaled");
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn timeout_is_unknown_and_kills_the_whole_group() {
    let dir = std::env::temp_dir().join(format!("runspore-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("grandchild.pid");
    let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
    let mut action = sh(&script);
    action["timeoutMs"] = json!("500");
    let out = run(action, json!({})).await;
    assert_eq!(unknown(&out), "command.timeout");
    let pid = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .to_string();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while alive(&pid) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!alive(&pid), "grandchild {pid} survived the timeout");
}

#[tokio::test]
async fn engine_stop_is_unknown_stopped() {
    let (stop, rx) = watch::channel(false);
    let action = sh("sleep 60");
    let context = ctx(rx);
    let input = json!({});
    let running = CommandRunner.run(&context, &action, &input);
    let stopper = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        stop.send_replace(true);
    };
    let (out, ()) = tokio::join!(running, stopper);
    assert_eq!(unknown(&out), "command.stopped");
}

#[tokio::test]
async fn stdout_over_64_kib_is_a_permanent_failure() {
    let out = run(sh("head -c 70000 /dev/zero"), json!({})).await;
    assert_eq!(failure(&out).0, "command.output-too-large");
    assert!(!failure(&out).1);
}

#[tokio::test]
async fn json_output_must_be_in_the_canonical_domain() {
    let mut action = sh("echo '{\"a\": 1.5}'");
    action["output"] = json!("json");
    let out = run(action, json!({})).await;
    assert_eq!(failure(&out).0, "command.malformed-output");
    assert!(!failure(&out).1);
    let mut action = sh("echo '{\"a\": [1]}'");
    action["output"] = json!("json");
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: json!({"a": [1]})
        }
    );
}

#[tokio::test]
async fn stdin_carries_the_canonical_input() {
    let out = run(sh("cat"), json!({"b": 2, "a": "x"})).await;
    let expected = json!({"exitCode": 0, "stdout": "{\"a\":\"x\",\"b\":2}"});
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: expected
        }
    );
}

#[tokio::test]
async fn runspore_variables_and_env_are_set() {
    let mut action = sh(
        "printf '%s|%s|%s|%s|%s|%s' \"$RUNSPORE_RUN_ID\" \"$RUNSPORE_NODE_ID\" \
         \"$RUNSPORE_INVOCATION_ID\" \"$RUNSPORE_ATTEMPT\" \"$RUNSPORE_EFFECT_KEY\" \"$CI\"",
    );
    action["env"] = json!({"CI": "1"});
    let out = run(action, json!({})).await;
    let expected = json!({"exitCode": 0, "stdout": "run-1|node-a|inv-1|2|effect-1|1"});
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: expected
        }
    );
}

#[test]
fn validate_rejects_undeclared_exit_outcomes_and_bad_fields() {
    let mut action = sh("true");
    action["exitOutcomes"] = json!({"2": "purple"});
    assert!(CommandRunner.validate("act", &action).is_err());
    assert!(CommandRunner
        .validate("act", &json!({"kind": "command", "argv": []}))
        .is_err());
    let mut action = sh("true");
    action["output"] = json!("xml");
    assert!(CommandRunner.validate("act", &action).is_err());
}
