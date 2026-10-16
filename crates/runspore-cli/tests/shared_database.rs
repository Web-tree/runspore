//! The shared database (spec/cli.md §Project folder) through the compiled binary and
//! real git: every worktree and clone of one repository uses
//! `$SPORE_HOME/db/<repo key>/runspore.db`, so runs outlive the worktree they started
//! in.

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
    "5000",
    "--heartbeat-ms",
    "100",
];

/// A scratch folder holding repositories and the `SPORE_HOME` their `spore` uses.
struct Scratch {
    dir: Dir,
    home: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = Dir::new(name);
        let home = dir.path.canonicalize().unwrap().join("home");
        Self { dir, home }
    }

    /// A folder `name` in the scratch folder, at its real path.
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.file(name);
        std::fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }

    /// `git` in `cwd`, away from the person's git settings; panics unless it succeeds.
    fn git(&self, cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        isolate(&mut cmd);
        let out = cmd.current_dir(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repository at `name` on branch `main`, with `origin` when given, holding a
    /// committed project folder whose `where` workflow appends the folder it runs in
    /// to `where.log` in the scratch folder.
    fn repo(&self, name: &str, origin: Option<&str>) -> PathBuf {
        let root = self.folder(name);
        self.git(&root, &["init", "--quiet", "--initial-branch", "main"]);
        if let Some(url) = origin {
            self.git(&root, &["remote", "add", "origin", url]);
        }
        ok(&self.spore(&root, &["init"]));
        let flow = root.join(".spore/workflows/where");
        std::fs::create_dir_all(&flow).unwrap();
        std::fs::write(
            flow.join("workflow.dot"),
            "digraph \"where\" {\n  dialect = \"runspore/0.2\";\n  graph [start=\"work\"];\n  \
             done [kind=complete];\n  work -> done;\n}\n",
        )
        .unwrap();
        let script = format!("pwd -P >> '{}'", self.log().display());
        let settings = json!({"format": "runspore.workflow/0.2",
                              "actions": {"step": {"kind": "command", "effect": "idempotent",
                                                   "argv": ["/bin/sh", "-c", script]}},
                              "nodes": {"work": {"action": "step"}}});
        std::fs::write(
            flow.join("settings.json"),
            serde_json::to_vec_pretty(&settings).unwrap(),
        )
        .unwrap();
        self.git(&root, &["add", "."]);
        self.git(&root, &["commit", "--quiet", "-m", "where"]);
        root
    }

    fn log(&self) -> PathBuf {
        self.dir.file("where.log")
    }

    fn logged(&self) -> Vec<PathBuf> {
        std::fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(PathBuf::from)
            .collect()
    }

    /// `spore` in `cwd` with no `RUNSPORE_DB`, so it finds its database itself.
    fn spore(&self, cwd: &Path, args: &[&str]) -> Out {
        let mut cmd = Command::new(BIN);
        isolate(&mut cmd);
        cmd.current_dir(cwd)
            .env("SPORE_HOME", &self.home)
            .env_remove("RUNSPORE_DB")
            .env_remove("RUNSPORE_FAILPOINTS")
            .env_remove("RUNSPORE_RUN_ID")
            .env_remove("RUNSPORE_NODE_ID")
            .args(FAST)
            .args(args)
            .stdin(Stdio::null());
        run(cmd)
    }

    /// The database a `--json list` in `cwd` used, and the run ids it lists.
    fn listed(&self, cwd: &Path) -> (PathBuf, Vec<Value>) {
        let listed = ok(&self.spore(cwd, &["--json", "list"]));
        let runs = listed["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["runId"].clone())
            .collect();
        (PathBuf::from(listed["db"].as_str().unwrap()), runs)
    }

    /// The shared database of the repository key `key`.
    fn db(&self, key: impl AsRef<Path>) -> PathBuf {
        self.home.join("db").join(key).join("runspore.db")
    }
}

/// Keeps git and `spore`'s calls to it off the person's and the system's git settings.
fn isolate(cmd: &mut Command) {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "spore")
        .env("GIT_AUTHOR_EMAIL", "spore@example.com")
        .env("GIT_COMMITTER_NAME", "spore")
        .env("GIT_COMMITTER_EMAIL", "spore@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR");
}

/// Asserts exit 0; the output as JSON.
fn ok(out: &Out) -> Value {
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
    serde_json::from_str(out.stdout.trim()).unwrap_or(Value::Null)
}

/// `path` without its root, as a repository key.
fn key(path: &Path) -> PathBuf {
    path.strip_prefix("/").unwrap().to_path_buf()
}

#[test]
fn two_worktrees_of_one_repository_share_their_runs() {
    let scratch = Scratch::new("shared-worktrees");
    let main = scratch.repo("main", None);
    let wt = scratch.dir.path.canonicalize().unwrap().join("wt");
    scratch.git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "wt",
            wt.to_str().unwrap(),
        ],
    );

    // Without `origin`, the key is the git common dir every worktree shares.
    let db = scratch.db(key(&main.join(".git")));
    let started = ok(&scratch.spore(&main, &["--json", "start", "where"]));
    let run = started["runId"].clone();
    for cwd in [&main, &wt] {
        assert_eq!(scratch.listed(cwd), (db.clone(), vec![run.clone()]));
    }
    assert!(!main.join(".spore/runspore.db").exists());
    assert!(!wt.join(".spore/runspore.db").exists());

    // A worker in the other worktree takes the run, and works it where it started.
    let worked = scratch.spore(
        &wt,
        &[
            "--json",
            "worker",
            "--run",
            run.as_str().unwrap(),
            "--until-parked",
        ],
    );
    assert_eq!(ok(&worked)["status"], json!("completed"));
    assert_eq!(scratch.logged(), std::slice::from_ref(&main));
}

