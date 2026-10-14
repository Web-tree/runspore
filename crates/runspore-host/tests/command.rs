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

/// A fresh folder under the context's base folder (the temp dir), by name and absolute.
fn scratch(name: &str) -> (String, std::path::PathBuf) {
    let name = format!("runspore-cwd-{name}-{}", std::process::id());
    let path = std::env::temp_dir().join(&name);
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    (name, path.canonicalize().unwrap())
}

fn in_folder(pointer: &str) -> Value {
    let mut action = sh("pwd -P");
    action["cwd"] = json!({ "fromInput": pointer });
    action
}

#[tokio::test]
async fn cwd_from_input_runs_in_the_folder_the_input_names() {
    let (name, path) = scratch("ok");
    let absolute = json!({"place": {"dir": path}});
    let relative = json!({"place": {"dir": name}});
    for input in [absolute, relative] {
        let out = run(in_folder("/place/dir"), input.clone()).await;
        let ActivityOutput::Success { output, .. } = out else {
            panic!("{input}: {out:?}");
        };
        assert_eq!(output["stdout"], json!(format!("{}\n", path.display())));
    }
    // RFC 6901 escapes: `~1` is `/`, `~0` is `~`.
    let input = json!({"a/b": {"~c": path}});
    let out = run(in_folder("/a~1b/~0c"), input).await;
    assert!(matches!(out, ActivityOutput::Success { .. }), "{out:?}");
    std::fs::remove_dir_all(&path).unwrap();
}

#[tokio::test]
async fn cwd_from_input_that_names_no_folder_is_cwd_invalid() {
    let (_, path) = scratch("invalid");
    let file = path.join("file");
    std::fs::write(&file, "").unwrap();
    let cases = [
        (json!({}), json!({"pointer": "/dir"})),
        (json!({"dir": null}), json!({"pointer": "/dir"})),
        (json!({"dir": 7}), json!({"pointer": "/dir"})),
        (
            json!({"dir": "runspore-cwd-no-such-folder"}),
            json!({"pointer": "/dir", "cwd": "runspore-cwd-no-such-folder"}),
        ),
        (
            json!({"dir": file}),
            json!({"pointer": "/dir", "cwd": file}),
        ),
    ];
    for (input, expected) in cases {
        let out = run(in_folder("/dir"), input.clone()).await;
        let (code, retryable, details) = failure(&out);
        assert_eq!((code, retryable), ("command.cwd-invalid", false), "{input}");
        assert_eq!(details, &expected, "{input}");
    }
    std::fs::remove_dir_all(&path).unwrap();
}

#[test]
fn validate_accepts_only_a_string_or_a_from_input_pointer_as_cwd() {
    for cwd in [
        json!("."),
        json!({"fromInput": ""}),
        json!({"fromInput": "/a/~0b~1"}),
    ] {
        let mut action = sh("true");
        action["cwd"] = cwd.clone();
        assert!(CommandRunner.validate("act", &action).is_ok(), "{cwd}");
    }
    for cwd in [
        json!(7),
        json!({}),
        json!({"fromInput": 1}),
        json!({"fromInput": "dir"}),
        json!({"fromInput": "/a~2"}),
        json!({"fromInput": "/a~"}),
        json!({"fromInput": "/a", "other": 1}),
    ] {
        let mut action = sh("true");
        action["cwd"] = cwd.clone();
        assert!(CommandRunner.validate("act", &action).is_err(), "{cwd}");
    }
}

/// A json-mode action whose `result` block reads the stdout `script` prints.
fn mapped(script: &str, outcome: Value, output: Value) -> Value {
    let mut action = sh(script);
    action["output"] = json!("json");
    action["result"] = json!({"outcome": outcome, "output": output});
    action
}

/// `{"$get": ["stdout", ...path]}`.
fn from_stdout(path: &[&str]) -> Value {
    let mut steps = vec!["stdout"];
    steps.extend_from_slice(path);
    json!({ "$get": steps })
}

#[tokio::test]
async fn a_result_block_maps_outcome_and_output_from_json_stdout() {
    let stdout = json!({"is_error": false, "session_id": "s",
                        "structured_output": {"outcome": "red", "output": {"n": 1}}});
    let action = mapped(
        &format!("printf '%s' '{stdout}'"),
        from_stdout(&["structured_output", "outcome"]),
        from_stdout(&["structured_output", "output"]),
    );
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "red".into(),
            output: json!({"n": 1})
        }
    );
}

