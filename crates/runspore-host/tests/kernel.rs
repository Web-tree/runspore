//! Full runs: the real kernel, the real SQLite store, and the engine between them.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::*;
use runspore_host::{
    ActivityContext, ActivityOutput, ActivityRegistry, Engine, EngineConfig, NativeRunner,
    TickReport,
};
use runspore_kernel::NativeReducer;
use runspore_store_conformance::ManualClock;
use runspore_store_sqlite::SqliteStore;
use runspore_types::digest::ids;
use runspore_types::model::Resolution;
use runspore_types::reducer::Reducer;
use runspore_types::store::{RunKey, RunView, Store};
use runspore_wasmtime::WasmtimeReducer;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

const T0: u64 = 1_000_000;

/// `fetch`, then `flaky` (fails its first attempt), then the `approval` signal, then
/// `publish`; the run's output is `publish`'s output.
fn flow() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.1",
        "name": "flow",
        "start": "fetch",
        "actions": {
            "fetch": {"kind": "native", "function": "fetch", "effect": "idempotent"},
            "flaky": {"kind": "native", "function": "flaky", "effect": "idempotent",
                      "retry": {"maxAttempts": 3}},
            "publish": {"kind": "native", "function": "publish", "effect": "idempotent"}
        },
        "nodes": {
            "fetch": {"kind": "activity", "action": "fetch", "input": {"$get": ["input"]},
                      "outcomes": {"ok": "flaky"}},
            "flaky": {"kind": "activity", "action": "flaky",
                      "input": {"$get": ["nodes", "fetch", "output"]},
                      "outcomes": {"ok": "approve"}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "publish"}},
            "publish": {"kind": "activity", "action": "publish",
                        "input": {"checked": {"$get": ["nodes", "flaky", "output"]},
                                  "approval": {"$get": ["nodes", "approve", "output"]}},
                        "outcomes": {"ok": "done"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "publish", "output"]}}
        }
    }))
    .unwrap()
}

/// One activity `work` with the given effect and attempt budget, then `complete`.
fn single(effect: &str, max_attempts: u32) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.1",
        "name": "single",
        "start": "work",
        "actions": {
            "work": {"kind": "native", "function": "work", "effect": effect,
                     "retry": {"maxAttempts": max_attempts}}
        },
        "nodes": {
            "work": {"kind": "activity", "action": "work", "input": {"$get": ["input"]},
                     "outcomes": {"ok": "done"}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}}
        }
    }))
    .unwrap()
}

/// Every call of a native function: its attempt number and effect key, by function.
#[derive(Default)]
struct Effects(Mutex<BTreeMap<String, Vec<(u32, String)>>>);

impl Effects {
    fn record(&self, function: &str, ctx: &ActivityContext) {
        let mut calls = self.0.lock().unwrap();
        let entry = calls.entry(function.to_string()).or_default();
        entry.push((ctx.attempt, ctx.effect_key.clone()));
    }

    fn calls(&self, function: &str) -> Vec<(u32, String)> {
        self.0
            .lock()
            .unwrap()
            .get(function)
            .cloned()
            .unwrap_or_default()
    }

    fn counts(&self) -> BTreeMap<String, u32> {
        let calls = self.0.lock().unwrap();
        calls
            .iter()
            .map(|(f, c)| (f.clone(), c.len() as u32))
            .collect()
    }
}

fn ok(output: Value) -> ActivityOutput {
    ActivityOutput::Success {
        outcome: "ok".into(),
        output,
    }
}

fn transient() -> ActivityOutput {
    ActivityOutput::Failure {
        error: ActivityOutput::error("work.transient", "try again", Value::Null),
        retryable: true,
    }
}

/// The functions of `flow`. When `entered` is set, `flaky`'s first attempt sends an
/// acknowledgement sender through it and waits for the acknowledgement before failing.
fn flow_functions(
    effects: Arc<Effects>,
    entered: Option<mpsc::UnboundedSender<oneshot::Sender<()>>>,
) -> NativeRunner {
    let (e1, e2, e3) = (effects.clone(), effects.clone(), effects);
    NativeRunner::new()
        .function("fetch", move |ctx, input| {
            e1.record("fetch", &ctx);
            async move { ok(json!({"fetched": input})) }
        })
        .function("flaky", move |ctx, input| {
            e2.record("flaky", &ctx);
            let entered = entered.clone();
            async move {
                if ctx.attempt > 1 {
                    return ok(json!({"checked": input}));
                }
                if let Some(entered) = entered {
                    let (ack, acked) = oneshot::channel();
                    let _ = entered.send(ack);
                    let _ = acked.await;
                }
                transient()
            }
        })
        .function("publish", move |ctx, input| {
            e3.record("publish", &ctx);
            async move { ok(json!({"published": input})) }
        })
}

