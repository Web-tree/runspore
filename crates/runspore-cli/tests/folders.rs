//! Workflow folders (spec/dot.md) through the compiled binary: a folder runs, and each
//! build error exits 2 naming its file and line.

mod common;

use common::{effect_argv, Dir};
use serde_json::{json, Value};

/// The heartbeat renews the lease well before it runs out on a busy machine.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

const DOT: &str = r#"// Two steps, a decision, two ends.
digraph "folder-flow" {
  dialect = "runspore/0.2";
  graph [start="prep", rankdir=LR];
  node [shape=box];
  /* a block
     comment */
  prep [label="Prep" + "are"];
  approve [kind="await-signal"];
  done [kind=complete, shape=doublecircle];
  broken [kind="fail"];
  prep -> work -> approve;
  work -> broken [on="failed"];
  approve -> done [on="approved"];
  approve -> broken [on="rejected", style=dashed];
}
"#;

fn settings() -> String {
    let argv = effect_argv("step", "true");
    format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "limits": {{"maxVisitsPerNode": 4}},
  "actions": {{
    "step": {{"kind": "command", "effect": "idempotent", "argv": {argv},
             "assets": ["assets/tool.sh"]}}
  }},
  "nodes": {{
    "prep": {{"action": "step"}},
    "work": {{"action": "step", "instructions": "steps/work.md"}},
    "approve": {{"signal": "approval", "outputSchemas": {{"approved": {{"type": "object"}}}}}},
    "done": {{"output": {{"$get": ["nodes", "approve", "outcome"]}}}},
    "broken": {{"error": {{"code": "rejected", "message": "rejected"}}}}
  }}
}}
"#
    )
}

