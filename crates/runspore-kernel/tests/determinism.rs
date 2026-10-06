//! `spec/kernel.md` section 8: K1 (replay), K8 (supplementary-plane keys), K9
//! (map insertion order), and byte-identical output for identical requests.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::model::{code, event_kind, WORKFLOW_FORMAT};
use runspore_types::reducer::{Decision, FailureKind};
use serde_json::{json, Value};

/// Drives the example workflow through a retry, a rework loop, buffered and
/// unrouted signals, a stale result, and completion.
fn scenario(graph: &[u8], reorder: fn(&Value) -> Vec<u8>) -> Run {
    let mut run = Run::new(&json!(null));
    run.graph = graph.to_vec();
    let feed = |run: &mut Run, kind: &str, payload: Value, at: u64| {
        let mut request = run.next(kind, &payload, at);
        request.input_event.payload =
            canonical::to_vec(&canonical::parse(&reorder(&payload)).unwrap()).unwrap();
        let decision = transition(&request).expect("scenario step");
        run.snapshot = Some(decision.snapshot.clone());
        run.sequence += 1;
        run.log.push((request.input_event, decision));
    };

    feed(
        &mut run,
        event_kind::RUN_STARTED,
        started(json!({"repo": "r", "\u{fb33}": 1, "\u{1f600}": 2, "nested": {"b": 1, "a": 2}})),
        1_000,
    );
    let first = run.invocation_id();
    let mut failure = result_for(&first, 1, "failure");
    set(
        &mut failure,
        "/error",
        json!({"code": "io.broken", "message": "boom"}),
    );
    feed(&mut run, event_kind::ACTIVITY_RESULT, failure, 2_000);
    feed(
        &mut run,
        event_kind::SIGNAL_RECEIVED,
        json!({"name": "approval", "outcome": "maybe", "data": {"z": 1, "a": 2}}),
        2_100,
    );
    let mut red = result_for(&first, 2, "success");
    set(&mut red, "/outcome", json!("red"));
    set(&mut red, "/output", json!({"log": ["b", "a"], "exit": 1}));
    feed(&mut run, event_kind::ACTIVITY_RESULT, red.clone(), 3_000);
    feed(&mut run, event_kind::ACTIVITY_RESULT, red, 3_100);
    let second = run.invocation_id();
    feed(
        &mut run,
        event_kind::ACTIVITY_RESULT,
        result_for(&second, 1, "expired"),
        4_000,
    );
    let mut ok = result_for(&second, 2, "success");
    set(
        &mut ok,
        "/output",
        json!({"\u{1f600}": "astral", "\u{fb33}": "bmp", "exit": 0}),
    );
    feed(&mut run, event_kind::ACTIVITY_RESULT, ok, 5_000);
    feed(
        &mut run,
        event_kind::SIGNAL_RECEIVED,
        json!({"name": "approval", "outcome": "approved", "data": null}),
        6_000,
    );
    feed(
        &mut run,
        event_kind::SIGNAL_RECEIVED,
        json!({"name": "late"}),
        7_000,
    );
    run
}

fn sorted(value: &Value) -> Vec<u8> {
    canon(value)
}

/// Serializes with object members in reverse order and generous whitespace.
fn reversed(value: &Value) -> Vec<u8> {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Array(items) => {
                out.push_str("[ ");
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(" ,\n");
                    }
                    write(item, out);
                }
                out.push_str(" ]");
            }
            Value::Object(map) => {
                out.push_str("{\n\t");
                for (index, (key, item)) in map.iter().rev().enumerate() {
                    if index > 0 {
                        out.push_str(" , ");
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push_str(" :  ");
                    write(item, out);
                }
                out.push_str("\r\n}");
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
    let mut out = String::from("  ");
    write(value, &mut out);
    out.push('\n');
    out.into_bytes()
}

/// Serializes with object members rotated by one position.
fn rotated(value: &Value) -> Vec<u8> {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                let mut members: Vec<(&String, &Value)> = map.iter().collect();
                if !members.is_empty() {
                    members.rotate_left(1);
                }
                out.push('{');
                for (index, (key, item)) in members.into_iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(item, out);
                }
                out.push('}');
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out.into_bytes()
}