/// A function `work` that records its calls and answers with `answer(attempt)`.
fn work(
    effects: Arc<Effects>,
    answer: impl Fn(u32) -> ActivityOutput + Send + Sync + 'static,
) -> NativeRunner {
    let answer = Arc::new(answer);
    NativeRunner::new().function("work", move |ctx, _input| {
        effects.record("work", &ctx);
        let answer = answer.clone();
        async move { answer(ctx.attempt) }
    })
}

fn store(path: &Path, clock: Option<Arc<ManualClock>>) -> Arc<dyn Store> {
    match clock {
        Some(clock) => Arc::new(SqliteStore::open_with_clock(path, clock).unwrap()),
        None => Arc::new(SqliteStore::open(path).unwrap()),
    }
}

fn kernel_engine(
    store: Arc<dyn Store>,
    reducer: Arc<dyn Reducer>,
    native: NativeRunner,
    config: EngineConfig,
) -> Engine {
    Engine::new(
        store,
        reducer,
        ActivityRegistry::with_builtins(native),
        config,
    )
}

fn native(path: &Path, clock: Option<Arc<ManualClock>>, functions: NativeRunner) -> Engine {
    kernel_engine(
        store(path, clock),
        Arc::new(NativeReducer),
        functions,
        EngineConfig::default(),
    )
}

/// The decoded `model::State` of a run.
fn state(view: &RunView) -> Value {
    serde_json::from_slice(view.snapshot.as_deref().unwrap()).unwrap()
}

fn diagnostic_codes(events: &[runspore_types::store::EventRecord]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| e.diagnostics.iter().map(|d| d.code.clone()))
        .collect()
}

