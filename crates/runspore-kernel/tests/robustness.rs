//! The kernel never panics: every request, however malformed, returns a decision
//! or a failure. A panic anywhere below fails the test that reached it.

mod common;

use common::*;
use runspore_kernel::transition;
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::model::event_kind;
use runspore_types::reducer::TransitionRequest;
use serde_json::{json, Value};

/// Valid requests covering every event kind and every non-terminal status.
fn corpus() -> Vec<TransitionRequest> {
    let mut requests = Vec::new();
    let workflow = with(review_loop(), "/actions/build/effect", json!("unsafe"));

    let mut run = Run::new(&workflow);
    requests.push(run.next(
        event_kind::RUN_STARTED,
        &started(json!({"repo": "r"})),
        START_MS,
    ));
    run.start(json!({"repo": "r"}));
    let invocation = run.invocation_id();
    let resolutions = [
        json!({"action": "retry"}),
        json!({"action": "complete", "outcome": "ok", "output": [1]}),
        json!({"action": "complete", "outcome": "blue"}),
        json!({"action": "fail", "error": {"code": "x", "message": "m"}}),
    ];

    let push_all = |run: &Run, requests: &mut Vec<TransitionRequest>| {
        for status in ["success", "failure", "unknown", "expired"] {
            for attempt in [1, 2] {
                requests.push(run.next(
                    event_kind::ACTIVITY_RESULT,
                    &result_for(&invocation, attempt, status),
                    5_000,
                ));
            }
        }
        for outcome in [Some("approved"), Some("rejected"), Some("zzz"), None] {
            requests.push(run.next(
                event_kind::SIGNAL_RECEIVED,
                &json!({"name": "approval", "outcome": outcome, "data": {"k": [1]}}),
                5_000,
            ));
        }
        for resolution in &resolutions {
            requests.push(run.next(
                event_kind::INVOCATION_RESOLVED,
                &json!({"invocationId": invocation, "resolution": resolution}),
                5_000,
            ));
        }
    };

    push_all(&run, &mut requests);
    run.signal("approval", Some("zzz"), json!(1), 1_500);
    run.fail(true, 2_000);
    push_all(&run, &mut requests);
    run.lose("unknown", 3_000);
    assert_eq!(run.status(), "needs-intervention");
    push_all(&run, &mut requests);
    run.resolve(
        &invocation,
        json!({"action": "complete", "outcome": "ok"}),
        4_000,
    );
    assert_eq!(run.status(), "waiting");
    push_all(&run, &mut requests);
    run.signal("approval", Some("approved"), json!(null), 4_500);
    assert_eq!(run.status(), "completed");
    push_all(&run, &mut requests);
    requests
}

fn pointers(value: &Value, prefix: &str, out: &mut Vec<String>) {
    out.push(prefix.to_string());
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                pointers(item, &format!("{prefix}/{key}"), out);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                pointers(item, &format!("{prefix}/{index}"), out);
            }
        }
        _ => {}
    }
}

fn replacements() -> Vec<Value> {
    vec![
        json!(null),
        json!(true),
        json!(0),
        json!(1),
        json!(-1),
        json!(1_000),
        json!(4_294_967_295u64),
        json!(4_294_967_296u64),
        json!(9_007_199_254_740_991u64),
        json!(""),
        json!("x"),
        json!("0"),
        json!("9223372036854775807"),
        json!("9223372036854775808"),
        json!("build"),
        json!("approve"),
        json!("done"),
        json!("broken"),
        json!("nowhere"),
        json!("build/1"),
        json!("running"),
        json!("waiting"),
        json!("needs-intervention"),
        json!("completed"),
        json!("failed"),
        json!("scheduled"),
        json!("unknown"),
        json!("pure"),
        json!("reconcilable"),
        json!([]),
        json!(["input"]),
        json!({}),
        json!({"$get": ["nodes", "approve", "output", 0]}),
        json!({"output": 1}),
        json!({"error": {"code": "x", "message": "m"}}),
    ]
}

/// Every document obtained by replacing or removing one member of `doc`.
fn mutations(doc: &Value) -> Vec<Value> {
    let mut paths = Vec::new();
    pointers(doc, "", &mut paths);
    let mut out = Vec::new();
    for path in &paths {
        for replacement in replacements() {
            let mut mutated = doc.clone();
            *mutated.pointer_mut(path).expect("path exists") = replacement;
            out.push(mutated);
        }
        if let Some((parent, key)) = path.rsplit_once('/') {
            let mut mutated = doc.clone();
            if let Some(map) = mutated.pointer_mut(parent).and_then(Value::as_object_mut) {
                map.remove(key);
                out.push(mutated);
            }
        }
    }
    out
}

/// Runs a request and checks that whatever comes back is well formed.
fn survive(request: &TransitionRequest) -> bool {
    match transition(request) {
        Ok(decision) => {
            assert!(canonical::parse_canonical(&decision.snapshot).is_ok());
            assert_eq!(decision.snapshot_digest, digest::state(&decision.snapshot));
            for command in &decision.commands {
                assert!(canonical::parse_canonical(&command.payload).is_ok());
            }
            for diagnostic in &decision.diagnostics {
                assert!(canonical::parse_canonical(&diagnostic.details).is_ok());
            }
            true
        }
        Err(failure) => {
            assert!(!failure.code.is_empty());
            false
        }
    }
}