fn package(text: &[u8]) -> Vec<u8> {
    canonical::to_vec(&canonical::parse(text).expect("workflow text parses"))
        .expect("workflow is in the canonical domain")
}

fn decisions(run: &Run) -> Vec<&Decision> {
    run.log.iter().map(|(_, decision)| decision).collect()
}

#[test]
fn the_scenario_exercises_the_interesting_paths() {
    let run = scenario(&canon(&review_loop()), sorted);
    let seen: Vec<&str> = run
        .log
        .iter()
        .flat_map(|(_, decision)| codes(decision))
        .collect();
    assert_eq!(
        seen,
        [
            code::ACTIVITY_RETRY_SCHEDULED,
            code::RESULT_STALE,
            code::ACTIVITY_RETRY_SCHEDULED,
            code::SIGNAL_OUTCOME_UNROUTED,
            code::EVENT_IGNORED_TERMINAL,
        ]
    );
    let state = run.state();
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["result"],
        json!({"output": {"\u{1f600}": "astral", "\u{fb33}": "bmp", "exit": 0}})
    );
    let commands: usize = run.log.iter().map(|(_, d)| d.commands.len()).sum();
    assert_eq!(commands, 4);
}

/// K1 (C20): replaying the accepted events from sequence 1 reproduces every
/// snapshot, command and diagnostic byte for byte.
#[test]
fn replay_reproduces_every_decision() {
    let graph = canon(&review_loop());
    let original = scenario(&graph, sorted);

    let mut snapshot: Option<Vec<u8>> = None;
    for (event, decision) in &original.log {
        let replayed = transition(&request(&graph, snapshot.as_deref(), event.clone()))
            .expect("replay accepts what the run accepted");
        assert_eq!(&replayed, decision);
        assert_eq!(digest::decision(&replayed), digest::decision(decision));
        snapshot = Some(replayed.snapshot);
    }
    assert_eq!(snapshot, original.snapshot);
}

#[test]
fn the_same_request_gives_the_same_bytes() {
    let run = scenario(&canon(&review_loop()), sorted);
    let mut snapshot: Option<Vec<u8>> = None;
    for (event, decision) in &run.log {
        let request = request(&run.graph, snapshot.as_deref(), event.clone());
        let first = transition(&request).unwrap();
        let second = transition(&request).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.snapshot, second.snapshot);
        assert_eq!(&first, decision);
        snapshot = Some(first.snapshot);
    }

    let mut failing = start_request(&review_loop(), json!(null));
    failing.frozen_limits.microsteps = 0;
    assert_eq!(transition(&failing), transition(&failing));
}

/// K9: neither the key order nor the whitespace of the source documents reaches
/// the package bytes, the package digest, or any decision.
#[test]
fn key_order_and_whitespace_of_the_documents_do_not_matter() {
    let workflow = review_loop();
    let reference_package = canon(&workflow);
    let reference = scenario(&reference_package, sorted);

    for reorder in [reversed, rotated] {
        let text = reorder(&workflow);
        assert_ne!(text, reference_package, "the variant must differ in form");
        let variant_package = package(&text);
        assert_eq!(variant_package, reference_package);
        assert_eq!(
            digest::package(&variant_package),
            digest::package(&reference_package)
        );

        let variant = scenario(&variant_package, reorder);
        assert_eq!(decisions(&variant), decisions(&reference));
        for ((_, a), (_, b)) in variant.log.iter().zip(&reference.log) {
            assert_eq!(digest::decision(a), digest::decision(b));
        }
    }
}

