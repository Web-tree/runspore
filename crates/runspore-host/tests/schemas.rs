//! Output schemas in the engine (spec/host.md §2, H17): results that do not fit become
//! `output.invalid`; the client operations refuse data that does not fit and record
//! nothing; `validate_workflow` refuses a schema outside the subset.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use runspore_host::{
    validate_workflow, ActivityOutput, ActivityRegistry, Engine, EngineConfig, HostError,
    NativeRunner,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::model::Resolution;
use runspore_types::reducer::Limits;
use runspore_types::store::{Disposition, RunKey};
use serde_json::{json, Value};

/// `work` (three tries allowed), then the `approval` signal. With `failed_route`, a
/// failure of `work` ends at `fallback` instead of leaving the step stuck.
fn workflow(failed_route: bool, summary: Value) -> Vec<u8> {
    let mut outcomes = json!({"ok": "approve"});
    if failed_route {
        outcomes["failed"] = json!("fallback");
    }
    let mut workflow = json!({
        "format": "runspore.workflow/0.2",
        "name": "schemas",
        "start": "work",
        "actions": {
            "work": {"kind": "native", "function": "work", "effect": "idempotent",
                     "retry": {"maxAttempts": 3}}
        },
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": outcomes,
                     "outputSchemas": {"ok": {"type": "object", "required": ["summary"],
                                              "properties": {"summary": summary}}}},
            "approve": {"kind": "await-signal", "signal": "approval",
                        "outcomes": {"approved": "done", "rejected": "work"},
                        "outputSchemas": {"rejected": {
                            "type": "object", "required": ["reason"],
                            "properties": {"reason": {"type": "string", "minLength": 1}}}}},
            "done": {"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}}
        }
    });
    // Only a `failed` arrow reaches `fallback` (W14).
    if failed_route {
        workflow["nodes"]["fallback"] =
            json!({"kind": "complete", "output": {"$get": ["nodes", "work", "output"]}});
    }
    serde_json::to_vec(&workflow).unwrap()
}

/// A function `work` that answers `{"summary": <summary>}` and counts its calls.
fn work(summary: Value, calls: Arc<AtomicUsize>) -> NativeRunner {
    NativeRunner::new().function("work", move |_ctx, _input| {
        calls.fetch_add(1, Ordering::SeqCst);
        let output = json!({"summary": summary.clone()});
        async move {
            ActivityOutput::Success {
                outcome: "ok".into(),
                output,
            }
        }
    })
}

fn engine(name: &str, summary: Value, calls: Arc<AtomicUsize>) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(temp_db(name)).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(work(summary, calls)),
        EngineConfig::default(),
    )
}

fn misfit(result: Result<impl std::fmt::Debug, HostError>) -> runspore_schema::Misfit {
    match result {
        Err(HostError::OutputInvalid(misfit)) => misfit,
        other => panic!("expected output.invalid, got {other:?}"),
    }
}

fn paths(misfit: &runspore_schema::Misfit) -> Vec<&str> {
    misfit.errors.iter().map(|e| e.path.as_str()).collect()
}

async fn start(engine: &Engine, failed_route: bool) -> RunKey {
    let started = engine
        .start(
            &workflow(failed_route, json!({"type": "string"})),
            json!(null),
            "k",
        )
        .await
        .unwrap();
    started.key
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_native_result_that_does_not_fit_is_not_retried_and_takes_the_failed_route() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = engine("schemas-failed-route", json!(7), calls.clone());
    let key = start(&engine, true).await;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a misfit is not retried");
    let results = results(&engine, &key).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["status"], "failure");
    assert_eq!(results[0]["retryable"], false);
    let error = &results[0]["error"];
    assert_eq!(error["code"], "output.invalid");
    assert_eq!(
        error["details"],
        json!({"outcome": "ok", "errors": [
            {"path": "/summary", "message": "expected type \"string\", got integer (7)"}]})
    );
    let state: Value = serde_json::from_slice(view.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(
        state["result"]["output"]["error"]["code"], "output.invalid",
        "{state}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_operations_refuse_data_that_does_not_fit_and_record_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = engine("schemas-client", json!(7), calls);
    let key = start(&engine, false).await;

    // A signal is checked before the run reaches the node that waits for it.
    let early = engine
        .signal(&key, "m0", "approval", Some("rejected".into()), json!({}))
        .await;
    assert_eq!(paths(&misfit(early)), ["/reason"]);

    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "needs-intervention");
    let invocation = engine.list_invocations(&key).await.unwrap()[0]
        .invocation_id
        .clone();
    let recorded = events(&engine, &key).await.len();

    let complete = |output: Value| Resolution::Complete {
        outcome: "ok".into(),
        output,
    };
    let refused = engine
        .resolve(&key, "r1", &invocation, complete(json!({"summary": 5})))
        .await;
    let refused = misfit(refused);
    assert_eq!(refused.outcome, "ok");
    assert_eq!(paths(&refused), ["/summary"]);
    let missing = engine
        .resolve(&key, "r2", &invocation, complete(Value::Null))
        .await;
    assert_eq!(paths(&misfit(missing)), [""]);
    assert_eq!(
        events(&engine, &key).await.len(),
        recorded,
        "nothing was recorded"
    );

    // The same request ID with data that fits is a new request, and is applied.
    let accepted = engine
        .resolve(
            &key,
            "r1",
            &invocation,
            complete(json!({"summary": "by hand"})),
        )
        .await
        .unwrap();
    assert_eq!(accepted.disposition, Disposition::Applied);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "waiting");

    let null = engine
        .signal(&key, "m1", "approval", Some("rejected".into()), Value::Null)
        .await;
    assert_eq!(
        misfit(null).errors[0].message,
        "expected type \"object\", got null"
    );
    // An outcome without a schema is not checked.
    let approved = engine
        .signal(&key, "m2", "approval", Some("approved".into()), json!(42))
        .await
        .unwrap();
    assert_eq!(approved.disposition, Disposition::Applied);
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(view.status, "completed");
}

