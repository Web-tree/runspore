//! Candidate answers (spec/host.md §2, Candidate answers; H46): the work of a held
//! step proposes a checked answer without completing the step, replaces or withdraws
//! it, and the action's probe commits it once it says the work has stopped. The real
//! kernel, the SQLite store, and the test runner of `crates/runspore-test-runner` on
//! both tracks of its world.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::*;
use runspore_host::{
    ActivityRegistry, Answer, Engine, EngineConfig, HostError, NativeRunner, StepToken,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::RunKey;
use serde_json::{json, Value};

/// The test runner as the 0.1 track's `runspore:runner@0.1.1` built it, before the
/// probe could answer `stopped`.
const TEST_RUNNER_0_1_1: &[u8] = include_bytes!("fixtures/test-runner-0.1.1.wasm");

const DOT: &str = r#"digraph candidate {
  dialect = "runspore/0.2";
  start = "work";
  done [kind="complete"];
  redo [kind="complete"];
  work -> done [on="ok"];
  work -> redo [on="changes"];
}
"#;

/// A run of `work`, the test runner, whose handle is `{"answer": <file>}`, so the
/// probe answers what the file says, with the token of its held attempt.
struct Held {
    dir: PathBuf,
    path: PathBuf,
    engine: Engine,
    key: RunKey,
    token: String,
}

/// The settings of the run, with `async` for the action when given.
fn settings(effect: &str, attempts: u32, asynchronous: Option<Value>) -> Value {
    let mut action = json!({"kind": "component", "module": "runner.wasm", "effect": effect,
                            "outcomes": ["ok", "changes"],
                            "grants": {"folders": ["."], "writeFolders": ["."]},
                            "retry": {"maxAttempts": attempts}});
    if let Some(asynchronous) = asynchronous {
        action["async"] = asynchronous;
    }
    json!({
        "format": "runspore.workflow/0.2",
        "actions": {"work": action},
        "nodes": {"work": {"action": "work", "input": {"$get": ["input"]},
                           "outputSchemas": {"ok": {"type": "object", "required": ["summary"],
                                                     "properties": {"summary": {"type": "string"}}}}}}
    })
}

fn worker(path: &Path) -> Engine {
    let dir = path.parent().unwrap();
    Engine::new(
        Arc::new(SqliteStore::open(path).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        EngineConfig {
            poll_ms: 5,
            heartbeat_ms: 50,
            stop_grace_ms: 200,
            base_dir: dir.to_path_buf(),
            files_dir: Some(dir.join("store.db.files")),
            guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
            ..EngineConfig::default()
        },
    )
}

/// Starts a run of `runner` with `settings`, works it until its step is held, and
/// takes the token of the held attempt.
async fn held(name: &str, runner: &[u8], settings: &Value) -> Held {
    held_with(name, runner, settings, None).await
}

/// [`held`] with `handle` as the holder handle instead of the answer file's.
async fn held_with(name: &str, runner: &[u8], settings: &Value, handle: Option<Value>) -> Held {
    let path = temp_db(name);
    let dir = path.parent().unwrap().to_path_buf();
    let folder = dir.join("workflow");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("workflow.dot"), DOT).unwrap();
    fs::write(folder.join("settings.json"), settings.to_string()).unwrap();
    fs::write(folder.join("runner.wasm"), runner).unwrap();
    let sealed = runspore_build::build(&folder).unwrap().seal().unwrap();
    let artifact = runspore_wasmtime::load_artifact(&sealed.bytes).unwrap();
    fs::write(dir.join("answer"), "alive").unwrap();
    let handle = handle.unwrap_or_else(|| json!({"answer": dir.join("answer").to_string_lossy()}));
    let input = json!({"handle": handle.to_string()});
    let engine = worker(&path);
    let key = engine
        .start_artifact(&artifact, input, name)
        .await
        .unwrap()
        .key;
    engine.run_until_parked(&key).await.unwrap();
    let invocation = engine.list_invocations(&key).await.unwrap().remove(0);
    let attempt = invocation.attempt.expect("held");
    let token = StepToken {
        run: key.run.clone(),
        invocation_id: attempt.invocation_id,
        attempt: invocation.attempt_number,
        owner: attempt.owner,
        fence: attempt.fence,
    }
    .to_string();
    Held {
        dir,
        path,
        engine,
        key,
        token,
    }
}

