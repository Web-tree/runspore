//! The project folder (spec/cli.md §Project folder) through the compiled binary:
//! walk-up to one database, `init` and the ignore file, read-only commands that create
//! nothing, workflows by name, the run's base folder, and `RUNSPORE_DB` for steps.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{run, Dir, Out, BIN};
use serde_json::{json, Value};

/// The heartbeat renews the lease well before it runs out on a busy machine. No test
/// here waits for a lease to run out, so the lease is long: a step that starts `spore`
/// itself can stall the worker's heartbeats past a short one under load.
const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "5000",
    "--heartbeat-ms",
    "100",
];

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

/// A scratch repository: a `.git` folder and the subfolders `a/b`, at its real path.
fn repo(name: &str) -> (Dir, PathBuf) {
    let dir = Dir::new(name);
    let root = dir.path.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    (dir, root)
}

/// Writes a workflow folder at `folder` whose graph id is `name`: one command step,
/// `/bin/sh -c <script>`, run from the action's `cwd`, then the end.
fn workflow(folder: &Path, name: &str, script: &str, cwd: Option<&str>) {
    std::fs::create_dir_all(folder).unwrap();
    let dot = format!(
        "digraph \"{name}\" {{\n  dialect = \"runspore/0.2\";\n  graph [start=\"work\"];\n  \
         done [kind=complete];\n  work -> done;\n}}\n"
    );
    std::fs::write(folder.join("workflow.dot"), dot).unwrap();
    let mut action = json!({"kind": "command", "effect": "idempotent",
                            "argv": ["/bin/sh", "-c", script]});
    if let Some(cwd) = cwd {
        action["cwd"] = json!(cwd);
    }
    let settings = json!({"format": "runspore.workflow/0.2",
                          "actions": {"step": action},
                          "nodes": {"work": {"action": "step"}}});
    std::fs::write(
        folder.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Asserts exit 0; the output as JSON, or null when it is text.
fn ok(out: &Out) -> Value {
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
    serde_json::from_str(out.stdout.trim()).unwrap_or(Value::Null)
}

const IGNORED: &str = "runspore.db\nrunspore.db-*\nrunspore.db.files/\n";

#[test]
fn init_creates_the_project_folder_once() {
    let (_dir, root) = repo("init");
    let out = spore(&root.join("a/b"), &["--json", "init"]);
    let created = ok(&out);
    let spore_dir = root.join(".spore");
    assert_eq!(
        created,
        json!({"format": "runspore.cli/0.2", "root": root,
               "created": [spore_dir, spore_dir.join("workflows"), spore_dir.join(".gitignore")]})
    );
    assert!(spore_dir.join("workflows").is_dir());
    assert_eq!(read(&spore_dir.join(".gitignore")), IGNORED);
    assert!(
        !spore_dir.join("runspore.db").exists(),
        "init opened a database"
    );

    let again = ok(&spore(&root, &["--json", "init"]));
    assert_eq!(again["created"], json!([]));
    assert_eq!(read(&spore_dir.join(".gitignore")), IGNORED);

    std::fs::write(spore_dir.join(".gitignore"), "keep-me\nrunspore.db-*").unwrap();
    ok(&spore(&root, &["--json", "init"]));
    assert_eq!(
        read(&spore_dir.join(".gitignore")),
        "keep-me\nrunspore.db-*\nrunspore.db\nrunspore.db.files/\n"
    );
}

#[test]
fn init_refuses_a_nested_or_0_1_layout_and_creates_nothing() {
    let (_dir, root) = repo("init-refuses");
    std::fs::create_dir_all(root.join("a/.spore")).unwrap();
    let nested = spore(&root.join("a/b"), &["--json", "init"]);
    assert_eq!(
        (nested.code(), nested.json()["error"]["code"].clone()),
        (Some(2), json!("project.nested"))
    );
    assert!(!root.join(".spore").exists());
    std::fs::remove_dir(root.join("a/.spore")).unwrap();

    for legacy in ["runspore.db", "runspore.db-wal", "runspore.db.files"] {
        let path = root.join(legacy);
        std::fs::write(&path, "").unwrap();
        let out = spore(&root.join("a"), &["--json", "init"]);
        let error = &out.json()["error"];
        assert_eq!(
            (out.code(), error["code"].clone()),
            (Some(2), json!("project.legacy-database")),
            "{legacy}"
        );
        assert!(error["message"].as_str().unwrap().contains("0.1"));
        assert!(!root.join(".spore").exists(), "{legacy}");
        std::fs::remove_file(&path).unwrap();
    }

    std::fs::create_dir_all(root.join(".spore")).unwrap();
    std::fs::write(root.join(".spore/workflows"), "").unwrap();
    let unusable = spore(&root, &["--json", "init"]);
    assert_eq!(
        (unusable.code(), unusable.json()["error"]["code"].clone()),
        (Some(2), json!("project.unusable"))
    );
    assert!(!root.join(".spore/.gitignore").exists());
}

#[test]
fn every_subfolder_reaches_one_database_and_read_only_commands_create_none() {
    let (_dir, root) = repo("walk-up");
    ok(&spore(&root, &["init"]));
    let db = root.join(".spore/runspore.db");
    for cwd in [root.clone(), root.join("a/b")] {
        let listed = ok(&spore(&cwd, &["--json", "list"]));
        assert_eq!(listed["db"], json!(db));
        assert_eq!(listed["runs"], json!([]));
        for read_only in [&["status", "run_x"][..], &["events", "run_x"]] {
            let out = spore(&cwd, read_only);
            assert_eq!(out.code(), Some(2), "{read_only:?}: {}", out.stderr);
        }
    }
    assert!(!db.exists(), "a read-only command created the database");
    let plain = Dir::new("walk-up-plain");
    let listed = ok(&spore(&plain.path, &["--json", "list"]));
    assert_eq!(listed["runs"], json!([]));
    assert!(!plain.file("runspore.db").exists());

    std::fs::remove_file(root.join(".spore/.gitignore")).unwrap();
    workflow(&root.join("flow"), "flow", "true", None);
    let started = ok(&spore(&root.join("a"), &["--json", "start", "../flow"]));
    let run = started["runId"].as_str().unwrap();
    assert!(db.exists());
    assert_eq!(
        read(&root.join(".spore/.gitignore")),
        IGNORED,
        "a writing command keeps the ignore file"
    );
    for cwd in [root.clone(), root.join("a"), root.join("a/b")] {
        let listed = ok(&spore(&cwd, &["--json", "list"]));
        assert_eq!(listed["db"], json!(db));
        assert_eq!(listed["runs"][0]["runId"], json!(run));
        let status = ok(&spore(&cwd, &["--json", "status", run]));
        assert_eq!(
            (status["db"].clone(), status["runId"].clone()),
            (json!(db), json!(run))
        );
    }
}

#[test]
fn start_finds_a_workflow_by_its_graph_id() {
    let (_dir, root) = repo("by-name");
    let missing = spore(&root, &["--json", "start", "coding-loop"]);
    assert_eq!(
        (missing.code(), missing.json()["error"]["code"].clone()),
        (Some(2), json!("workflow.not-found"))
    );
    ok(&spore(&root, &["init"]));
    let workflows = root.join(".spore/workflows");
    workflow(&workflows.join("loop"), "coding-loop", "true", None);
    workflow(&workflows.join("other"), "review", "true", None);

    let started = ok(&spore(&root.join("a/b"), &["--json", "run", "coding-loop"]));
    assert_eq!(started["status"], json!("completed"));
    ok(&spore(&root.join("a"), &["--json", "validate", "review"]));

    let unknown = spore(&root, &["--json", "start", "loop"]);
    let error = &unknown.json()["error"];
    assert_eq!(
        (unknown.code(), error["code"].clone()),
        (Some(2), json!("workflow.not-found"))
    );
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("coding-loop, review"));

    workflow(&workflows.join("copy"), "review", "true", None);
    let twice = spore(&root, &["--json", "start", "review"]);
    assert_eq!(
        (twice.code(), twice.json()["error"]["code"].clone()),
        (Some(2), json!("workflow.ambiguous"))
    );
}

/// The `where` workflow appends the folder its command runs in to `log`.
fn where_flow(root: &Path, log: &Path) {
    let script = format!("pwd -P >> '{}'", log.display());
    workflow(&root.join(".spore/workflows/where"), "where", &script, None);
}

fn lines(path: &Path) -> Vec<PathBuf> {
    read(path).lines().map(PathBuf::from).collect()
}

#[test]
fn a_run_works_in_the_base_folder_recorded_at_start() {
    let (dir, root) = repo("base-dir");
    ok(&spore(&root, &["init"]));
    let log = dir.file("where.log");
    where_flow(&root, &log);

    // Started from a subfolder, continued from another: both tries run at the root.
    let started = ok(&spore(&root.join("a"), &["--json", "start", "where"]));
    let run = started["runId"].as_str().unwrap();
    let worked = spore(
        &root.join("a/b"),
        &["--json", "worker", "--run", run, "--until-parked"],
    );
    assert_eq!(ok(&worked)["status"], json!("completed"));
    assert_eq!(lines(&log), std::slice::from_ref(&root));

    // `--db` into a `.spore/` from outside the repository uses its parent.
    let outside = Dir::new("base-dir-outside");
    let db = root.join(".spore/runspore.db");
    let flow = root.join(".spore/workflows/where");
    let out = spore(
        &outside.path,
        &[
            "--json",
            "--db",
            db.to_str().unwrap(),
            "run",
            flow.to_str().unwrap(),
        ],
    );
    assert_eq!(ok(&out)["status"], json!("completed"));
    // `--base-dir` overrides it.
    let out = spore(&root, &["--json", "run", "where", "--base-dir", "a/b"]);
    assert_eq!(ok(&out)["status"], json!("completed"));
    let bad = spore(
        &root,
        &["--json", "start", "where", "--base-dir", "nowhere"],
    );
    assert_eq!(bad.code(), Some(2));
    assert_eq!(lines(&log), [root.clone(), root.clone(), root.join("a/b")]);
}

#[test]
fn a_step_reaches_the_run_database_through_runspore_db() {
    let (dir, root) = repo("step-db");
    ok(&spore(&root, &["init"]));
    // The step works in a checkout with its own project folder, as a git worktree does.
    let inner = root.join("a/b");
    std::fs::create_dir_all(inner.join(".spore/workflows")).unwrap();
    let seen = dir.file("seen.json");
    let script = format!("'{BIN}' --json list > '{}'", seen.display());
    workflow(
        &root.join(".spore/workflows/nested"),
        "nested",
        &script,
        Some("a/b"),
    );

    let out = ok(&spore(&root, &["--json", "run", "nested"]));
    assert_eq!(out["status"], json!("completed"));
    let seen: Value = serde_json::from_str(&read(&seen)).unwrap();
    assert_eq!(seen["db"], json!(root.join(".spore/runspore.db")));
    assert_eq!(seen["runs"][0]["runId"], out["runId"]);
    assert!(!inner.join(".spore/runspore.db").exists());
}
