//! Workflow folders (spec/dot.md) through the compiled binary: a folder runs, and each
//! build problem exits 2 with its code, file and line.

mod common;

use common::{effect_argv, Dir};
use serde_json::json;

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
  "format": "runspore.workflow/0.2", "inputSchema": {{"type": "object", "properties": {{}}}},
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
    case("no dialect", Some(("  dialect = \"runspore/0.2\";\n", "")), None, "dot.no-dialect", D, 2),
    case("dialect 0.1", Some(("runspore/0.2", "runspore/0.1")), None, "dot.old-dialect", D, 3),
    case("unknown dialect", Some(("runspore/0.2", "runspore/9")), None, "dot.unknown-dialect", D, 3),
    case("dialect before syntax", Some(("  dialect = \"runspore/0.2\";", "  subgraph s { a; }")), None, "dot.no-dialect", D, 2),
    case("subgraph", Some(("  node [shape=box];", "  subgraph s { a; }")), None, "dot.unsupported", D, 5),
    case("anonymous subgraph", Some(("  node [shape=box];", "  { a; }")), None, "dot.unsupported", D, 5),
    case("port", Some(("prep -> work", "prep:e -> work")), None, "dot.unsupported", D, 12),
    case("html label", Some(("\"Prep\" + \"are\"", "<<b>Prep</b>>")), None, "dot.unsupported", D, 8),
    case("strict", Some(("digraph \"folder-flow\"", "strict digraph \"folder-flow\"")), None, "dot.unsupported", D, 2),
    case("undirected graph", Some(("digraph \"folder-flow\"", "graph \"folder-flow\"")), None, "dot.unsupported", D, 2),
    case("undirected edge", Some(("work -> broken", "work -- broken")), None, "dot.unsupported", D, 13),
    case("attribute without value", Some(("shape=doublecircle", "bold")), None, "dot.no-value", D, 10),
    case("after the graph", Some(("}\n", "}\ndigraph again {}\n")), None, "dot.after-graph", D, 17),
    case("misspelt attribute", Some(("shape=doublecircle", "colour=red")), None, "dot.attribute-typo", D, 10),
    case("unknown attribute", Some(("shape=doublecircle", "priority=1")), None, "dot.unknown-attribute", D, 10),
    case("kind on an edge", Some(("style=dashed", "kind=\"fail\"")), None, "dot.misplaced-attribute", D, 15),
    case("on in defaults", Some(("node [shape=box]", "edge [on=\"x\"]")), None, "dot.misplaced-attribute", D, 5),
    case("bad kind", Some(("broken [kind=\"fail\"]", "broken [kind=\"end\"]")), None, "dot.unknown-kind", D, 11),
    case("two kinds", Some(("  approve [kind=\"await-signal\"];", "  approve [kind=\"await-signal\"];\n  approve [kind=\"activity\"];")), None, "dot.kind-conflict", D, 10),
    case("list in on", Some(("on=\"approved\"", "on=\"approved,ok\"")), None, "dot.bad-outcome", D, 14),
    case("twice in a list", Some(("on=\"rejected\", style=dashed", "on=\"rejected\", on=\"no\"")), None, "dot.repeated-attribute", D, 15),
    case("no graph id", Some(("digraph \"folder-flow\"", "digraph")), None, "dot.no-name", D, 2),
    case("bad node id", Some(("prep -> work ->", "prep -> \"wo rk\" ->")), None, "dot.bad-step-name", D, 12),
    case("edge from an end", Some(("work -> broken [on=\"failed\"]", "done -> broken")), None, "dot.arrow-from-end", D, 13),
    case("same outcome twice", Some(("on=\"rejected\", style=dashed", "on=\"approved\"")), None, "dot.repeated-arrow", D, 15),
    case("no start", Some(("start=\"prep\", ", "")), None, "dot.no-start", D, 2),
    case("start not a node", Some(("start=\"prep\"", "start=\"nope\"")), None, "dot.unknown-start", D, 4),
    case("declared outcome without edge", None, Some(("\"effect\": \"idempotent\"", "\"effect\": \"idempotent\", \"outcomes\": [\"ok\", \"red\"]")), "dot.missing-outcome-arrow", D, 8),
    case("edge for an undeclared outcome", Some(("on=\"failed\"", "on=\"red\"")), None, "dot.undeclared-outcome", D, 13),
    case("format disagrees", None, Some(("runspore.workflow/0.2", "runspore.workflow/0.1")), "settings.wrong-format", S, 2),
    case("settings not json", None, Some(("4},", "4},,")), "settings.not-json", S, 3),
    case("start in settings", None, Some(("4},", "4}, \"start\": \"prep\",")), "settings.from-dot", S, 3),
    case("field not allowed", None, Some(("\"approval\",", "\"approval\", \"timeout\": 5,")), "settings.unknown-key", S, 11),
    case("field of another kind", None, Some(("\"approval\",", "\"approval\", \"input\": {},")), "node.field-not-allowed", S, 11),
    case("outcomes in settings", None, Some(("{\"action\": \"step\"}", "{\"action\": \"step\", \"outcomes\": {}}")), "node.from-dot", S, 9),
    case("unknown action", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"nope\"}")), "node.unknown-action", S, 9),
    case("node not in the dot", None, Some(("\"prep\": {\"action\": \"step\"},", "\"prep\": {\"action\": \"step\"}, \"ghost\": {},")), "settings.unknown-step", S, 9),
    case("node not in settings", None, Some(("    \"prep\": {\"action\": \"step\"},\n", "")), "node.missing-field", D, 8),
    case("reference escapes", None, Some(("steps/work.md", "../outside.md")), "file.outside", S, 10),
    case("reference absolute", None, Some(("steps/work.md", "/etc/hosts")), "file.absolute", S, 10),
    case("reference missing", None, Some(("steps/work.md", "steps/nope.md")), "file.missing", S, 10),
    case("symlink escapes", None, Some(("steps/work.md", "steps/link.md")), "file.outside", S, 10),
    case("asset escapes", None, Some(("assets/tool.sh", "../outside.md")), "file.outside", S, 6),
    case("schema outside the subset", None, Some(("{\"type\": \"object\"}", "{\"type\": \"number\"}")), "schema.outside-subset", S, 11),
    case("schema file missing", None, Some(("{\"type\": \"object\"}", "\"schemas/nope.json\"")), "file.missing", S, 11),
    case("schema file not json", None, Some(("{\"type\": \"object\"}", "\"steps/work.md\"")), "schema.file-not-json", S, 11),
    case("schema with a remote ref", None, Some(("{\"type\": \"object\"}", "{\"$ref\": \"other.json\"}")), "schema.outside-subset", S, 11),
    case("schemas not an object", None, Some(("\"outputSchemas\": {\"approved\": {\"type\": \"object\"}}", "\"outputSchemas\": []")), "settings.wrong-type", S, 11),
    case("input schema root not an object", None, Some(("\"inputSchema\": {\"type\": \"object\", \"properties\": {}}", "\"inputSchema\": {\"type\": \"string\"}")), "schema.input-not-object", S, 2),
    case("input schema outside the subset", None, Some(("\"inputSchema\": {\"type\": \"object\", \"properties\": {}}", "\"inputSchema\": {\"type\": \"object\", \"pattern\": \"x\"}")), "schema.input-outside-subset", S, 2),
    case("input schema not a schema", None, Some(("\"inputSchema\": {\"type\": \"object\", \"properties\": {}}", "\"inputSchema\": 5")), "settings.wrong-type", S, 2),
    case("input schema file missing", None, Some(("\"inputSchema\": {\"type\": \"object\", \"properties\": {}}", "\"inputSchema\": \"schemas/nope.json\"")), "file.missing", S, 2),
    case("input schema file not json", None, Some(("\"inputSchema\": {\"type\": \"object\", \"properties\": {}}", "\"inputSchema\": \"steps/work.md\"")), "schema.input-file-not-json", S, 2),
    case("marker without schemas", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"step\", \"input\": {\"$outputSchemas\": {}}}")), "input.marker-no-schemas", S, 9),
    case("marker not alone", None, Some(("\"prep\": {\"action\": \"step\"}", "\"prep\": {\"action\": \"step\", \"input\": {\"$outputSchemas\": {}, \"x\": 1}}")), "input.marker-shape", S, 9),
    case("dotted escape", None, Some(("steps/work.md", "steps/../../outside.md")), "file.outside", S, 10),
    case("backslash", None, Some(("steps/work.md", "steps\\\\work.md")), "file.backslash", S, 10),
    case("names a fixed file", None, Some(("steps/work.md", "./settings.json")), "file.fixed-name", S, 10),
    case("probe escapes", None, Some(("[\"assets/tool.sh\"]", "[\"assets/tool.sh\"], \"async\": {\"probe\": {\"command\": {\"argv\": [\"./../outside.md\"]}}}")), "file.outside", S, 6),
    case("visits out of range", None, Some(("\"maxVisitsPerNode\": 4", "\"maxVisitsPerNode\": 0")), "settings.too-small", S, 3),
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
        let report = out.json();
        assert_eq!(
            (out.code(), report["valid"].clone()),
            (Some(2), json!(false)),
            "{}: {report}",
            c.name
        );
        let file = format!("flow/{}", c.file);
        let found = report["diagnostics"].as_array().unwrap().iter().any(|d| {
            d["code"] == c.code && d["file"] == file && d["range"]["start"]["line"] == c.line
        });
        assert!(
            found,
            "{}: no {} at {file}:{}: {report}",
            c.name, c.code, c.line
        );
        // The build words every problem itself: nothing of the kernel shows.
        let text = report.to_string();
        assert!(
            !text.contains("kernel") && !text.contains(" W0") && !text.contains(" W1"),
            "{}: {text}",
            c.name
        );
    }
}