/// The kernel takes canonical bytes only: a reordered document is rejected, not
/// silently reinterpreted.
#[test]
fn documents_that_are_not_canonical_are_rejected() {
    let workflow = review_loop();
    for reorder in [reversed, rotated] {
        let mut request = start_request(&workflow, json!({"repo": "r"}));
        request.graph = reorder(&workflow);
        assert_failure(
            transition(&request),
            FailureKind::InvalidInput,
            code::WORKFLOW_INVALID,
        );

        let mut request = start_request(&workflow, json!({"repo": "r"}));
        request.input_event.payload = reorder(&started(json!({"repo": "r"})));
        assert_failure(
            transition(&request),
            FailureKind::InvalidInput,
            code::EVENT_INVALID,
        );
    }
}

/// K8 (C15): keys beyond the basic plane keep JCS order (UTF-16 code units) in
/// everything the kernel emits, and digests are taken over those bytes.
#[test]
fn supplementary_plane_keys_keep_jcs_order() {
    let astral = "\u{1f600}";
    let high = "\u{fb33}";
    let workflow = json!({
        "format": WORKFLOW_FORMAT,
        "name": "unicode",
        "start": "work",
        "actions": {"act": {"effect": "pure"}},
        "nodes": {
            "work": {"kind": "activity", "action": "act",
                     "input": {high: {"$get": ["input", high]},
                               astral: {"$get": ["input", astral]},
                               "all": {"$get": ["input"]}},
                     "outcomes": {"ok": "done"}},
            "done": {"kind": "complete",
                     "output": {"picked": {"$get": ["nodes", "work", "output", astral]},
                                "whole": {"$get": ["nodes", "work", "output"]}}}
        }
    });
    let mut run = Run::new(&workflow);
    let decision = run.start(json!({high: "bmp", astral: "astral"}));

    let input = format!(
        "{{\"all\":{{\"{astral}\":\"astral\",\"{high}\":\"bmp\"}},\"{astral}\":\"astral\",\"{high}\":\"bmp\"}}"
    );
    let payload = String::from_utf8(decision.commands[0].payload.clone()).unwrap();
    assert!(payload.contains(&format!("\"input\":{input}")), "{payload}");
    assert_eq!(
        run.state()["invocation"]["inputDigest"],
        json!(digest::body(input.as_bytes()))
    );
    let snapshot = String::from_utf8(decision.snapshot.clone()).unwrap();
    assert!(
        snapshot.contains(&format!(
            "\"input\":{{\"{astral}\":\"astral\",\"{high}\":\"bmp\"}}"
        )),
        "{snapshot}"
    );
    assert!(snapshot.find(astral).unwrap() < snapshot.find(high).unwrap());

    let decision = run.succeed("ok", json!({high: 1, astral: 2}), 2_000);
    let snapshot = String::from_utf8(decision.snapshot.clone()).unwrap();
    assert!(
        snapshot.contains(&format!(
            "\"result\":{{\"output\":{{\"picked\":2,\"whole\":{{\"{astral}\":2,\"{high}\":1}}}}}}"
        )),
        "{snapshot}"
    );
    assert_eq!(decision.snapshot_digest, digest::state(snapshot.as_bytes()));
    assert_eq!(
        canonical::to_vec(&canonical::parse(snapshot.as_bytes()).unwrap()).unwrap(),
        decision.snapshot
    );
}

#[test]
fn everything_emitted_is_canonical() {
    let run = scenario(&canon(&review_loop()), sorted);
    for (_, decision) in &run.log {
        assert!(canonical::parse_canonical(&decision.snapshot).is_ok());
        assert_eq!(decision.snapshot_digest, digest::state(&decision.snapshot));
        for command in &decision.commands {
            assert!(canonical::parse_canonical(&command.payload).is_ok());
        }
        for diagnostic in &decision.diagnostics {
            assert!(canonical::parse_canonical(&diagnostic.details).is_ok());
        }
    }
}