#[test]
fn two_clones_of_one_origin_share_a_database_and_a_folder_outside_git_has_its_own() {
    let scratch = Scratch::new("shared-clones");
    let ssh = scratch.repo("ssh", Some("git@github.com:Web-tree/runspore.git"));
    let https = scratch.repo("https", Some("https://github.com/Web-tree/runspore"));
    let db = scratch.db("github.com/Web-tree/runspore");

    let started = ok(&scratch.spore(&ssh, &["--json", "start", "where"]));
    for cwd in [&ssh, &https, &https.join(".spore/workflows")] {
        assert_eq!(
            scratch.listed(cwd),
            (db.clone(), vec![started["runId"].clone()])
        );
    }

    let plain = scratch.folder("plain");
    ok(&scratch.spore(&plain, &["init"]));
    assert_eq!(scratch.listed(&plain), (scratch.db(key(&plain)), vec![]));
}

#[test]
fn a_removed_worktree_keeps_its_runs_and_its_next_step_names_the_folder() {
    let scratch = Scratch::new("shared-removed");
    let main = scratch.repo("main", None);
    let wt = scratch.dir.path.canonicalize().unwrap().join("wt");
    scratch.git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "wt",
            wt.to_str().unwrap(),
        ],
    );
    let started = ok(&scratch.spore(&wt, &["--json", "start", "where"]));
    let run = started["runId"].as_str().unwrap();
    scratch.git(
        &main,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    assert!(!wt.exists());

    assert_eq!(scratch.listed(&main).1, [json!(run)]);
    let out = scratch.spore(&main, &["--json", "worker", "--run", run, "--until-parked"]);
    assert_eq!(out.code(), Some(4), "{}{}", out.stdout, out.stderr);
    let status = out.json();
    assert_eq!(status["status"], json!("needs-intervention"));
    let error = &status["invocation"]["error"];
    assert_eq!(error["code"], json!("command.base-dir-missing"), "{status}");
    assert_eq!(error["details"], json!({"baseDir": wt}));
    let message = error["message"].as_str().unwrap();
    assert!(message.contains(wt.to_str().unwrap()), "{message}");
    assert_eq!(status["invocation"]["attempt"], json!(1), "not retried");
    assert!(scratch.logged().is_empty(), "the step ran");
}