async fn h1_once(iteration: usize) {
    let path = temp_db(&format!("h1-{iteration}"));
    let effects = Arc::new(Effects::default());
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let config = |worker: &str| EngineConfig {
        worker_id: worker.into(),
        poll_ms: 1,
        ..EngineConfig::default()
    };
    let functions = || flow_functions(effects.clone(), Some(entered_tx.clone()));
    let reducer: Arc<dyn Reducer> = Arc::new(NativeReducer);
    let a = kernel_engine(
        store(&path, None),
        reducer.clone(),
        functions(),
        config("a"),
    );
    let b = kernel_engine(store(&path, None), reducer, functions(), config("b"));
    let key = a
        .start(&flow(), json!({"n": iteration}), "h1")
        .await
        .unwrap()
        .key;
    let (stop, stopped) = oneshot::channel::<()>();
    let serving = tokio::spawn({
        let b = b.clone();
        async move {
            b.serve(async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
        }
    });
    let signaller = tokio::spawn({
        let (a, key) = (a.clone(), key.clone());
        async move {
            let ack = entered_rx.recv().await.unwrap();
            let approval = json!({"by": "ops"});
            a.signal(&key, "m1", "approval", Some("approved".into()), approval)
                .await
                .unwrap();
            ack.send(()).unwrap();
        }
    });
    let view = a.run_until_parked(&key).await.unwrap();
    stop.send(()).unwrap();
    serving.await.unwrap();
    signaller.await.unwrap();

    let at = format!("iteration {iteration}");
    assert_eq!(view.status, "completed", "{at}");
    let events = events(&a, &key).await;
    assert!(events.iter().all(|e| e.consumed_revision.is_some()), "{at}");
    assert_eq!(
        view.revision,
        events.len() as u64,
        "{at}: one commit per event"
    );
    let codes = diagnostic_codes(&events);
    assert!(
        !codes.iter().any(|c| c == "result.stale"),
        "{at}: {codes:?}"
    );
    assert!(
        !codes.iter().any(|c| c == "event.ignored-terminal"),
        "{at}: {codes:?}"
    );

    let mut authorized = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for invocation in a.list_invocations(&key).await.unwrap() {
        authorized.insert(invocation.node_id.clone(), invocation.attempt_number);
        for attempt in 1..=u64::from(invocation.attempt_number) {
            expected.insert((invocation.invocation_id.clone(), attempt), 1);
        }
    }
    let mut accepted = BTreeMap::new();
    for result in results(&a, &key).await {
        let id = result["invocationId"].as_str().unwrap().to_string();
        let attempt = result["attempt"].as_u64().unwrap();
        *accepted.entry((id, attempt)).or_insert(0) += 1;
    }
    assert_eq!(accepted, expected, "{at}: accepted results per attempt");
    let want = BTreeMap::from([
        ("fetch".to_string(), 1),
        ("flaky".to_string(), 2),
        ("publish".to_string(), 1),
    ]);
    assert_eq!(authorized, want, "{at}: attempts the kernel authorized");
    assert_eq!(effects.counts(), want, "{at}: attempts executed");
    let output = &state(&view)["result"]["output"];
    let fetched = json!({"fetched": {"n": iteration}});
    let expected_output = json!({"published": {
        "checked": {"checked": fetched},
        "approval": {"by": "ops"}
    }});
    assert_eq!(output, &expected_output, "{at}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h1_two_engines_on_one_store_complete_a_run_exactly_once() {
    for iteration in 0..20 {
        h1_once(iteration).await;
    }
}

/// Starts `single("unsafe", 1)` whose first attempt reports `unknown`, and drives it
/// until it parks. Returns the engine, the run, and the invocation ID.
async fn parked_unknown(name: &str, effects: &Arc<Effects>) -> (Engine, RunKey, String) {
    let path = temp_db(name);
    let functions = work(effects.clone(), |attempt| {
        if attempt == 1 {
            ActivityOutput::Unknown {
                error: ActivityOutput::error("work.lost", "connection dropped", Value::Null),
            }
        } else {
            ok(json!({"attempt": attempt}))
        }
    });
    let engine = native(&path, None, functions);
    let key = engine
        .start(&single("unsafe", 1), json!({}), name)
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    for _ in 0..3 {
        assert_eq!(engine.tick().await.unwrap(), TickReport::default());
    }
    assert_eq!(
        effects.calls("work").len(),
        1,
        "a parked run was dispatched"
    );
    let invocation = engine.list_invocations(&key).await.unwrap()[0]
        .invocation_id
        .clone();
    (engine, key, invocation)
}

#[tokio::test]
async fn an_unknown_unsafe_effect_parks_and_resolving_complete_finishes_the_run() {
    let effects = Arc::new(Effects::default());
    let (engine, key, invocation) = parked_unknown("unsafe-complete", &effects).await;
    let resolution = Resolution::Complete {
        outcome: "ok".into(),
        output: json!({"charged": true}),
    };
    engine
        .resolve(&key, "op-1", &invocation, resolution)
        .await
        .unwrap();
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(state(&view)["result"]["output"], json!({"charged": true}));
    assert_eq!(effects.calls("work").len(), 1, "the effect ran again");
}

#[tokio::test]
async fn an_unknown_unsafe_effect_resolved_with_retry_runs_again_with_the_same_key() {
    let effects = Arc::new(Effects::default());
    let (engine, key, invocation) = parked_unknown("unsafe-retry", &effects).await;
    engine
        .resolve(&key, "op-1", &invocation, Resolution::Retry)
        .await
        .unwrap();
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(state(&view)["result"]["output"], json!({"attempt": 2}));
    let calls = effects.calls("work");
    let key_of_attempt_1 = calls[0].1.clone();
    assert_eq!(
        calls,
        vec![(1, key_of_attempt_1.clone()), (2, key_of_attempt_1)]
    );
}

/// Runs attempt 1 of `single(effect, 2)` on an engine whose runtime is then dropped
/// with the attempt in flight, then recovers on a new engine whose clock is past the
/// lease. Returns the recovered run, the calls of `work`, and the result statuses.
fn crash_then_recover(name: &str, effect: &str) -> (RunView, Vec<(u32, String)>, Vec<Value>) {
    const LEASE_MS: u64 = 1_000;
    let path = temp_db(name);
    let effects = Arc::new(Effects::default());
    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel::<()>();
    let functions = || {
        let (effects, entered) = (effects.clone(), entered_tx.clone());
        NativeRunner::new().function("work", move |ctx, input| {
            effects.record("work", &ctx);
            let entered = entered.clone();
            async move {
                if ctx.attempt == 1 {
                    let _ = entered.send(());
                    std::future::pending::<()>().await;
                }
                ok(input)
            }
        })
    };
    let config = EngineConfig {
        lease_ms: LEASE_MS,
        ..EngineConfig::default()
    };
    let runtime = |threads| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .build()
            .unwrap()
    };

    let crashed = runtime(2);
    let key = crashed.block_on(async {
        let engine = kernel_engine(
            store(&path, Some(ManualClock::new(T0))),
            Arc::new(NativeReducer),
            functions(),
            config.clone(),
        );
        let key = engine
            .start(&single(effect, 2), json!({"n": 1}), name)
            .await
            .unwrap()
            .key;
        assert_eq!(engine.tick().await.unwrap().dispatched, 1);
        entered_rx.recv().await.unwrap();
        key
    });
    crashed.shutdown_background();

    runtime(2).block_on(async {
        let engine = kernel_engine(
            store(&path, Some(ManualClock::new(T0 + LEASE_MS + 1))),
            Arc::new(NativeReducer),
            functions(),
            config,
        );
        let view = engine.run_until_parked(&key).await.unwrap();
        let statuses = results(&engine, &key)
            .await
            .into_iter()
            .map(|r| r["status"].clone())
            .collect();
        (view, effects.calls("work"), statuses)
    })
}

#[test]
fn a_crashed_idempotent_attempt_is_expired_and_repeated_with_its_effect_key() {
    let (view, calls, statuses) = crash_then_recover("crash-idempotent", "idempotent");
    assert_eq!(view.status, "completed");
    assert_eq!(statuses, vec![json!("expired"), json!("success")]);
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].1, calls[1].1,
        "the effect key changed between attempts"
    );
}