#[tokio::test]
async fn a_result_block_reads_input_params_run_and_step() {
    let output = json!({
        "input": {"$get": ["input", "task"]},
        "params": {"$get": ["params", "tools"]},
        "run": {"$get": ["run"]},
        "step": {"$get": ["step"]},
        "joined": {"$join": [{"$get": ["stdout", "words"]}, " "]},
    });
    let stdout = json!({"words": ["a", "b"]});
    let action = mapped(&format!("printf '%s' '{stdout}'"), json!("ok"), output);
    CommandRunner
        .validate("act", &action)
        .expect("valid action");
    let (_stop, rx) = watch::channel(false);
    let mut context = ctx(rx);
    context.params = json!({"tools": ["Read"]});
    let out = CommandRunner
        .run(&context, &action, &json!({"task": "t"}))
        .await;
    let expected = json!({
        "input": "t",
        "params": ["Read"],
        "run": {"id": "run-1", "tenant": "default"},
        "step": {"node": "node-a", "visit": 3, "attempt": 2, "resumed": true},
        "joined": "a b",
    });
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: expected
        }
    );
}

#[tokio::test]
async fn a_result_block_that_does_not_resolve_fails_with_the_end_of_stdout() {
    let stdout = json!({"structured_output": {"outcome": 7, "output": null}}).to_string();
    let script = format!("printf '%s' '{stdout}'");
    let cases = [
        (
            from_stdout(&["structured_output", "verdict"]),
            json!(null),
            "command.result-invalid",
            json!({"field": "outcome", "path": ["stdout", "structured_output", "verdict"]}),
        ),
        (
            from_stdout(&["structured_output", "outcome"]),
            json!(null),
            "command.result-invalid",
            json!({"field": "outcome", "value": 7}),
        ),
        (
            json!("purple"),
            json!(null),
            "command.undeclared-outcome",
            json!({"outcome": "purple"}),
        ),
        (
            json!("ok"),
            from_stdout(&["nothing"]),
            "command.result-invalid",
            json!({"field": "output", "path": ["stdout", "nothing"]}),
        ),
    ];
    for (outcome, output, code, mut details) in cases {
        let out = run(mapped(&script, outcome, output), json!({})).await;
        details["stdoutTail"] = json!(stdout);
        assert_eq!(failure(&out), (code, false, &details), "{details}");
    }
}

#[tokio::test]
async fn a_result_block_failure_keeps_only_the_end_of_a_long_stdout() {
    let script = r#"printf '{"pad":"'; head -c 10000 /dev/zero | tr '\0' x; printf '"}'"#;
    let out = run(
        mapped(script, from_stdout(&["outcome"]), json!(null)),
        json!({}),
    )
    .await;
    let (code, _, details) = failure(&out);
    assert_eq!(code, "command.result-invalid");
    let kept = details["stdoutTail"].as_str().unwrap();
    assert_eq!(kept.len(), 4 * 1024);
    assert!(kept.ends_with("xx\"}"), "{kept}");
}

#[tokio::test]
async fn a_mapped_exit_code_gives_its_outcome_and_the_parsed_stdout() {
    let mut action = mapped(
        r#"printf '{"a":1}'; exit 1"#,
        from_stdout(&["missing"]),
        from_stdout(&["missing"]),
    );
    action["exitOutcomes"] = json!({"1": "red"});
    let out = run(action, json!({})).await;
    assert_eq!(
        out,
        ActivityOutput::Success {
            outcome: "red".into(),
            output: json!({"a": 1})
        }
    );
}

#[test]
fn validate_takes_a_result_block_only_in_json_mode_with_its_roots() {
    let good = mapped("true", from_stdout(&["o"]), from_stdout(&["p"]));
    assert!(CommandRunner.validate("act", &good).is_ok());
    for mode in [None, Some("text"), Some("result"), Some("none")] {
        let mut action = good.clone();
        match mode {
            Some(mode) => action["output"] = json!(mode),
            None => drop(action.as_object_mut().unwrap().remove("output")),
        }
        assert!(CommandRunner.validate("act", &action).is_err(), "{mode:?}");
    }
    for (outcome, output) in [
        (json!({"$get": ["nodes", "a"]}), json!(null)),
        (json!("ok"), json!({"$get": ["from"]})),
        (json!({"$nope": 1}), json!(null)),
    ] {
        let action = mapped("true", outcome.clone(), output.clone());
        assert!(
            CommandRunner.validate("act", &action).is_err(),
            "{outcome} {output}"
        );
    }
    for block in [
        json!("x"),
        json!({"outcome": "ok"}),
        json!({"outcome": "ok", "output": 1, "extra": 2}),
    ] {
        let mut action = good.clone();
        action["result"] = block.clone();
        assert!(CommandRunner.validate("act", &action).is_err(), "{block}");
    }
    let mut action = good;
    action["exitOutcomes"] = json!({"0": "red"});
    assert!(
        CommandRunner.validate("act", &action).is_err(),
        "exit 0 reads stdout"
    );
}

/// `sh -c 'printf "%s|" "$@"' sh <items>`: prints each argument after the script.
fn printing(items: Value) -> Value {
    let mut argv = vec![
        json!("sh"),
        json!("-c"),
        json!("printf '%s|' \"$@\""),
        json!("sh"),
    ];
    argv.extend(items.as_array().unwrap().iter().cloned());
    json!({"kind": "command", "argv": argv})
}

