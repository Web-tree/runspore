//! H5: every row of the `command` result table.

use runspore_host::{ActivityContext, ActivityOutput, ActivityRunner, CommandRunner};
use runspore_types::store::RunKey;
use serde_json::{json, Value};
use tokio::sync::watch;

fn ctx(stop: watch::Receiver<bool>) -> ActivityContext {
    let mut ctx = ActivityContext::new(
        RunKey::new("default", "run-1"),
        "node-a".into(),
        "act".into(),
        "inv-1".into(),
        2,
        3,
        "effect-1".into(),
        std::env::temp_dir(),
        stop,
    );
    ctx.guard = env!("CARGO_BIN_EXE_runspore-guard").into();
    ctx
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
    let expected = json!({"exitCode": 0, "stdout": "hi", "stderr": "", "truncated": false});
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
async fn stdout_over_64_kib_is_a_permanent_failure_outside_text_mode() {
    for mode in ["json", "result", "none"] {
        let mut action = sh("head -c 70000 /dev/zero");
        action["output"] = json!(mode);
        let out = run(action, json!({})).await;
        assert_eq!(failure(&out).0, "command.output-too-large", "{mode}");
        assert!(!failure(&out).1);
    }
}

fn text(output: &ActivityOutput) -> (&str, &str, bool) {
    match output {
        ActivityOutput::Success { outcome, output } => {
            assert_eq!(outcome, "ok");
            (
                output["stdout"].as_str().unwrap(),
                output["stderr"].as_str().unwrap(),
                output["truncated"].as_bool().unwrap(),
            )
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn a_long_text_log_keeps_the_end_of_each_stream() {
    // About 200 KiB per stream of two-byte characters. The markers' lengths put both
    // cuts in the middle of a character, so a plain byte cut would split one.
    let dir = std::env::temp_dir().join(format!("runspore-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("out");
    let err_file = dir.join("err");
    std::fs::write(&out_file, format!("x{}OUT-END!", "é".repeat(100_000))).unwrap();
    std::fs::write(&err_file, format!("y{}ERR-END", "é".repeat(100_000))).unwrap();
    let script = format!("cat {}; cat {} >&2", out_file.display(), err_file.display());
    let out = run(sh(&script), json!({})).await;
    let (stdout, stderr, truncated) = text(&out);
    assert!(truncated);
    assert!(stdout.ends_with("OUT-END!"), "stdout keeps its end");
    assert!(stderr.ends_with("ERR-END"), "stderr keeps its end");
    assert!(
        stderr.len() <= 16 * 1024,
        "stderr is {} bytes",
        stderr.len()
    );
    assert!(stderr.len() > 16 * 1024 - 4);
    assert!(stdout.len() + stderr.len() <= 64 * 1024);
    assert!(stdout.len() + stderr.len() > 64 * 1024 - 4);
    assert!(stdout.starts_with('é') && stderr.starts_with('é'));
    assert!(!stdout.contains('\u{fffd}') && !stderr.contains('\u{fffd}'));
}

#[tokio::test]
async fn a_short_text_log_is_kept_whole() {
    let out = run(sh("printf out; printf err >&2"), json!({})).await;
    assert_eq!(text(&out), ("out", "err", false));
    let out = run(
        sh("head -c 20000 /dev/zero | tr '\\0' e >&2; printf out"),
        json!({}),
    )
    .await;
    let (stdout, stderr, truncated) = text(&out);
    assert_eq!((stdout, stderr.len(), truncated), ("out", 20000, false));
}

fn result_mode(script: &str) -> Value {
    let mut action = sh(script);
    action["output"] = json!("result");
    action
}

#[tokio::test]
async fn result_mode_success_and_failure() {
    let out = run(
        result_mode(r#"printf '{"outcome":"red","output":{"n":1}}'"#),
        json!({}),
    )
    .await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "red".into(),
            output: json!({"n": 1})
        }
    );
    for retryable in [true, false] {
        let stdout = json!({"error": {"code": "tests.flaky", "message": "m", "details": [1]},
                            "retryable": retryable});
        let out = run(result_mode(&format!("printf '%s' '{stdout}'")), json!({})).await;
        assert_eq!(failure(&out), ("tests.flaky", retryable, &json!([1])));
    }
}

#[tokio::test]
async fn result_mode_rejects_any_other_stdout_on_exit_zero() {
    for stdout in [
        "",
        "ok",
        r#"{"outcome":"ok"}"#,
        r#"{"outcome":"ok","output":null,"extra":1}"#,
        r#"{"outcome":1,"output":null}"#,
        r#"{"error":{"code":"x"},"retryable":true}"#,
        r#"{"error":{"code":"x","message":"m"},"retryable":"yes"}"#,
        r#"{"outcome":"ok","output":1.5}"#,
    ] {
        let out = run(result_mode(&format!("printf '%s' '{stdout}'")), json!({})).await;
        assert_eq!(failure(&out).0, "command.malformed-output", "{stdout}");
        assert!(!failure(&out).1);
    }
}

#[tokio::test]
async fn result_mode_maps_non_zero_exits_through_exit_outcomes() {
    let mut action = result_mode(r#"printf '{"outcome":"ok","output":1}'; exit 1"#);
    action["exitOutcomes"] = json!({"1": "red"});
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "red".into(),
            output: Value::Null
        }
    );
    let out = run(
        result_mode(r#"printf '{"outcome":"ok","output":1}'; exit 2"#),
        json!({}),
    )
    .await;
    assert_eq!(failure(&out).0, "command.exit");
}

#[tokio::test]
async fn an_undeclared_outcome_fails_in_every_mode() {
    let out = run(
        result_mode(r#"printf '{"outcome":"purple","output":null}'"#),
        json!({}),
    )
    .await;
    assert_eq!(
        failure(&out),
        (
            "command.undeclared-outcome",
            false,
            &json!({"outcome": "purple"})
        )
    );
    for mode in ["text", "json", "none"] {
        let action = json!({"kind": "command", "argv": ["sh", "-c", "echo 1"],
                            "outcomes": ["green"], "output": mode});
        let out = run(action, json!({})).await;
        assert_eq!(
            failure(&out),
            (
                "command.undeclared-outcome",
                false,
                &json!({"outcome": "ok"})
            ),
            "{mode}"
        );
    }
    let action = json!({"kind": "command", "argv": ["true"], "output": "none"});
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: Value::Null
        }
    );
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
    let expected = json!({"exitCode": 0, "stdout": "{\"a\":\"x\",\"b\":2}", "stderr": "",
                          "truncated": false});
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
        "printf '%s|%s|%s|%s|%s|%s|%s' \"$RUNSPORE_RUN_ID\" \"$RUNSPORE_NODE_ID\" \
         \"$RUNSPORE_INVOCATION_ID\" \"$RUNSPORE_ATTEMPT\" \"$RUNSPORE_VISIT\" \
         \"$RUNSPORE_EFFECT_KEY\" \"$CI\"",
    );
    action["env"] = json!({"CI": "1"});
    let out = run(action, json!({})).await;
    let expected = json!({"exitCode": 0, "stdout": "run-1|node-a|inv-1|2|3|effect-1|1",
                          "stderr": "", "truncated": false});
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
    let mut action = sh("true");
    action["output"] = json!("result");
    assert!(CommandRunner.validate("act", &action).is_ok());
    action["exitOutcomes"] = json!({"0": "red"});
    assert!(CommandRunner.validate("act", &action).is_err());
}
