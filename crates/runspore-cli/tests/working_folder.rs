//! H20 through the compiled binary (spec/host.md §3): a step's `cwd` taken from its
//! input, `{"fromInput": "<JSON Pointer>"}`, at a folder an earlier step made.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{run, Dir, Out, BIN};
use serde_json::{json, Value};

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

const DOT: &str = r#"digraph "worktree" {
  dialect = "runspore/0.2";
  start = "make";
  done [kind=complete];
  make -> work -> done;
}
"#;

/// `spore` in `cwd` with no `RUNSPORE_DB`, so it finds its database itself.
fn spore(cwd: &Path, args: &[&str]) -> Out {
    let mut cmd = Command::new(BIN);
    cmd.current_dir(cwd)
        .env_remove("RUNSPORE_DB")
        .env_remove("RUNSPORE_FAILPOINTS")
        .args(FAST)
        .args(args)
        .stdin(Stdio::null());
    run(cmd)
}

/// A repository with a `.spore/` and the `worktree` workflow: `make` runs `made`, a
/// shell script whose stdout is its JSON output; `work` runs in the folder at
/// `/place/dir` of its input, which maps `make`'s output to `place`. `work` logs the
/// folder it runs in to `pwd.log` and what `spore list` shows from there to
/// `list.json`, both outside the repository.
fn repo(name: &str, made: &str) -> (Dir, PathBuf) {
    let dir = Dir::new(name);
    let root = dir.path.join("repo");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let root = root.canonicalize().unwrap();
    let out = spore(&root, &["init"]);
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    let flow = root.join(".spore/workflows/worktree");
    std::fs::create_dir_all(&flow).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    let work = format!(
        "pwd -P > '{}' && '{BIN}' --json list > '{}'",
        dir.file("pwd.log").display(),
        dir.file("list.json").display()
    );
    let settings = json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "make": {"kind": "command", "effect": "idempotent", "output": "json",
                     "argv": ["/bin/sh", "-c", made]},
            "work": {"kind": "command", "effect": "idempotent", "output": "none",
                     "argv": ["/bin/sh", "-c", work],
                     "cwd": {"fromInput": "/place/dir"},
                     "retry": {"maxAttempts": 3, "backoffMs": "10"}}
        },
        "nodes": {
            "make": {"action": "make"},
            "work": {"action": "work",
                     "input": {"place": {"$get": ["nodes", "make", "output"]}}},
            "done": {"output": {"$get": ["nodes", "work", "outcome"]}}
        }
    });
    std::fs::write(
        flow.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    (dir, root)
}

/// `make` creates `wt` with a project folder of its own, as a git worktree has.
const MAKE_WT: &str = "mkdir -p wt/.spore/workflows && ";

#[test]
fn a_step_runs_in_the_folder_an_earlier_step_returned() {
    let absolute = format!("{MAKE_WT}printf '{{\"dir\": \"%s\"}}' \"$(pwd -P)/wt\"");
    let relative = format!("{MAKE_WT}printf '{{\"dir\": \"wt\"}}'");
    for (name, made) in [("absolute", absolute), ("relative", relative)] {
        let (dir, root) = repo(&format!("cwd-{name}"), &made);
        let out = spore(&root, &["--json", "run", "worktree"]);
        assert_eq!(out.code(), Some(0), "{name}: {}{}", out.stdout, out.stderr);
        let status = out.json();
        assert_eq!(status["status"], json!("completed"), "{name}");

        let wt = root.join("wt");
        let pwd = std::fs::read_to_string(dir.file("pwd.log")).unwrap();
        assert_eq!(pwd.trim_end(), wt.to_str().unwrap(), "{name}");
        // From the worktree, with its own `.spore/`, `spore` reached the run's database.
        let listed: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.file("list.json")).unwrap()).unwrap();
        assert_eq!(
            listed["db"],
            json!(root.join(".spore/runspore.db")),
            "{name}"
        );
        assert_eq!(listed["runs"][0]["runId"], status["runId"], "{name}");
        assert!(!wt.join(".spore/runspore.db").exists(), "{name}");
    }
}

#[test]
fn a_cwd_from_input_that_names_no_folder_leaves_the_step_stuck() {
    let cases = [
        (
            "missing",
            "printf '{}'".to_string(),
            json!({"pointer": "/place/dir"}),
        ),
        (
            "not-a-string",
            "printf '{\"dir\": 7}'".to_string(),
            json!({"pointer": "/place/dir"}),
        ),
        (
            "no-folder",
            "printf '{\"dir\": \"nowhere\"}'".to_string(),
            json!({"pointer": "/place/dir", "cwd": "nowhere"}),
        ),
    ];
    for (name, made, details) in cases {
        let (dir, root) = repo(&format!("cwd-{name}"), &made);
        let out = spore(&root, &["--json", "run", "worktree"]);
        assert_eq!(out.code(), Some(4), "{name}: {}{}", out.stdout, out.stderr);
        let status = out.json();
        assert_eq!(status["status"], json!("needs-intervention"), "{name}");
        let invocation = &status["invocation"];
        assert_eq!(invocation["state"], json!("stuck"), "{name}");
        assert_eq!(invocation["attempt"], json!(1), "{name}: not retried");
        assert_eq!(invocation["error"]["code"], json!("command.cwd-invalid"));
        assert_eq!(invocation["error"]["nodeId"], json!("work"), "{name}");
        assert_eq!(invocation["error"]["details"], details, "{name}");
        assert!(!dir.file("pwd.log").exists(), "{name}: the step ran");
    }
}

#[test]
fn validate_refuses_a_malformed_cwd_object() {
    let (_dir, root) = repo("cwd-malformed", "printf '{}'");
    let settings = root.join(".spore/workflows/worktree/settings.json");
    let text = std::fs::read_to_string(&settings).unwrap();
    std::fs::write(&settings, text.replace("\"/place/dir\"", "\"place/dir\"")).unwrap();
    let out = spore(&root, &["--json", "validate", "worktree"]);
    assert_eq!(out.code(), Some(2), "{}{}", out.stdout, out.stderr);
    let report = out.json();
    let codes: Vec<&str> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["severity"] == json!("problem"))
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["command.cwd-not-pointer"], "{report}");
    assert_eq!(
        report["diagnostics"][0]["path"],
        json!("/actions/work/cwd/fromInput"),
        "{report}"
    );
}
