//! The project folder (spec/cli.md §Project folder) through the compiled binary:
//! walk-up to one database, the old place and the overrides, `init`, read-only
//! commands that create nothing, workflows by name, the run's base folder, and
//! `RUNSPORE_DB` for steps.

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

/// A scratch repository at `root`: a `.git` folder and the subfolders `a/b`, at its
/// real path. Its `spore` keeps shared databases in `home`, beside it.
struct Repo {
    dir: Dir,
    root: PathBuf,
    home: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Self {
        let dir = Dir::new(name);
        let root = dir.path.join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        let root = root.canonicalize().unwrap();
        let home = dir.file("home");
        Self { dir, root, home }
    }

    /// `spore` in `cwd` with no `RUNSPORE_DB`, so it finds its database itself.
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.current_dir(cwd)
            .env("SPORE_HOME", &self.home)
            .env_remove("RUNSPORE_DB")
            .env_remove("RUNSPORE_FAILPOINTS")
            .args(FAST)
            .args(args)
            .stdin(Stdio::null());
        cmd
    }

    fn spore(&self, cwd: &Path, args: &[&str]) -> Out {
        run(self.command(cwd, args))
    }
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

/// The database a `--json list` in `cwd` used.
fn db_of(repo: &Repo, cwd: &Path) -> PathBuf {
    let listed = ok(&repo.spore(cwd, &["--json", "list"]));
    PathBuf::from(listed["db"].as_str().unwrap())
}

const IGNORED: &str = "runspore.db\nrunspore.db-*\nrunspore.db.files/\n";

#[test]
fn init_creates_the_project_folder_once() {
    let repo = Repo::new("init");
    let root = &repo.root;
    let out = repo.spore(&root.join("a/b"), &["--json", "init"]);
    let created = ok(&out);
    let spore_dir = root.join(".spore");
    assert_eq!(
        created,
        json!({"format": "runspore.cli/0.2", "root": root,
               "created": [spore_dir, spore_dir.join("workflows")]})
    );
    assert!(spore_dir.join("workflows").is_dir());
    assert!(
        !spore_dir.join(".gitignore").exists(),
        "the database is not in the project folder, so nothing needs ignoring"
    );
    assert!(!repo.home.exists(), "init opened a database");

    let again = ok(&repo.spore(root, &["--json", "init"]));
    assert_eq!(again["created"], json!([]));
}

