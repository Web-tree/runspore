//! The handshake between a host and a workflow (spec/host.md §2, The handshake, H35):
//! what `validate_workflow` and `start` refuse, the result channel a run records, and a
//! step a later host cannot act on.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use runspore_host::{
    validate_workflow, ActivityOutput, ActivityRegistry, CommandRunner, Engine, EngineConfig,
    NativeRunner, StepState,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::store::InvocationStatus;
use serde_json::{json, Value};

/// A workflow whose one step runs `action`, then completes.
fn one_step(action: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "runspore.workflow/0.2",
        "name": "one",
        "start": "work",
        "actions": {"work": action},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    }))
    .unwrap()
}

fn spore() -> ActivityRegistry {
    ActivityRegistry::with_builtins(NativeRunner::new())
}

fn engine(path: &std::path::Path, activities: ActivityRegistry) -> Engine {
    Engine::new(
        Arc::new(SqliteStore::open(path).unwrap()),
        Arc::new(NativeReducer),
        activities,
        EngineConfig {
            poll_ms: 5,
            ..EngineConfig::default()
        },
    )
}

#[test]
fn the_capability_set_lists_the_runners_external_and_the_store() {
    let set = spore().capabilities();
    assert_eq!(
        set.action_kinds,
        ["command", "component", "external", "native"]
    );
    assert_eq!(set.async_kinds, ["command", "component", "external"]);
    assert_eq!(set.result_channels, ["store"]);
    assert_eq!(set.component_worlds, ["runspore:runner/runner@0.1.1"]);
}

#[tokio::test]
async fn h35_validate_and_start_refuse_what_the_host_does_not_offer_and_name_the_fix() {
    let cases = [
        (
            json!({"kind": "teleport", "effect": "pure"}),
            "action.kind-unregistered",
            ["`teleport`", "`command`, `component`, `external`, `native`"],
        ),
        (
            json!({"kind": "native", "function": "work", "effect": "pure",
                   "async": {"timeoutMs": "60000"}}),
            "action.async-unsupported",
            [
                "runs `command`, `component`, `external` async",
                "remove `async`",
            ],
        ),
        (
            json!({"kind": "external", "effect": "unsafe",
                   "async": {"probe": {"component": {"ref": "./p.wasm", "export": "probe"}}}}),
            "action.component-unsupported",
            ["`component` probe", "\"command\""],
        ),
        (
            json!({"kind": "command", "argv": ["start"], "effect": "idempotent",
                   "async": {"watch": {"component": {"ref": "./w.wasm"}}}}),
            "action.component-unsupported",
            ["`component` watch", "\"command\""],
        ),
        (
            json!({"kind": "external", "effect": "unsafe", "async": {"channels": ["nats"]}}),
            "action.no-result-channel",
            ["`nats`", "offers `store`"],
        ),
        (
            json!({"kind": "command", "argv": ["start"], "effect": "idempotent",
                   "async": {"channels": ["nats"]}}),
            "action.no-result-channel",
            ["`nats`", "offers `store`"],
        ),
    ];
    let path = temp_db("h35-refuse");
    let engine = engine(&path, spore());
    for (action, code, names) in cases {
        let workflow = one_step(action);
        let limits = EngineConfig::default().limits;
        let err = validate_workflow(&NativeReducer, &spore(), limits, &workflow).unwrap_err();
        assert_eq!(err.code(), code, "{err}");
        let message = err.to_string();
        assert!(message.contains("action `work`"), "{message}");
        for name in names {
            assert!(message.contains(name), "{code}: {message} lacks {name}");
        }
        let err = engine.start(&workflow, json!({}), code).await.unwrap_err();
        assert_eq!(err.code(), code);
    }
    assert!(engine.list_runs(None, 10).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn h35_start_records_the_picked_channel_and_a_claim_returns_it() {
    let path = temp_db("h35-channel");
    let engine = engine(&path, spore());
    let workflow = one_step(json!({"kind": "external", "effect": "unsafe",
                                   "async": {"channels": ["nats", "store"]}}));
    let key = engine.start(&workflow, json!({}), "k").await.unwrap().key;
    let run = engine.get_run(&key).await.unwrap().unwrap();
    let recorded = BTreeMap::from([("work".to_string(), "store".to_string())]);
    assert_eq!(run.result_channels, recorded);
    engine.run_until_parked(&key).await.unwrap();
    let claim = engine
        .claim_step(Some(&key.run), None, None)
        .await
        .unwrap()
        .expect("a ready step");
    assert_eq!(claim.result_channel, "store");

    // A sync action records no channel.
    let sync = one_step(json!({"kind": "command", "argv": ["true"], "effect": "pure"}));
    let key = engine.start(&sync, json!({}), "sync").await.unwrap().key;
    let run = engine.get_run(&key).await.unwrap().unwrap();
    assert!(run.result_channels.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h35_a_host_without_the_runner_leaves_the_step_untouched_and_reports_it() {
    let path = temp_db("h35-missing");
    let native = NativeRunner::new().function("work", |_, _| async {
        ActivityOutput::Success {
            outcome: "ok".into(),
            output: json!("worked"),
        }
    });
    let with = engine(&path, ActivityRegistry::with_builtins(native));
    let workflow = one_step(json!({"kind": "native", "function": "work", "effect": "pure"}));
    let key = with.start(&workflow, json!({}), "k").await.unwrap().key;

    // A host that lost the runner: no attempt is claimed, and nothing fails.
    let without = engine(
        &path,
        ActivityRegistry::new().with("command", Arc::new(CommandRunner)),
    );
    let parked = without.run_until_parked(&key).await.unwrap();
    assert_eq!(parked.status, "running");
    assert_eq!(without.tick().await.unwrap().dispatched, 0);
    let invocations = without.list_invocations(&key).await.unwrap();
    assert_eq!(
        (invocations[0].status, invocations[0].attempt.is_none()),
        (InvocationStatus::Pending, true)
    );
    let step = without
        .async_step(&parked)
        .await
        .unwrap()
        .expect("a step the host falls short of");
    assert_eq!(
        (step.kind.as_str(), step.state, step.attempt),
        ("native", StepState::CapabilityMissing, 1)
    );
    let missing = step.missing.expect("what the host lacks");
    assert!(missing.contains("no runner for kind `native`"), "{missing}");
    assert_eq!(with.async_step(&parked).await.unwrap(), None);

    // A host that has it runs the step as usual.
    let done = with.run_until_parked(&key).await.unwrap();
    assert_eq!(done.status, "completed");
}