/// A scratch directory holding the folder `flow` and a file `outside.md` beside it.
fn folder(name: &str, dot: &str, settings: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(flow.join("steps")).unwrap();
    std::fs::create_dir_all(flow.join("assets")).unwrap();
    std::fs::write(flow.join("workflow.dot"), dot).unwrap();
    std::fs::write(flow.join("settings.json"), settings).unwrap();
    std::fs::write(flow.join("steps/work.md"), "Do the work.\n").unwrap();
    std::fs::write(flow.join("assets/tool.sh"), "#!/bin/sh\n").unwrap();
    std::fs::write(flow.join("stray.md"), "never read\n").unwrap();
    std::fs::write(dir.file("outside.md"), "outside\n").unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

#[test]
fn a_folder_validates_starts_and_runs() {
    let dir = folder("folder-runs", DOT, &settings());
    for entry in ["flow", "flow/workflow.dot"] {
        let out = dir.run(&["--json", "validate", entry]);
        assert_eq!(
            (out.code(), out.json()["valid"].clone()),
            (Some(0), json!(true)),
            "{entry}: {}",
            out.stdout
        );
    }
    assert!(
        !dir.file("runspore.db").exists(),
        "validate created a database"
    );

    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    let status = parked.json();
    assert_eq!(
        (parked.code(), status["position"]["nodeId"].clone()),
        (Some(3), json!("approve")),
        "{status}"
    );
    let steps: Vec<String> = dir.effects().into_iter().map(|e| e.node).collect();
    assert_eq!(steps, ["step", "step"]);
    let again = dir.run(&args(&[
        "--json",
        "start",
        "flow/workflow.dot",
        "--key",
        "k",
    ]));
    assert_eq!(
        again.json()["created"],
        false,
        "the same folder is the same package"
    );

    let run = status["runId"].as_str().unwrap();
    // `approved` declares an object schema, so a signal without data is refused.
    let signal = ["signal", run, "approval", "--outcome", "approved"];
    assert_eq!(dir.run(&args(&signal)).code(), Some(2));
    let signal = [&signal[..], &["--data", "{}"]].concat();
    assert_eq!(dir.run(&args(&signal)).code(), Some(0));
    let done = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(
        (done.code(), done.json()["result"]["output"].clone()),
        (Some(0), json!("approved"))
    );
}

/// One error case: edits to the base files, then the code, file and line expected.
struct Case {
    name: &'static str,
    dot: Option<(&'static str, &'static str)>,
    settings: Option<(&'static str, &'static str)>,
    code: &'static str,
    file: &'static str,
    line: usize,
}

const fn case(
    name: &'static str,
    dot: Option<(&'static str, &'static str)>,
    settings: Option<(&'static str, &'static str)>,
    code: &'static str,
    file: &'static str,
    line: usize,
) -> Case {
    Case {
        name,
        dot,
        settings,
        code,
        file,
        line,
    }
}

const D: &str = "workflow.dot";
const S: &str = "settings.json";

#[rustfmt::skip]
const CASES: &[Case] = &[
    case("no dialect", Some(("  dialect = \"runspore/0.2\";\n", "")), None, "dot.dialect", D, 2),
    case("dialect 0.1", Some(("runspore/0.2", "runspore/0.1")), None, "dot.dialect", D, 3),
    case("unknown dialect", Some(("runspore/0.2", "runspore/9")), None, "dot.dialect", D, 3),
    case("dialect before syntax", Some(("  dialect = \"runspore/0.2\";", "  subgraph s { a; }")), None, "dot.dialect", D, 2),
    case("subgraph", Some(("  node [shape=box];", "  subgraph s { a; }")), None, "dot.syntax", D, 5),
    case("anonymous subgraph", Some(("  node [shape=box];", "  { a; }")), None, "dot.syntax", D, 5),
    case("port", Some(("prep -> work", "prep:e -> work")), None, "dot.syntax", D, 12),
    case("html label", Some(("\"Prep\" + \"are\"", "<<b>Prep</b>>")), None, "dot.syntax", D, 8),
    case("strict", Some(("digraph \"folder-flow\"", "strict digraph \"folder-flow\"")), None, "dot.syntax", D, 2),
    case("undirected graph", Some(("digraph \"folder-flow\"", "graph \"folder-flow\"")), None, "dot.syntax", D, 2),
    case("undirected edge", Some(("work -> broken", "work -- broken")), None, "dot.syntax", D, 13),
    case("attribute without value", Some(("shape=doublecircle", "bold")), None, "dot.syntax", D, 10),
    case("after the graph", Some(("}\n", "}\ndigraph again {}\n")), None, "dot.syntax", D, 17),
    case("unknown attribute", Some(("shape=doublecircle", "colour=red")), None, "dot.attribute", D, 10),
    case("kind on an edge", Some(("style=dashed", "kind=\"fail\"")), None, "dot.attribute", D, 15),
    case("on in defaults", Some(("node [shape=box]", "edge [on=\"x\"]")), None, "dot.attribute", D, 5),
    case("bad kind", Some(("broken [kind=\"fail\"]", "broken [kind=\"end\"]")), None, "dot.attribute", D, 11),
    case("two kinds", Some(("  approve [kind=\"await-signal\"];", "  approve [kind=\"await-signal\"];\n  approve [kind=\"activity\"];")), None, "dot.attribute", D, 10),
    case("list in on", Some(("on=\"approved\"", "on=\"approved,ok\"")), None, "dot.attribute", D, 14),
    case("twice in a list", Some(("on=\"rejected\", style=dashed", "on=\"rejected\", on=\"no\"")), None, "dot.attribute", D, 15),
    case("no graph id", Some(("digraph \"folder-flow\"", "digraph")), None, "dot.graph", D, 2),
    case("bad node id", Some(("prep -> work ->", "prep -> \"wo rk\" ->")), None, "dot.graph", D, 12),
    case("edge from an end", Some(("work -> broken [on=\"failed\"]", "done -> broken")), None, "dot.graph", D, 13),
    case("same outcome twice", Some(("on=\"rejected\", style=dashed", "on=\"approved\"")), None, "dot.graph", D, 15),
    case("no start", Some(("start=\"prep\", ", "")), None, "dot.graph", D, 2),
    case("start not a node", Some(("start=\"prep\"", "start=\"nope\"")), None, "dot.graph", D, 4),
    case("declared outcome without edge", None, Some(("\"effect\": \"idempotent\"", "\"effect\": \"idempotent\", \"outcomes\": [\"ok\", \"red\"]")), "dot.graph", D, 8),
    case("edge for an undeclared outcome", Some(("on=\"failed\"", "on=\"red\"")), None, "dot.graph", D, 13),
    case("format disagrees", None, Some(("runspore.workflow/0.2", "runspore.workflow/0.1")), "dot.dialect", S, 2),
    case("settings not json", None, Some(("4},", "4},,")), "settings.invalid", S, 3),
    case("start in settings", None, Some(("4},", "4}, \"start\": \"prep\",")), "settings.invalid", S, 3),
    case("field not allowed", None, Some(("\"approval\",", "\"approval\", \"timeout\": 5,")), "settings.invalid", S, 11),
    case("outcomes in settings", None, Some(("{\"action\": \"step\"}", "{\"action\": \"step\", \"outcomes\": {}}")), "settings.invalid", S, 9),
    case("unknown action", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"nope\"}")), "settings.invalid", S, 9),
    case("node not in the dot", None, Some(("\"prep\": {\"action\": \"step\"},", "\"prep\": {\"action\": \"step\"}, \"ghost\": {},")), "settings.mismatch", S, 9),
    case("node not in settings", None, Some(("    \"prep\": {\"action\": \"step\"},\n", "")), "settings.mismatch", D, 8),
    case("reference escapes", None, Some(("steps/work.md", "../outside.md")), "settings.reference", S, 10),
    case("reference absolute", None, Some(("steps/work.md", "/etc/hosts")), "settings.reference", S, 10),
    case("reference missing", None, Some(("steps/work.md", "steps/nope.md")), "settings.reference", S, 10),
    case("symlink escapes", None, Some(("steps/work.md", "steps/link.md")), "settings.reference", S, 10),
    case("asset escapes", None, Some(("assets/tool.sh", "../outside.md")), "settings.reference", S, 6),
    case("schema outside the subset", None, Some(("{\"type\": \"object\"}", "{\"type\": \"number\"}")), "workflow.invalid-schema", S, 11),
    case("schema file missing", None, Some(("{\"type\": \"object\"}", "\"schemas/nope.json\"")), "settings.reference", S, 11),
    case("schema file not json", None, Some(("{\"type\": \"object\"}", "\"steps/work.md\"")), "workflow.invalid-schema", S, 11),
    case("schema with a remote ref", None, Some(("{\"type\": \"object\"}", "{\"$ref\": \"other.json\"}")), "workflow.invalid-schema", S, 11),
    case("schemas not an object", None, Some(("\"outputSchemas\": {\"approved\": {\"type\": \"object\"}}", "\"outputSchemas\": []")), "settings.invalid", S, 11),
    case("marker without schemas", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"step\", \"input\": {\"$outputSchemas\": {}}}")), "settings.invalid", S, 9),
    case("marker not alone", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"step\", \"input\": {\"$outputSchemas\": {}, \"x\": 1}}")), "settings.invalid", S, 9),
];

