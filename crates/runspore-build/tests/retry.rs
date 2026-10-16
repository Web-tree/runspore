//! An action's `retry` (spec/dot.md §4): one whose effect is repeatable and that sets
//! no `maxAttempts` gets three tries, so a step killed mid-try reruns on its own; an
//! `unsafe` one keeps one try, and a `maxAttempts` the author wrote is kept.

use std::path::PathBuf;

use runspore_build::{check, Machine};
use serde_json::{json, Value};

const DOT: &str = "digraph \"retry\" {\n  dialect = \"runspore/0.2\";\n  start   = \"build\";\n\n  build [label=\"Build\"];\n  done  [kind=\"complete\"];\n\n  build -> done [on=\"ok\"];\n}\n";

/// The package built from a folder whose one action is `action`.
fn package(action: Value) -> Value {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "runspore-retry-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {"build": action},
        "nodes": {"build": {"action": "build"}},
    });
    std::fs::write(dir.join("workflow.dot"), DOT).unwrap();
    std::fs::write(dir.join("settings.json"), settings.to_string()).unwrap();
    let checked = check(&dir, &Machine::examples()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    match checked.built {
        Some(built) => built.package["actions"]["build"].clone(),
        None => panic!("not built: {}", checked.report.short()),
    }
}

fn action(effect: &str) -> Value {
    json!({"kind": "command", "argv": ["make"], "effect": effect})
}

#[test]
fn a_repeatable_action_with_no_retry_gets_three_tries() {
    for effect in ["pure", "read-only", "idempotent"] {
        assert_eq!(
            package(action(effect))["retry"],
            json!({"maxAttempts": 3}),
            "{effect}"
        );
    }
}

#[test]
fn a_repeatable_action_whose_retry_sets_no_max_attempts_gets_three_tries() {
    let mut build = action("idempotent");
    build["retry"] = json!({"backoffMs": "500"});
    assert_eq!(
        package(build)["retry"],
        json!({"maxAttempts": 3, "backoffMs": "500"})
    );
}

#[test]
fn a_max_attempts_the_author_wrote_is_kept() {
    let mut build = action("idempotent");
    build["retry"] = json!({"maxAttempts": 1});
    assert_eq!(package(build)["retry"], json!({"maxAttempts": 1}));
}

#[test]
fn an_unsafe_action_keeps_one_try() {
    assert_eq!(package(action("unsafe")).get("retry"), None);
}