async fn printed(action: Value, ctx: &ActivityContext, input: Value) -> String {
    CommandRunner
        .validate("act", &action)
        .expect("valid action");
    match CommandRunner.run(ctx, &action, &input).await {
        ActivityOutput::Success { output, .. } => output["stdout"].as_str().unwrap().to_string(),
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn argv_mappings_read_input_params_run_and_step() {
    let (_stop, rx) = watch::channel(false);
    let mut ctx = ctx(rx);
    ctx.params = json!({"tools": ["Read", "Edit"]});
    let action = printing(json!([
        "plain",
        {"$get": ["input", "repo"]},
        {"$join": [{"$get": ["params", "tools"]}, ","]},
        {"$get": ["run", "id"]},
        {"$get": ["run", "tenant"]},
        {"$get": ["step", "node"]},
        {"$json": [{"$get": ["step", "visit"]}, {"$get": ["step", "attempt"]}]},
        {"$if": [{"$get": ["step", "resumed"]}, "--resume", "--session-id"]},
        {"$uuid": [{"$get": ["run", "id"]}, {"$get": ["step", "node"]}]},
    ]));
    let out = printed(action, &ctx, json!({"repo": "shop"})).await;
    let session = runspore_types::digest::ids::uuid(br#"["run-1","node-a"]"#);
    assert_eq!(
        out,
        format!("plain|shop|Read,Edit|run-1|default|node-a|[3,2]|--resume|{session}|")
    );
}

#[tokio::test]
async fn step_resumed_is_false_only_on_the_first_try_of_the_first_visit() {
    let resumed = printing(json!([{"$json": {"$get": ["step", "resumed"]}}]));
    for (attempt, visit, expected) in [(1, 1, "false|"), (2, 1, "true|"), (1, 2, "true|")] {
        let (_stop, rx) = watch::channel(false);
        let mut ctx = ctx(rx);
        (ctx.attempt, ctx.visit) = (attempt, visit);
        let out = printed(resumed.clone(), &ctx, json!({})).await;
        assert_eq!(out, expected, "attempt {attempt}, visit {visit}");
    }
}

#[tokio::test]
async fn an_argv_mapping_that_gives_no_string_starts_nothing() {
    let (_, path) = scratch("argv");
    let started = path.join("started");
    let touch = |item: Value| json!({"kind": "command", "argv": ["sh", "-c", "touch \"$0\"", started, item]});
    let cases = [
        (
            touch(json!({"$get": ["input", "n"]})),
            "command.argv-invalid",
            json!({"index": 4, "value": 7}),
        ),
        (
            touch(json!({"$get": ["params", "tools"]})),
            "command.argv-invalid",
            json!({"index": 4, "value": ["Read"]}),
        ),
        (
            touch(json!({"$get": ["input", "missing"]})),
            "mapping.missing-path",
            json!({"path": ["input", "missing"]}),
        ),
        (
            touch(json!({"$join": [{"$get": ["input", "n"]}, ","]})),
            "mapping.invalid-operand",
            json!({"operator": "$join", "path": ["argv", 4, "$join", 0], "value": 7}),
        ),
    ];
    for (action, code, expected) in cases {
        let (_stop, rx) = watch::channel(false);
        let mut ctx = ctx(rx);
        ctx.params = json!({"tools": ["Read"]});
        CommandRunner
            .validate("act", &action)
            .expect("valid action");
        let out = CommandRunner.run(&ctx, &action, &json!({"n": 7})).await;
        let (got, retryable, details) = failure(&out);
        assert_eq!((got, retryable), (code, false), "{action}");
        assert_eq!(details, &expected, "{action}");
        assert!(!started.exists(), "{action} started its program");
    }
    std::fs::remove_dir_all(&path).unwrap();
}

#[test]
fn validate_checks_argv_mappings_and_their_roots() {
    for item in [
        json!({"$get": ["input", "a"]}),
        json!({"$get": ["params", "a"], "default": "x"}),
        json!({"$get": ["run", "id"]}),
        json!({"$get": ["step", "resumed"]}),
        json!({"$literal": "x"}),
    ] {
        let action = printing(json!([item.clone()]));
        assert!(CommandRunner.validate("act", &action).is_ok(), "{item}");
    }
    for item in [
        json!({"$get": ["nodes", "a"]}),
        json!({"$get": ["from", "node"]}),
        json!({"$get": "input"}),
        json!({"$if": [true, "a"]}),
        json!({"$nope": 1}),
        json!(7),
        json!(["a"]),
        json!(null),
    ] {
        let action = printing(json!([item.clone()]));
        assert!(CommandRunner.validate("act", &action).is_err(), "{item}");
    }
}
