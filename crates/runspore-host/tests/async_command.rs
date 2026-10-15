//! Async commands (spec/host.md §3, H31, H32): a start command hands its attempt to
//! a holder, the work reports with its token or through a watch, and a probe frees a
//! step whose holder is dead. The real kernel, SQLite store and guard.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use runspore_host::{
    ActivityRegistry, Answer, Engine, EngineConfig, HostError, NativeRunner, StepState,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::{Disposition, InvocationStatus, RunKey};
use serde_json::{json, Value};

/// Prints the holder handle `{"job": 7}` after keeping what it was given: its input,
/// its token and its result channel.
const START: &str = "cat > input.json; printf %s \"$RUNSPORE_TOKEN\" > token; \
                     printf %s \"$RUNSPORE_RESULT_CHANNEL\" > channel; echo '{\"job\": 7}'";

/// Answers with the file `answer`: a crash, a slow probe, or what it holds.
const PROBE: &str = "cat > /dev/null; a=$(cat answer); case \"$a\" in \
                     crash) exit 1;; slow) sleep 5;; *) printf %s \"$a\";; esac";

fn sh(script: &str) -> Value {
    json!(["/bin/sh", "-c", script])
}

/// `work`, a command `kind` with `extra` fields and `async`, then `done`, which
/// returns the step's output.
fn flow(kind: &str, extra: Value, effect: &str, attempts: u32, async_spec: Value) -> Vec<u8> {
    let mut action = json!({"kind": kind, "effect": effect, "outcomes": ["ok"],
                            "retry": {"maxAttempts": attempts, "backoffMs": "1"},
                            "async": async_spec});
    for (k, v) in extra.as_object().unwrap() {
        action[k] = v.clone();
    }
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2", "name": "async", "start": "work",
        "actions": {"work": action},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "input": {"$get": ["input"]},
                     "outcomes": {"ok": "done"},
                     "outputSchemas": {"ok": {"type": "object", "required": ["n"],
                                              "properties": {"n": {"type": "integer"}}}}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}}
        }
    }))
    .unwrap()
}

/// An async command whose start is `START`.
fn command(effect: &str, attempts: u32, async_spec: Value) -> Vec<u8> {
    flow(
        "command",
        json!({"argv": sh(START)}),
        effect,
        attempts,
        async_spec,
    )
}

struct World {
    engine: Engine,
    path: PathBuf,
    dir: PathBuf,
    key: RunKey,
}

fn config(dir: &Path) -> EngineConfig {
    EngineConfig {
        poll_ms: 5,
        heartbeat_ms: 50,
        stop_grace_ms: 200,
        base_dir: dir.to_path_buf(),
        guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
        ..EngineConfig::default()
    }
}

fn engine_on(path: &Path, config: EngineConfig) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(path).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        config,
    )
}

async fn world(name: &str, workflow: &[u8]) -> World {
    let path = temp_db(name);
    let dir = path.parent().unwrap().to_path_buf();
    let engine = engine_on(&path, config(&dir));
    let key = engine
        .start(workflow, json!({"change": 7}), name)
        .await
        .unwrap()
        .key;
    World {
        engine,
        path,
        dir,
        key,
    }
}

impl World {
    async fn parked(&self) -> String {
        self.engine
            .run_until_parked(&self.key)
            .await
            .unwrap()
            .status
    }