#[test]
fn init_refuses_a_nested_or_0_1_layout_and_creates_nothing() {
    let repo = Repo::new("init-refuses");
    let root = &repo.root;
    std::fs::create_dir_all(root.join("a/.spore")).unwrap();
    let nested = repo.spore(&root.join("a/b"), &["--json", "init"]);
    assert_eq!(
        (nested.code(), nested.json()["error"]["code"].clone()),
        (Some(2), json!("project.nested"))
    );
    assert!(!root.join(".spore").exists());
    std::fs::remove_dir(root.join("a/.spore")).unwrap();

    for legacy in ["runspore.db", "runspore.db-wal", "runspore.db.files"] {
        let path = root.join(legacy);
        std::fs::write(&path, "").unwrap();
        let out = repo.spore(&root.join("a"), &["--json", "init"]);
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
    let unusable = repo.spore(root, &["--json", "init"]);
    assert_eq!(
        (unusable.code(), unusable.json()["error"]["code"].clone()),
        (Some(2), json!("project.unusable"))
    );
}

#[test]
fn every_subfolder_reaches_one_shared_database_and_read_only_commands_create_none() {
    let repo = Repo::new("walk-up");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    let db = db_of(&repo, root);
    assert!(db.starts_with(repo.home.join("db")), "{}", db.display());
    assert!(db.ends_with("runspore.db"));
    for cwd in [root.clone(), root.join("a/b")] {
        let listed = ok(&repo.spore(&cwd, &["--json", "list"]));
        assert_eq!(listed["db"], json!(db));
        assert_eq!(listed["runs"], json!([]));
        for read_only in [&["status", "run_x"][..], &["events", "run_x"]] {
            let out = repo.spore(&cwd, read_only);
            assert_eq!(out.code(), Some(2), "{read_only:?}: {}", out.stderr);
        }
    }
    assert!(
        !repo.home.exists(),
        "a read-only command created the database"
    );

    // A folder outside any project and outside git has a database of its own.
    let plain = Dir::new("walk-up-plain");
    let plain_db = db_of(&repo, &plain.path);
    assert!(plain_db.starts_with(repo.home.join("db")));
    assert_ne!(plain_db, db);

    workflow(&root.join("flow"), "flow", "true", None);
    let started = ok(&repo.spore(&root.join("a"), &["--json", "start", "../flow"]));
    let run = started["runId"].as_str().unwrap();
    assert!(db.exists());
    assert!(!root.join(".spore/runspore.db").exists());
    assert!(!root.join(".spore/.gitignore").exists());
    for cwd in [root.clone(), root.join("a"), root.join("a/b")] {
        let listed = ok(&repo.spore(&cwd, &["--json", "list"]));
        assert_eq!(listed["db"], json!(db));
        assert_eq!(listed["runs"][0]["runId"], json!(run));
        let status = ok(&repo.spore(&cwd, &["--json", "status", run]));
        assert_eq!(
            (status["db"].clone(), status["runId"].clone()),
            (json!(db), json!(run))
        );
        assert_eq!(status.get("dbOldPlace"), None);
    }
}

#[test]
fn an_existing_database_at_the_old_place_is_still_used_and_status_says_so() {
    let repo = Repo::new("old-place");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    let shared = db_of(&repo, root);
    let old = root.join(".spore/runspore.db");
    workflow(&root.join("flow"), "flow", "true", None);
    let started = ok(&repo.spore(
        root,
        &["--json", "--db", old.to_str().unwrap(), "start", "flow"],
    ));
    let run = started["runId"].as_str().unwrap();
    assert_eq!(
        read(&root.join(".spore/.gitignore")),
        IGNORED,
        "a writing command keeps the old place's ignore file"
    );

    let cwd = root.join("a/b");
    let listed = ok(&repo.spore(&cwd, &["--json", "list"]));
    assert_eq!(listed["db"], json!(old));
    assert_eq!(listed["runs"][0]["runId"], json!(run));
    let status = ok(&repo.spore(&cwd, &["--json", "status", run]));
    assert_eq!(status["db"], json!(old));
    assert_eq!(status["dbOldPlace"], json!({"shared": shared}));
    let text = repo.spore(&cwd, &["status", run]);
    assert_eq!(text.code(), Some(0), "{}", text.stderr);
    let line = text
        .stdout
        .lines()
        .find(|line| line.starts_with("db "))
        .unwrap_or_else(|| panic!("no db line: {}", text.stdout));
    assert!(line.contains(old.to_str().unwrap()), "{line}");
    assert!(line.contains("old place"), "{line}");
    assert!(line.contains(shared.to_str().unwrap()), "{line}");
    assert!(!shared.exists(), "the shared database was not touched");
}

#[test]
fn db_flag_then_runspore_db_then_the_old_place_then_spore_home() {
    let repo = Repo::new("overrides");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    let shared = db_of(&repo, root);

    let elsewhere = repo.dir.file("elsewhere");
    let mut cmd = repo.command(root, &["--json", "list"]);
    cmd.env("SPORE_HOME", &elsewhere);
    let moved = PathBuf::from(ok(&run(cmd))["db"].as_str().unwrap());
    assert_eq!(
        moved.strip_prefix(&elsewhere).unwrap(),
        shared.strip_prefix(&repo.home).unwrap(),
        "SPORE_HOME moves the shared database and keeps its key"
    );

    // An existing database at the old place comes before the shared one.
    let old = root.join(".spore/runspore.db");
    workflow(&root.join("flow"), "flow", "true", None);
    ok(&repo.spore(root, &["--db", old.to_str().unwrap(), "start", "flow"]));
    assert_eq!(db_of(&repo, root), old);

    let env_db = repo.dir.file("env.db");
    let flag_db = repo.dir.file("flag.db");
    let listed = |extra: &[&str]| {
        let mut cmd = repo.command(root, &[extra, &["--json", "list"]].concat());
        cmd.env("RUNSPORE_DB", &env_db);
        ok(&run(cmd))["db"].clone()
    };
    assert_eq!(
        listed(&[]),
        json!(env_db),
        "RUNSPORE_DB before the old place"
    );
    assert_eq!(
        listed(&["--db", flag_db.to_str().unwrap()]),
        json!(flag_db),
        "--db before RUNSPORE_DB"
    );
}

#[test]
fn start_finds_a_workflow_by_its_graph_id() {
    let repo = Repo::new("by-name");
    let root = &repo.root;
    let missing = repo.spore(root, &["--json", "start", "coding-loop"]);
    assert_eq!(
        (missing.code(), missing.json()["error"]["code"].clone()),
        (Some(2), json!("workflow.not-found"))
    );
    ok(&repo.spore(root, &["init"]));
    let workflows = root.join(".spore/workflows");
    workflow(&workflows.join("loop"), "coding-loop", "true", None);
    workflow(&workflows.join("other"), "review", "true", None);

    let started = ok(&repo.spore(&root.join("a/b"), &["--json", "run", "coding-loop"]));
    assert_eq!(started["status"], json!("completed"));
    ok(&repo.spore(&root.join("a"), &["--json", "validate", "review"]));

    let unknown = repo.spore(root, &["--json", "start", "loop"]);
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
    let twice = repo.spore(root, &["--json", "start", "review"]);
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
    let repo = Repo::new("base-dir");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    let log = repo.dir.file("where.log");
    where_flow(root, &log);

    // Started from a subfolder, continued from another: both tries run at the root.
    let started = ok(&repo.spore(&root.join("a"), &["--json", "start", "where"]));
    let run = started["runId"].as_str().unwrap();
    let worked = repo.spore(
        &root.join("a/b"),
        &["--json", "worker", "--run", run, "--until-parked"],
    );
    assert_eq!(ok(&worked)["status"], json!("completed"));
    assert_eq!(lines(&log), std::slice::from_ref(root));

    // The base folder comes from the current folder, not from the database's path:
    // `--db` into a `.spore/` from outside the repository runs outside.
    let outside = Dir::new("base-dir-outside");
    let outside_path = outside.path.canonicalize().unwrap();
    let db = root.join(".spore/runspore.db");
    let flow = root.join(".spore/workflows/where");
    let out = repo.spore(
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
    let out = repo.spore(root, &["--json", "run", "where", "--base-dir", "a/b"]);
    assert_eq!(ok(&out)["status"], json!("completed"));
    let bad = repo.spore(root, &["--json", "start", "where", "--base-dir", "nowhere"]);
    assert_eq!(bad.code(), Some(2));
    assert_eq!(lines(&log), [root.clone(), outside_path, root.join("a/b")]);
}

#[test]
fn a_step_reaches_the_run_database_through_runspore_db() {
    let repo = Repo::new("step-db");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    // The step works in a checkout with its own project folder, as a git worktree does.
    let inner = root.join("a/b");
    std::fs::create_dir_all(inner.join(".spore/workflows")).unwrap();
    let seen = repo.dir.file("seen.json");
    // The native kernel: compiling the Wasmtime one takes seconds in a debug build,
    // longer than the step's 500 ms lease when a heartbeat is slow.
    let script = format!("'{BIN}' --native-kernel --json list > '{}'", seen.display());
    workflow(
        &root.join(".spore/workflows/nested"),
        "nested",
        &script,
        Some("a/b"),
    );

    let out = ok(&repo.spore(root, &["--json", "run", "nested"]));
    assert_eq!(out["status"], json!("completed"));
    let seen: Value = serde_json::from_str(&read(&seen)).unwrap();
    assert_eq!(seen["db"], out["db"]);
    assert_eq!(seen["runs"][0]["runId"], out["runId"]);
    assert!(!inner.join(".spore/runspore.db").exists());
}

/// A ticket step of the dogfood build runs `cargo test` with the build's database in
/// `RUNSPORE_DB`. Every `spore` a test starts names its own database, or clears the
/// variable and names its own `SPORE_HOME`, so the other tests here, run by a step,
/// pass and leave the run's database holding only the run that started them.
#[test]
fn tests_run_by_a_step_keep_off_the_run_database() {
    let repo = Repo::new("step-tests");
    let root = &repo.root;
    ok(&repo.spore(root, &["init"]));
    let tests = std::env::current_exe().unwrap();
    let script = format!(
        "'{}' --quiet --skip tests_run_by_a_step_keep_off",
        tests.display()
    );
    workflow(&root.join(".spore/workflows/tests"), "tests", &script, None);

    let out = ok(&repo.spore(root, &["--json", "run", "tests"]));
    assert_eq!(out["status"], json!("completed"), "{out}");
    let listed = ok(&repo.spore(root, &["--json", "list"]));
    assert_eq!(listed["runs"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["runs"][0]["runId"], out["runId"]);
}
