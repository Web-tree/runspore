//! `onIntervention` through the compiled binary (spec/host.md §2, Telling a person;
//! spec/cli.md): a run that gets stuck runs the workflow's command once with what
//! happened on stdin and its params in `argv`; a command that fails shows in `events`.

mod common;

use common::Dir;
use serde_json::{json, Value};

const FAST: [&str; 5] = ["--native-kernel", "--poll-ms", "20", "--lease-ms", "500"];

const DOT: &str = r#"digraph "told" {
  dialect = "runspore/0.2";
  start = "work";
  done [kind=complete];
  work -> done;
}
"#;

/// `work` always fails; `tell` writes its stdin to `told.json` and its argument and
/// node to `told.args`, then fails while the file `broken` exists.
fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let tell = "cat > told.json; echo \"$1 $RUNSPORE_NODE_ID\" >> told.args; test ! -f broken";
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "work": {"kind": "command", "effect": "idempotent",
                     "argv": ["/bin/sh", "-c", "echo no luck >&2; exit 3"]},
            "tell": {"kind": "command", "effect": "unsafe",
                     "argv": ["/bin/sh", "-c", tell, "tell", {"$get": ["params", "issue"]}]}
        },
        "nodes": {"work": {"action": "work"}},
        "onIntervention": {"action": "tell",
                           "params": {"issue": {"$get": ["input", "issue"]}}}
    });
    std::fs::write(
        flow.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

fn run(dir: &Dir) -> (Option<i32>, Value) {
    let input = r#"{"issue": "acme/app#7"}"#;
    let out = dir.run(&args(&[
        "--json", "run", "flow", "--key", "k", "--input", input,
    ]));
    (out.code(), out.json())
}

fn kinds(events: &Value) -> Vec<&str> {
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn a_stuck_run_runs_on_intervention_once_before_spore_run_exits() {
    let dir = folder("intervention-told");
    let (code, status) = run(&dir);
    assert_eq!(code, Some(4), "{status}");
    let run_id = status["runId"].as_str().unwrap();
    let told: Value =
        serde_json::from_slice(&std::fs::read(dir.file("told.json")).unwrap()).unwrap();
    assert_eq!(told["run"], json!({"id": run_id, "tenant": "default"}));
    assert_eq!(told["node"], "work");
    assert_eq!(told["status"], "needs-intervention");
    assert_eq!(told["reason"], "stuck");
    assert_eq!(told["error"]["code"], "command.exit");
    assert_eq!(told["error"]["details"]["stderrTail"], "no luck\n");
    let args_file = std::fs::read_to_string(dir.file("told.args")).unwrap();
    assert_eq!(args_file, "acme/app#7 work\n");

    // Running it again tells nobody again, and a success leaves no event.
    let (code, _) = run(&dir);
    assert_eq!(code, Some(4));
    let args_file = std::fs::read_to_string(dir.file("told.args")).unwrap();
    assert_eq!(args_file.lines().count(), 1, "{args_file}");
    let events = dir.run(&args(&["--json", "events", run_id])).json();
    assert_eq!(kinds(&events), ["run.started", "activity.result"]);
}

#[test]
fn a_failing_on_intervention_shows_in_events_and_nothing_else() {
    let dir = folder("intervention-broken");
    std::fs::write(dir.file("broken"), "").unwrap();
    let (code, status) = run(&dir);
    assert_eq!(code, Some(4), "{status}");
    assert_eq!(status["invocation"]["state"], "stuck");
    let run_id = status["runId"].as_str().unwrap();
    let events = dir.run(&args(&["--json", "events", run_id])).json();
    assert_eq!(
        kinds(&events),
        ["run.started", "activity.result", "on-intervention.failed"]
    );
    let failed = &events["events"][2];
    assert_eq!(failed["payload"]["action"], "tell");
    assert_eq!(failed["payload"]["status"], "needs-intervention");
    assert_eq!(failed["payload"]["error"]["code"], "command.exit");
    let text = dir.run(&args(&["events", run_id])).stdout;
    assert!(text.contains("on-intervention.failed"), "{text}");
}