#[test]
fn the_corpus_itself_is_accepted() {
    let corpus = corpus();
    assert!(corpus.len() > 80);
    for request in &corpus {
        assert!(survive(request), "{}", request.input_event.kind);
    }
}

#[test]
fn truncated_documents_are_failures() {
    let corpus = corpus();
    for request in corpus.iter().take(2) {
        for cut in 0..request.graph.len() {
            let mut mutated = request.clone();
            mutated.graph.truncate(cut);
            assert!(!survive(&mutated));
        }
    }
    for request in &corpus {
        for cut in 0..request.input_event.payload.len() {
            let mut mutated = request.clone();
            mutated.input_event.payload.truncate(cut);
            assert!(!survive(&mutated));
        }
    }
    for request in corpus.iter().step_by(3) {
        let Some(snapshot) = &request.snapshot else {
            continue;
        };
        for cut in 0..snapshot.len() {
            let mut mutated = request.clone();
            mutated.snapshot = Some(snapshot[..cut].to_vec());
            assert!(!survive(&mutated));
        }
    }
}

#[test]
fn corrupted_bytes_never_panic() {
    let corpus = corpus();
    let bytes = [b'0', b'"', b'}', b'x', 0xff];
    for request in corpus.iter().take(2) {
        for byte in bytes {
            for index in 0..request.graph.len() {
                let mut mutated = request.clone();
                mutated.graph[index] = byte;
                survive(&mutated);
            }
        }
    }
    for request in corpus.iter().step_by(7) {
        for byte in bytes {
            for index in 0..request.input_event.payload.len() {
                let mut mutated = request.clone();
                mutated.input_event.payload[index] = byte;
                survive(&mutated);
            }
            let Some(snapshot) = &request.snapshot else {
                continue;
            };
            for index in 0..snapshot.len() {
                let mut mutated = request.clone();
                let mut corrupted = snapshot.clone();
                corrupted[index] = byte;
                mutated.snapshot = Some(corrupted);
                survive(&mutated);
            }
        }
    }
}

#[test]
fn restructured_snapshots_never_panic() {
    for request in corpus().iter().step_by(3) {
        let Some(snapshot) = &request.snapshot else {
            continue;
        };
        for mutated_state in mutations(&parse(snapshot)) {
            let mut mutated = request.clone();
            mutated.snapshot = Some(canon(&mutated_state));
            survive(&mutated);
        }
    }
}

#[test]
fn restructured_payloads_never_panic() {
    for request in corpus() {
        for payload in mutations(&parse(&request.input_event.payload)) {
            let mut mutated = request.clone();
            mutated.input_event.payload = canon(&payload);
            survive(&mutated);
        }
    }
}

#[test]
fn restructured_workflows_never_panic() {
    let corpus = corpus();
    let graphs = mutations(&parse(&corpus[0].graph));
    for request in corpus.iter().step_by(5) {
        for graph in &graphs {
            let mut mutated = request.clone();
            mutated.graph = canon(graph);
            survive(&mutated);
        }
    }
}

#[test]
fn extreme_envelopes_and_limits_never_panic() {
    for request in corpus() {
        for sequence in [0, 1, 2, u64::MAX - 1, u64::MAX, 1 << 63, (1 << 63) - 1] {
            for at in [0, u64::MAX, 1 << 63, (1 << 63) - 1] {
                for limit in [0, 1, u32::MAX] {
                    let mut mutated = request.clone();
                    mutated.input_event.sequence = sequence;
                    mutated.input_event.accepted_at_ms = at;
                    mutated.frozen_limits.microsteps = limit;
                    mutated.frozen_limits.expression_operations = limit;
                    mutated.frozen_limits.max_state_bytes = limit;
                    mutated.frozen_limits.max_command_count = limit;
                    survive(&mutated);
                }
            }
        }
    }
}

#[test]
fn hostile_documents_are_failures() {
    let deep = [vec![b'['; 100_000], vec![b']'; 100_000]].concat();
    let deep_object = format!("{}1{}", "{\"a\":".repeat(50_000), "}".repeat(50_000)).into_bytes();
    let hostile: Vec<Vec<u8>> = vec![
        deep,
        deep_object,
        vec![0xff, 0xfe, 0xfd],
        vec![0x00],
        b"\"\\ud800\"".to_vec(),
        b"1e999999".to_vec(),
        b"-".to_vec(),
        b"123456789012345678901234567890".to_vec(),
        b"{\"a\":1,\"a\":1}".to_vec(),
        "\u{feff}{}".as_bytes().to_vec(),
    ];
    let corpus = corpus();
    for request in corpus.iter().step_by(9) {
        for document in &hostile {
            let mut mutated = request.clone();
            mutated.graph = document.clone();
            assert!(!survive(&mutated));

            let mut mutated = request.clone();
            mutated.input_event.payload = document.clone();
            assert!(!survive(&mutated));

            if request.snapshot.is_some() {
                let mut mutated = request.clone();
                mutated.snapshot = Some(document.clone());
                assert!(!survive(&mutated));
            }
        }
    }
}