#[test]
fn each_build_error_names_its_file_and_line() {
    let base = settings();
    for c in CASES {
        let edit = |text: &str, e: Option<(&str, &str)>| match e {
            Some((from, to)) => {
                assert!(text.contains(from), "{}: no `{from}` to edit", c.name);
                text.replacen(from, to, 1)
            }
            None => text.to_string(),
        };
        let dir = folder("folder-error", &edit(DOT, c.dot), &edit(&base, c.settings));
        std::os::unix::fs::symlink("../../outside.md", dir.file("flow/steps/link.md")).unwrap();
        let out = dir.run(&["--json", "validate", "flow"]);
        let error = out.json()["error"].clone();
        let at = format!("flow/{}:{}: ", c.file, c.line);
        assert_eq!(
            (out.code(), error["code"].as_str()),
            (Some(2), Some(c.code)),
            "{}: {error}",
            c.name
        );
        let message = error["message"].as_str().unwrap();
        assert!(
            message.starts_with(&at),
            "{}: wanted `{at}…`, got {message}",
            c.name
        );
    }
}

#[test]
fn errors_name_the_fix_and_both_files() {
    let check = |name, dot: &str, settings: &str, wanted: &[&str]| {
        let dir = folder(name, dot, settings);
        let out = dir.run(&["validate", "flow"]);
        assert_eq!(out.code(), Some(2), "{name}");
        for w in wanted {
            assert!(out.stderr.contains(w), "{name}: no `{w}` in {}", out.stderr);
        }
    };
    let base = settings();
    check(
        "fix-dialect",
        &DOT.replace("runspore/0.2", "runspore/0.1"),
        &base,
        &[
            "spore: dot.dialect: flow/workflow.dot:3: ",
            "runspore/0.2",
            "runspore.workflow/0.2",
        ],
    );
    check(
        "both-files-format",
        DOT,
        &base.replace("workflow/0.2", "workflow/0.1"),
        &["flow/settings.json:2: ", "flow/workflow.dot:3"],
    );
    check(
        "both-files-node",
        DOT,
        &base.replace("    \"prep\": {\"action\": \"step\"},\n", ""),
        &["flow/workflow.dot:8: ", "flow/settings.json"],
    );
}

#[test]
fn entries_other_than_a_folder_or_its_dot_file() {
    let dir = folder("folder-entry", DOT, &settings());
    std::fs::copy(dir.file("flow/workflow.dot"), dir.file("other.dot")).unwrap();
    let wrong_name = dir.run(&["--json", "validate", "other.dot"]);
    assert_eq!(
        (
            wrong_name.code(),
            wrong_name.json()["error"]["code"].clone()
        ),
        (Some(2), json!("workflow.folder"))
    );

    std::fs::remove_file(dir.file("flow/settings.json")).unwrap();
    let missing = dir.run(&["--json", "start", "flow"]);
    let error = missing.json()["error"].clone();
    assert_eq!(
        (missing.code(), error["code"].clone()),
        (Some(2), json!("workflow.folder"))
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("flow/settings.json: "),
        "{error}"
    );
}

#[test]
fn a_kernel_rejection_of_a_built_folder_names_the_folder() {
    let settings = settings().replace("\"maxVisitsPerNode\": 4", "\"maxVisitsPerNode\": 0");
    let dir = folder("folder-kernel", DOT, &settings);
    let out = dir.run(&["--json", "--native-kernel", "validate", "flow"]);
    let error: Value = out.json()["error"].clone();
    assert_eq!(
        (out.code(), error["code"].clone()),
        (Some(2), json!("workflow.invalid"))
    );
    assert!(
        error["message"].as_str().unwrap().starts_with("flow: "),
        "{error}"
    );
}