impl Held {
    fn answer(&self, answer: &str) {
        fs::write(self.dir.join("answer"), answer).unwrap();
    }

    async fn status(&self) -> String {
        self.engine
            .get_run(&self.key)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    /// The candidate `status` shows: its outcome, and whether its time is set.
    async fn candidate(&self) -> Option<String> {
        let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
        let step = self.engine.async_step(&view).await.unwrap()?;
        step.candidate.map(|c| {
            assert!(c.at_ms > 0);
            c.outcome
        })
    }

    async fn results(&self) -> Vec<Value> {
        results(&self.engine, &self.key).await
    }

    async fn propose(&self, outcome: &str, output: Value) -> Result<(), HostError> {
        self.engine
            .propose_step(&self.token, outcome, output)
            .await
            .map(|proposed| assert_eq!(proposed.candidate.outcome, outcome))
    }
}

fn code(e: HostError) -> String {
    e.code().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_a_proposal_is_checked_held_replaced_and_withdrawn() {
    let run = held(
        "h46-propose",
        runspore_wasmtime::TEST_RUNNER,
        &settings("idempotent", 1, None),
    )
    .await;
    assert_eq!(run.candidate().await, None);
    // A misfit is refused with `complete_step`'s error, and nothing is stored.
    let misfit = run.propose("ok", json!({"summary": 3})).await.unwrap_err();
    assert_eq!(code(misfit), "output.invalid");
    let undeclared = run.propose("maybe", json!({})).await.unwrap_err();
    assert_eq!(code(undeclared), "step.undeclared-outcome");
    assert_eq!(run.candidate().await, None);

    run.propose("changes", json!(["a"])).await.unwrap();
    assert_eq!(run.candidate().await.as_deref(), Some("changes"));
    run.propose("ok", json!({"summary": "done"})).await.unwrap();
    assert_eq!(run.candidate().await.as_deref(), Some("ok"), "replaced");
    // A misfit leaves the last one.
    assert!(run.propose("ok", json!({})).await.is_err());
    assert_eq!(run.candidate().await.as_deref(), Some("ok"));

    // Alive and unknown leave the step held with its candidate.
    for answer in ["alive", "nonsense"] {
        run.answer(answer);
        let said = run.engine.probe_step(&run.key).await.unwrap();
        assert!(!said.unwrap().ends(), "{answer}: {said:?}");
        assert_eq!(run.candidate().await.as_deref(), Some("ok"));
    }

    let withdrawn = run.engine.withdraw_step(&run.token).await.unwrap();
    assert!(withdrawn.withdrawn);
    assert_eq!(run.candidate().await, None);
    let again = run.engine.withdraw_step(&run.token).await.unwrap();
    assert!(!again.withdrawn, "withdrawing nothing is harmless");
    assert!(run.results().await.is_empty(), "nothing was reported");
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_stopped_commits_the_candidate_held_at_the_commit() {
    let run = held(
        "h46-stopped",
        runspore_wasmtime::TEST_RUNNER,
        &settings("idempotent", 1, None),
    )
    .await;
    run.propose("changes", json!(["old"])).await.unwrap();
    run.propose("ok", json!({"summary": "new"})).await.unwrap();
    run.answer("stopped");
    assert_eq!(
        run.engine.probe_step(&run.key).await.unwrap(),
        Some(Answer::Stopped)
    );
    run.engine.run_until_parked(&run.key).await.unwrap();
    assert_eq!(run.status().await, "completed");
    let results = run.results().await;
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(
        (
            &results[0]["status"],
            &results[0]["outcome"],
            &results[0]["output"]
        ),
        (&json!("success"), &json!("ok"), &json!({"summary": "new"}))
    );
    // The attempt has ended: a late proposal and a late withdrawal are refused.
    for late in [
        run.propose("ok", json!({"summary": "late"}))
            .await
            .unwrap_err(),
        run.engine.withdraw_step(&run.token).await.unwrap_err(),
    ] {
        assert_eq!(code(late), "lease.lost");
    }
    assert_eq!(audits(&run.path), 0, "a late proposal is not kept");
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_stopped_without_a_candidate_fails_the_try_retryable() {
    let run = held(
        "h46-none",
        runspore_wasmtime::TEST_RUNNER,
        &settings("idempotent", 2, None),
    )
    .await;
    run.answer("stopped");
    assert_eq!(
        run.engine.probe_step(&run.key).await.unwrap(),
        Some(Answer::Stopped)
    );
    let results = run.results().await;
    assert_eq!(results[0]["status"], "failure", "{results:?}");
    assert_eq!(results[0]["error"]["code"], "step.no-answer");
    assert_eq!(results[0]["retryable"], true);
    // The kernel tries again: the second attempt is held, with no candidate.
    run.answer("alive");
    run.engine.run_until_parked(&run.key).await.unwrap();
    let view = run.engine.get_run(&run.key).await.unwrap().unwrap();
    let step = run.engine.async_step(&view).await.unwrap().expect("held");
    assert_eq!((step.attempt, step.candidate), (2, None));
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_a_dead_holder_commits_its_candidate_on_either_track() {
    // The 0.1.1 test runner knows no `answer` file, but answers a `probe` handle.
    for (name, runner, handle) in [
        ("h46-dead", runspore_wasmtime::TEST_RUNNER, None),
        (
            "h46-dead-0-1",
            TEST_RUNNER_0_1_1,
            Some(json!({"probe": "dead"})),
        ),
    ] {
        let run = held_with(name, runner, &settings("unsafe", 1, None), handle).await;
        run.propose("changes", json!([])).await.unwrap();
        run.answer("dead");
        let said = run.engine.probe_step(&run.key).await.unwrap();
        assert_eq!(said, Some(Answer::Dead), "{name}");
        run.engine.run_until_parked(&run.key).await.unwrap();
        assert_eq!(run.status().await, "completed", "{name}");
        assert_eq!(run.results().await[0]["outcome"], "changes", "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_a_command_probe_that_says_stopped_commits_the_candidate() {
    let probe = json!({"command": {"argv": ["sh", "-c", "printf '{\"holder\":\"stopped\"}'"]}});
    let run = held(
        "h46-command",
        runspore_wasmtime::TEST_RUNNER,
        &settings("idempotent", 1, Some(json!({"probe": probe}))),
    )
    .await;
    run.propose("ok", json!({"summary": "by command"}))
        .await
        .unwrap();
    assert_eq!(
        run.engine.probe_step(&run.key).await.unwrap(),
        Some(Answer::Stopped)
    );
    run.engine.run_until_parked(&run.key).await.unwrap();
    assert_eq!(run.status().await, "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_two_engines_that_commit_at_once_complete_the_step_once() {
    let run = held(
        "h46-race",
        runspore_wasmtime::TEST_RUNNER,
        &settings("idempotent", 1, None),
    )
    .await;
    run.propose("ok", json!({"summary": "once"})).await.unwrap();
    run.answer("stopped");
    let second = worker(&run.path);
    let (a, b) = tokio::join!(run.engine.probe_step(&run.key), second.probe_step(&run.key));
    assert!(a.is_ok() && b.is_ok(), "{a:?} {b:?}");
    let results = run.results().await;
    assert_eq!(results.len(), 1, "one result: {results:?}");
    assert_eq!(results[0]["output"], json!({"summary": "once"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn h46_an_abandon_or_a_cancel_commits_nothing() {
    for how in ["abandon", "cancel"] {
        let run = held(
            &format!("h46-{how}"),
            runspore_wasmtime::TEST_RUNNER,
            &settings("idempotent", 2, None),
        )
        .await;
        run.propose("ok", json!({"summary": "never"}))
            .await
            .unwrap();
        match how {
            "abandon" => {
                run.engine.abandon_step(&run.key).await.unwrap();
            }
            _ => {
                run.engine.cancel(&run.key, "stop", None).await.unwrap();
            }
        }
        let results = run.results().await;
        assert_eq!(results.len(), 1, "{how}: {results:?}");
        assert_eq!(results[0]["status"], "expired", "{how}");
        if how == "abandon" {
            run.engine.run_until_parked(&run.key).await.unwrap();
            let view = run.engine.get_run(&run.key).await.unwrap().unwrap();
            let step = run.engine.async_step(&view).await.unwrap().expect("held");
            assert_eq!(
                (step.attempt, step.candidate),
                (2, None),
                "the next attempt holds none"
            );
        }
    }
}