#[test]
fn every_problem_of_a_folder_shows_at_once() {
    // A missing start, a misspelt action and an unknown key: three problems, one run.
    let dot = DOT.replace("start=\"prep\", ", "");
    let settings = settings()
        .replace(
            "\"prep\": {\"action\": \"step\"}",
            "\"prep\": {\"action\": \"stpe\"}",
        )
        .replace("\"approval\",", "\"approval\", \"timeout\": 5,");
    let dir = folder("folder-all", &dot, &settings);
    let out = dir.run(&["validate", "flow"]);
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    for wanted in [
        "flow/workflow.dot:2  dot.no-start",
        "flow/settings.json:9  node.unknown-action",
        "flow/settings.json:11  settings.unknown-key",
        "3 problems in folder-flow. Nothing was run.",
    ] {
        assert!(
            out.stderr.contains(wanted),
            "no {wanted:?} in\n{}",
            out.stderr
        );
    }
    assert!(
        !dir.file("runspore.db").exists(),
        "validate created a database"
    );

    // `start` refuses the same folder with the same report, and creates no run.
    let start = dir.run(&["start", "flow"]);
    assert_eq!(start.code(), Some(2));
    assert!(
        start.stderr.contains("3 problems in folder-flow."),
        "{}",
        start.stderr
    );
    let json = dir.run(&["--json", "start", "flow"]).json();
    assert_eq!(json["error"]["code"], "workflow.invalid", "{json}");
    assert_eq!(json["error"]["details"]["problems"], "3", "{json}");
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