#[tokio::test]
async fn a_schema_outside_the_subset_is_refused_before_a_run_starts() {
    let bad = workflow(true, json!({"type": "string", "pattern": "^x"}));
    let registry = ActivityRegistry::with_builtins(work(json!("x"), Arc::new(AtomicUsize::new(0))));
    let refused = validate_workflow(&NativeReducer, &registry, Limits::default(), &bad);
    let Err(HostError::Invalid { code, message }) = refused else {
        panic!("accepted: {refused:?}");
    };
    assert_eq!(code, "workflow.invalid-schema");
    assert_eq!(
        message,
        "nodes.work.outputSchemas.ok: /properties/summary/pattern: `pattern` is not in the \
         schema subset"
    );
    let engine = engine("schemas-invalid", json!("x"), Arc::new(AtomicUsize::new(0)));
    let started = engine.start(&bad, json!(null), "k").await;
    assert_eq!(started.unwrap_err().code(), "workflow.invalid-schema");
    let good = workflow(true, json!({"type": "string"}));
    validate_workflow(&NativeReducer, &registry, Limits::default(), &good).unwrap();
}

/// The `work` workflow with `schema` as its `inputSchema`.
fn with_input_schema(schema: Value) -> Vec<u8> {
    let mut package: Value =
        serde_json::from_slice(&workflow(true, json!({"type": "string"}))).unwrap();
    package["inputSchema"] = schema;
    serde_json::to_vec(&package).unwrap()
}

fn registry() -> ActivityRegistry {
    ActivityRegistry::with_builtins(work(json!("x"), Arc::new(AtomicUsize::new(0))))
}

/// H20: `start` checks the input against the package's `inputSchema` before anything
/// is created; a package without one accepts any input.
#[tokio::test]
async fn start_checks_the_input_against_the_input_schema() {
    let package = with_input_schema(json!({
        "type": "object", "required": ["task"], "additionalProperties": false,
        "properties": {"task": {"type": "string", "minLength": 1,
                                "description": "What should this run build or fix?"}}
    }));
    validate_workflow(&NativeReducer, &registry(), Limits::default(), &package).unwrap();
    let engine = engine("schemas-input", json!("x"), Arc::new(AtomicUsize::new(0)));

    let refused = engine
        .start(&package, json!({"task": "", "extra": 1}), "k")
        .await;
    let Err(HostError::InputInvalid(misfit)) = refused else {
        panic!("accepted: {refused:?}");
    };
    let paths: Vec<&str> = misfit.errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["/extra", "/task"]);
    assert_eq!(
        misfit.errors[1].message,
        "expected at least 1 characters, got 0"
    );
    let missing = engine.start(&package, Value::Null, "k").await;
    assert_eq!(missing.unwrap_err().code(), "input.invalid");
    let runs = engine.list_runs(None, 10).await.unwrap();
    assert!(runs.items.is_empty(), "a refused start creates nothing");

    let started = engine
        .start(&package, json!({"task": "t"}), "k")
        .await
        .unwrap();
    assert!(started.created);
    let plain = workflow(true, json!({"type": "string"}));
    engine.start(&plain, Value::Null, "k2").await.unwrap();
}

/// H20: an `inputSchema` whose root is not an object is refused like any schema
/// outside the subset, by `validate_workflow` and by `start`.
#[tokio::test]
async fn an_input_schema_root_must_be_an_object() {
    let engine = engine(
        "schemas-input-root",
        json!("x"),
        Arc::new(AtomicUsize::new(0)),
    );
    for root in [
        json!({"type": "string"}),
        json!({"anyOf": [{"type": "object"}]}),
    ] {
        let bad = with_input_schema(root.clone());
        let refused = validate_workflow(&NativeReducer, &registry(), Limits::default(), &bad);
        let Err(HostError::Invalid { code, message }) = refused else {
            panic!("accepted {root}: {refused:?}");
        };
        assert_eq!(code, "workflow.invalid-schema");
        assert_eq!(
            message,
            "inputSchema: /type: an input schema's root must be `\"type\": \"object\"`"
        );
        let started = engine.start(&bad, json!({}), "k").await;
        assert_eq!(started.unwrap_err().code(), "workflow.invalid-schema");
    }
    let subset =
        with_input_schema(json!({"type": "object", "properties": {"n": {"type": "number"}}}));
    let refused = validate_workflow(&NativeReducer, &registry(), Limits::default(), &subset);
    let Err(HostError::Invalid { code, message }) = refused else {
        panic!("accepted: {refused:?}");
    };
    assert_eq!(code, "workflow.invalid-schema");
    assert!(
        message.starts_with("inputSchema: /properties/n/type: "),
        "{message}"
    );
}