    async fn step(&self) -> Option<runspore_host::AsyncStep> {
        let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
        self.engine.async_step(&view).await.unwrap()
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    fn write(&self, name: &str, content: &str) {
        std::fs::write(self.dir.join(name), content).unwrap();
    }

    /// Ticks until the run has `status`, for at most 10 s.
    async fn until(&self, status: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.engine.tick().await.unwrap();
            let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
            if view.status == status {
                return;
            }
            assert!(Instant::now() < deadline, "still {}", view.status);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn last_result(&self) -> Value {
        results(&self.engine, &self.key)
            .await
            .pop()
            .expect("a result")
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_start_command_hands_over_and_the_work_reports_with_its_token() {
    let w = world(
        "h31-token",
        &command("idempotent", 1, json!({"timeoutMs": "60000"})),
    )
    .await;
    let before = now_ms();
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.expect("at an async step");
    assert_eq!(
        (step.kind.as_str(), step.state, step.attempt, step.actor),
        ("command", StepState::Held, 1, None)
    );
    assert_eq!(step.holder, Some(json!({"job": 7})));
    let deadline = step.deadline_ms.expect("a deadline");
    assert!(deadline >= before + 60_000 && deadline <= now_ms() + 60_000);
    assert_eq!(step.lease_until_ms, Some(deadline));
    let invocations = w.engine.list_invocations(&w.key).await.unwrap();
    assert_eq!(
        invocations[0].process, None,
        "hold_attempt cleared the record"
    );
    assert!(w.engine.held_slots(now_ms()).await.unwrap().is_empty());
    assert_eq!(w.read("input.json"), r#"{"change":7}"#);
    assert_eq!(w.read("channel"), "store");

    let token = w.read("token");
    let report = w
        .engine
        .complete_step(&token, "ok", json!({"n": 3}))
        .await
        .unwrap();
    assert_eq!(report.disposition, Disposition::Applied);
    assert_eq!(w.parked().await, "completed");
    assert_eq!(w.last_result().await["output"], json!({"n": 3}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_watch_says_its_holder_waits_then_delivers_the_result() {
    let watch = "cat > handle.json; \
                 echo '{\"needs\": \"person\", \"message\": \"answer in pane 2\"}'; \
                 while [ ! -f go ]; do sleep 0.02; done; \
                 echo '{\"outcome\": \"ok\", \"output\": {\"n\": 1}}'";
    let spec = json!({"timeoutMs": "60000", "watch": {"command": {"argv": sh(watch)}}});
    let w = world("h31-watch", &command("idempotent", 1, spec)).await;
    assert_eq!(
        w.parked().await,
        "running",
        "returns while the holder waits"
    );
    assert_eq!(
        w.engine.holder_waits(&w.key).as_deref(),
        Some("answer in pane 2")
    );
    assert_eq!(w.read("handle.json"), r#"{"job":7}"#);
    w.write("go", "");
    w.until("completed").await;
    assert_eq!(w.engine.holder_waits(&w.key), None);
    assert_eq!(w.last_result().await["output"], json!({"n": 1}));

    // The first result won; the work's own report comes too late.
    let late = w
        .engine
        .complete_step(&w.read("token"), "ok", json!({"n": 2}))
        .await;
    match late {
        Err(HostError::Store(f)) => assert_eq!(f.code, "lease.lost"),
        other => panic!("expected lease.lost, got {other:?}"),
    }
    assert_eq!(audits(&w.path), 1, "kept as late evidence");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_watch_result_is_checked_like_a_sync_result() {
    let watch = "echo '{\"outcome\": \"ok\", \"output\": {\"m\": 1}}'";
    let spec = json!({"watch": {"command": {"argv": sh(watch)}}});
    let w = world("h31-schema", &command("idempotent", 1, spec)).await;
    assert_eq!(w.parked().await, "needs-intervention");
    let result = w.last_result().await;
    assert_eq!(
        (&result["status"], &result["error"]["code"]),
        (&json!("failure"), &json!("output.invalid"))
    );

    let watch = "echo '{\"outcome\": \"maybe\", \"output\": null}'";
    let spec = json!({"watch": {"command": {"argv": sh(watch)}}});
    let w = world("h31-undeclared", &command("idempotent", 1, spec)).await;
    assert_eq!(w.parked().await, "needs-intervention");
    let result = w.last_result().await;
    assert_eq!(result["error"]["code"], "command.undeclared-outcome");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_watch_that_loses_sight_leaves_the_attempt_held() {
    let spec = json!({"watch": {"command": {"argv": sh("cat > /dev/null; exit 3")}}});
    let w = world("h31-lost", &command("idempotent", 1, spec)).await;
    assert_eq!(w.parked().await, "running");
    let step = w.step().await.expect("still held");
    assert_eq!(step.state, StepState::Held);
    assert!(results(&w.engine, &w.key).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_worker_takes_over_the_watch_of_an_attempt_another_one_holds() {
    let watch = "echo '{\"outcome\": \"ok\", \"output\": {\"n\": 5}}'";
    let spec = json!({"watch": {"command": {"argv": sh(watch)}}});
    let path = temp_db("h31-takeover");
    let dir = path.parent().unwrap().to_path_buf();
    let starter = engine_on(
        &path,
        EngineConfig {
            max_watches: 0,
            ..config(&dir)
        },
    );
    let workflow = command("idempotent", 1, spec);
    let key = starter
        .start(&workflow, json!({}), "takeover")
        .await
        .unwrap()
        .key;
    let parked = starter.run_until_parked(&key).await.unwrap();
    assert_eq!(parked.status, "running", "no watch here: it parks");
    let next = engine_on(&path, config(&dir));
    assert_eq!(
        next.run_until_parked(&key).await.unwrap().status,
        "completed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h31_a_start_that_prints_no_handle_fails() {
    for (name, script, code) in [
        ("h31-not-json", "echo not json", "command.malformed-output"),
        (
            "h31-too-large",
            "awk 'BEGIN { printf \"\\\"\"; for (i = 0; i < 5000; i++) printf \"x\"; printf \"\\\"\" }'",
            "command.output-too-large",
        ),
    ] {
        let workflow = flow(
            "command",
            json!({"argv": sh(script)}),
            "idempotent",
            1,
            json!({}),
        );
        let w = world(name, &workflow).await;
        assert_eq!(w.parked().await, "needs-intervention", "{name}");
        let result = w.last_result().await;
        assert_eq!(
            (&result["error"]["code"], &result["retryable"]),
            (&json!(code), &json!(false)),
            "{name}"
        );
    }
}

/// An async command with `PROBE`, whose answer each case writes.
fn probed(effect: &str, attempts: u32) -> Vec<u8> {
    let probe = json!({"command": {"argv": sh(PROBE), "timeoutMs": "300"}});
    command(effect, attempts, json!({"probe": probe}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h32_only_a_dead_answer_frees_the_step() {
    let w = world("h32-answers", &probed("idempotent", 2)).await;
    assert_eq!(w.parked().await, "running");
    for (answer, read) in [
        (r#"{"holder": "alive"}"#, Answer::Alive),
        (r#"{"holder": "unknown"}"#, Answer::Unknown),
        ("crash", Answer::Unknown),
        ("slow", Answer::Unknown),
        (r#"{"holder": "dead", "since": 1}"#, Answer::Unknown),
        ("dead", Answer::Unknown),
    ] {
        w.write("answer", answer);
        assert_eq!(
            w.engine.probe_step(&w.key).await.unwrap(),
            Some(read),
            "{answer}"
        );
        assert_eq!(
            w.step().await.expect("held").state,
            StepState::Held,
            "{answer}"
        );
    }
    w.write("answer", r#"{"holder": "dead"}"#);
    assert_eq!(
        w.engine.probe_step(&w.key).await.unwrap(),
        Some(Answer::Dead)
    );
    let invocations = w.engine.list_invocations(&w.key).await.unwrap();
    assert_eq!(invocations[0].status, InvocationStatus::Settled);
    assert_eq!(expiry_audits(&w.path), [json!({"reason": "probe-dead"})]);
    // A repeatable step gets its next try, which starts the work again.
    w.write("answer", r#"{"holder": "alive"}"#);
    assert_eq!(w.parked().await, "running");
    assert_eq!(w.step().await.expect("held again").attempt, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h32_the_sweep_of_the_next_worker_probes_and_an_unsafe_step_needs_a_person() {
    let w = world("h32-sweep", &probed("unsafe", 3)).await;
    w.write("answer", r#"{"holder": "alive"}"#);
    assert_eq!(w.parked().await, "running");
    w.write("answer", r#"{"holder": "dead"}"#);
    let next = engine_on(&w.path, config(&w.dir));
    let view = next.run_until_parked(&w.key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    assert_eq!(expiry_audits(&w.path), [json!({"reason": "probe-dead"})]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h32_a_claim_probes_a_held_external_step_first() {
    let probe = json!({"command": {"argv": sh(PROBE)}});
    let workflow = flow(
        "external",
        json!({"actor": "reviewer"}),
        "idempotent",
        2,
        json!({"probe": probe}),
    );
    let w = world("h32-claim", &workflow).await;
    assert_eq!(w.parked().await, "running");
    let e = &w.engine;
    let first = e
        .claim_step(Some(&w.key.run), Some("reviewer"), Some(json!({"pid": 1})))
        .await
        .unwrap()
        .expect("ready");
    w.write("answer", r#"{"holder": "alive"}"#);
    let held = e.claim_step(Some(&w.key.run), Some("reviewer"), None).await;
    assert_eq!(held.unwrap(), None, "an alive holder keeps it");
    w.write("answer", r#"{"holder": "dead"}"#);
    let second = e
        .claim_step(Some(&w.key.run), Some("reviewer"), None)
        .await
        .unwrap()
        .expect("freed and ready again");
    assert_eq!((first.attempt, second.attempt), (1, 2));
}