#[test]
fn a_crashed_unsafe_attempt_parks_the_run_for_an_operator() {
    let (view, calls, statuses) = crash_then_recover("crash-unsafe", "unsafe");
    assert_eq!(view.status, "needs-intervention");
    assert_eq!(statuses, vec![json!("expired")]);
    assert_eq!(calls.len(), 1);
}

/// Drives `flow` on one engine with a fixed store clock, signalling once it waits.
async fn drive_flow(name: &str, reducer: Arc<dyn Reducer>) -> (RunView, Vec<Value>) {
    let path = temp_db(name);
    let effects = Arc::new(Effects::default());
    let engine = kernel_engine(
        store(&path, Some(ManualClock::new(T0))),
        reducer,
        flow_functions(effects, None),
        EngineConfig::default(),
    );
    let key = engine
        .start(&flow(), json!({"n": 7}), "flow")
        .await
        .unwrap()
        .key;
    assert_eq!(
        engine.run_until_parked(&key).await.unwrap().status,
        "waiting"
    );
    let approval = json!({"by": "ops"});
    engine
        .signal(&key, "m1", "approval", Some("approved".into()), approval)
        .await
        .unwrap();
    let view = engine.run_until_parked(&key).await.unwrap();
    let history = events(&engine, &key)
        .await
        .into_iter()
        .map(|e| {
            json!([
                e.event.kind,
                String::from_utf8(e.event.body).unwrap(),
                e.diagnostics
            ])
        })
        .collect();
    (view, history)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_wasm_kernel_and_the_native_kernel_drive_a_run_to_identical_bytes() {
    let wasm = Arc::new(WasmtimeReducer::new().unwrap());
    let (native_view, native_history) = drive_flow("same-native", Arc::new(NativeReducer)).await;
    let (wasm_view, wasm_history) = drive_flow("same-wasm", wasm).await;
    assert_eq!(native_view.status, "completed");
    assert_eq!(wasm_view.status, "completed");
    assert_eq!(native_history, wasm_history);
    assert_eq!(native_view.snapshot_digest, wasm_view.snapshot_digest);
    assert_eq!(
        native_view.snapshot, wasm_view.snapshot,
        "the final snapshot bytes differ"
    );
}

#[tokio::test]
async fn every_attempt_of_an_invocation_carries_its_effect_key() {
    let path = temp_db("effect-key");
    let effects = Arc::new(Effects::default());
    let functions = work(effects.clone(), |attempt| {
        if attempt == 1 {
            transient()
        } else {
            ok(Value::Null)
        }
    });
    let engine = native(&path, None, functions);
    let key = engine
        .start(&single("idempotent", 2), json!({}), "effect-key")
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    let invocation = &engine.list_invocations(&key).await.unwrap()[0];
    let effect_key = ids::effect_key(&invocation.invocation_id);
    assert_eq!(
        effects.calls("work"),
        vec![(1, effect_key.clone()), (2, effect_key)]
    );
}

#[tokio::test]
async fn a_loop_ends_failed_at_its_visit_limit_without_spinning() {
    let path = temp_db("visits");
    let polls = Arc::new(AtomicUsize::new(0));
    let count = polls.clone();
    let functions = NativeRunner::new().function("poll", move |_ctx, _input| {
        count.fetch_add(1, Ordering::SeqCst);
        async {
            ActivityOutput::Success {
                outcome: "again".into(),
                output: Value::Null,
            }
        }
    });
    let workflow = serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.1",
        "name": "loop",
        "start": "poll",
        "limits": {"maxVisitsPerNode": 3},
        "actions": {
            "poll": {"kind": "native", "function": "poll", "effect": "read-only",
                     "outcomes": ["again"]}
        },
        "nodes": {
            "poll": {"kind": "activity", "action": "poll", "outcomes": {"again": "poll"}}
        }
    }))
    .unwrap();
    let engine = native(&path, None, functions);
    let key = engine
        .start(&workflow, json!({}), "visits")
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "failed");
    let error = &state(&view)["result"]["error"];
    assert_eq!(error["code"], "limit.visits-exceeded");
    assert_eq!(error["nodeId"], "poll");
    assert_eq!(polls.load(Ordering::SeqCst), 3);
    for _ in 0..3 {
        assert_eq!(engine.tick().await.unwrap(), TickReport::default());
    }
}
