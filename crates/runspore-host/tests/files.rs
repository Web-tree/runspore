//! Step files and work folders (spec/host.md §3, H20–H24): a step's declared files
//! reach it read-only from the run's stored artifact, checked before every use, each
//! invocation gets a work folder, and the sweep removes what no unfinished run needs.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::*;
use nix::fcntl::{Flock, FlockArg};
use runspore_host::{
    ActivityContext, ActivityOutput, ActivityRegistry, Engine, EngineConfig, NativeRunner,
    StepFiles,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::artifact::Artifact;
use runspore_types::store::{RunKey, RunView, Store};
use serde_json::{json, Value};

/// `work` runs the bundled `./tools/work.sh`, which fails its first try of every
/// visit; `check` sends the run back to `work` once.
const LOOP: &str = r#"digraph files {
  dialect = "runspore/0.2";
  start = "work";
  done [kind="complete"];
  work -> check;
  check -> work [on="again"];
  check -> done [on="ok"];
}
"#;

/// `work`, then a wait for the `approval` signal: the run stays unfinished.
const WAITING: &str = r#"digraph waiting {
  dialect = "runspore/0.2";
  start = "work";
  approve [kind="await-signal"];
  done [kind="complete"];
  work -> approve;
  approve -> done [on="approved"];
}
"#;

/// Records what the script saw in its work folder, then fails try 1.
const SCRIPT: &str = r#"#!/bin/sh
set -e
w="$RUNSPORE_WORK_DIR"
echo "$RUNSPORE_ATTEMPT" >> "$w/tries"
{
  echo "$0"
  echo "$RUNSPORE_STEP_FILES"
  echo "${RUNSPORE_INSTRUCTIONS-unset}"
} > "$w/env.$RUNSPORE_ATTEMPT"
cat "$RUNSPORE_INSTRUCTIONS" > "$w/instructions.$RUNSPORE_ATTEMPT"
cat "$RUNSPORE_STEP_FILES/tools/data.txt" > "$w/data.$RUNSPORE_ATTEMPT"
[ "$RUNSPORE_ATTEMPT" != 1 ]
"#;

fn settings() -> Value {
    json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "work": {"kind": "command", "effect": "idempotent", "argv": ["./tools/work.sh"],
                     "assets": ["tools/work.sh", "tools/data.txt"],
                     "retry": {"maxAttempts": 2, "backoffMs": "0"}},
            "check": {"kind": "native", "function": "check", "effect": "idempotent",
                      "outcomes": ["again", "ok"]}
        },
        "nodes": {
            "work": {"action": "work", "instructions": "work.md"},
            "check": {"action": "check", "instructions": "check.md"}
        }
    })
}

/// `settings` without the `check` node and its action, for [`WAITING`].
fn waiting_settings() -> Value {
    let mut settings = settings();
    settings["actions"].as_object_mut().unwrap().remove("check");
    settings["nodes"].as_object_mut().unwrap().remove("check");
    settings["nodes"]["approve"] = json!({"signal": "approval"});
    settings
}

/// Writes a workflow folder under `root` and builds and loads its artifact.
fn artifact(root: &Path, name: &str, dot: &str, settings: &Value) -> Artifact {
    let folder = root.join(name);
    fs::create_dir_all(folder.join("tools")).unwrap();
    fs::write(folder.join("workflow.dot"), dot).unwrap();
    fs::write(folder.join("settings.json"), settings.to_string()).unwrap();
    fs::write(folder.join("work.md"), "Do the work.\n").unwrap();
    fs::write(folder.join("check.md"), "Check the work.\n").unwrap();
    fs::write(folder.join("tools/data.txt"), "built\n").unwrap();
    let script = folder.join("tools/work.sh");
    fs::write(&script, SCRIPT).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let sealed = runspore_build::build(&folder).unwrap().seal().unwrap();
    runspore_wasmtime::load_artifact(&sealed.bytes).unwrap()
}

/// Paths a native function saw in its context, by visit.
type Seen = Arc<Mutex<Vec<(Option<StepFiles>, Option<PathBuf>)>>>;

/// `check` answers `again` on its first visit and `ok` after; `on_first` runs first.
fn check(seen: Seen, on_first: impl Fn(&ActivityContext) + Send + Sync + 'static) -> NativeRunner {
    NativeRunner::new().function("check", move |ctx: ActivityContext, _input| {
        let first = ctx.visit == 1;
        if first {
            on_first(&ctx);
        }
        seen.lock()
            .unwrap()
            .push((ctx.files.clone(), ctx.work_dir.clone()));
        async move {
            let outcome = if first { "again" } else { "ok" };
            ActivityOutput::Success {
                outcome: outcome.into(),
                output: Value::Null,
            }
        }
    })
}

fn engine(store: Arc<dyn Store>, native: NativeRunner, files_dir: Option<PathBuf>) -> Engine {
    Engine::new(
        store,
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(native),
        EngineConfig {
            guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
            files_dir,
            ..EngineConfig::default()
        },
    )
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

/// Every file under `dir`, by relative path, with its mode.
fn tree(dir: &Path) -> BTreeMap<String, u32> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, u32>) {
        for item in fs::read_dir(dir).unwrap() {
            let item = item.unwrap();
            let name = format!("{prefix}{}", item.file_name().to_string_lossy());
            if item.file_type().unwrap().is_dir() {
                walk(&item.path(), &format!("{name}/"), out);
            } else {
                out.insert(name, mode(&item.path()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, "", &mut out);
    out
}

fn hex(artifact: &Artifact) -> &str {
    artifact.digest.strip_prefix("sha256:").unwrap()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

async fn finished(engine: &Engine, key: &RunKey) -> RunView {
    let view = engine.run_until_parked(key).await.unwrap();
    assert_eq!(view.quarantine, None);
    view
}

#[tokio::test]
async fn h20_a_step_gets_its_own_files_and_a_work_folder_per_visit() {
    let db = temp_db("files-h20");
    let root = db.parent().unwrap().to_path_buf();
    let files_dir = root.join("store.db.files");
    let artifact = artifact(&root, "loop", LOOP, &settings());
    let seen: Seen = Arc::default();
    let store = Arc::new(SqliteStore::open(&db).unwrap());
    let engine = engine(store, check(seen.clone(), |_| {}), Some(files_dir.clone()));
    let key = engine
        .start_artifact(&artifact, Value::Null, "h20")
        .await
        .unwrap()
        .key;
    let view = finished(&engine, &key).await;
    assert_eq!(view.status, "completed");

    let steps = files_dir.join("steps").join(hex(&artifact));
    let work_files = steps.join("work");
    assert_eq!(
        tree(&work_files),
        BTreeMap::from([
            ("tools/data.txt".into(), 0o444),
            ("tools/work.sh".into(), 0o555),
            ("work.md".into(), 0o444),
        ]),
        "only the node's instructions and its action's assets"
    );
    assert_eq!(mode(&work_files), 0o555);
    assert_eq!(mode(&work_files.join("tools")), 0o555);
    assert_eq!(
        tree(&steps.join("check")),
        BTreeMap::from([("check.md".into(), 0o444)])
    );

    let mut folders: Vec<PathBuf> = fs::read_dir(files_dir.join("work").join(&key.run))
        .unwrap()
        .map(|i| i.unwrap().path())
        .collect();
    folders.sort();
    // Two visits of `work` and two of `check`: four invocations, four folders.
    assert_eq!(folders.len(), 4, "{folders:?}");
    let work_folders: Vec<&PathBuf> = folders
        .iter()
        .filter(|f| f.join("tries").exists())
        .collect();
    assert_eq!(
        work_folders.len(),
        2,
        "each visit of work has its own folder"
    );
    for folder in work_folders {
        assert_eq!(mode(folder), 0o700);
        assert_eq!(
            read(&folder.join("tries")),
            "1\n2\n",
            "both tries share the folder"
        );
        let env = read(&folder.join("env.2"));
        let lines: Vec<&str> = env.lines().collect();
        assert_eq!(
            lines,
            [
                work_files.join("tools/work.sh").to_str().unwrap(),
                work_files.to_str().unwrap(),
                work_files.join("work.md").to_str().unwrap(),
            ],
            "argv[0] ran the step files' copy, and the variables name them"
        );
        assert_eq!(read(&folder.join("instructions.2")), "Do the work.\n");
    }

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let (files, work_dir) = &seen[0];
    let files = files.as_ref().unwrap();
    assert_eq!(files.dir, steps.join("check"));
    assert_eq!(files.instructions, Some(steps.join("check/check.md")));
    assert!(files.assets.is_empty());
    assert_ne!(work_dir, &seen[1].1, "each visit has its own work folder");
}

#[tokio::test]
async fn h21_tampered_step_files_are_made_again_before_the_next_use() {
    let db = temp_db("files-h21");
    let root = db.parent().unwrap().to_path_buf();
    let files_dir = root.join("store.db.files");
    let artifact = artifact(&root, "loop", LOOP, &settings());
    let work_files = files_dir.join("steps").join(hex(&artifact)).join("work");
    let tamper = {
        let work_files = work_files.clone();
        move |_: &ActivityContext| {
            fs::set_permissions(&work_files, fs::Permissions::from_mode(0o755)).unwrap();
            fs::set_permissions(work_files.join("tools"), fs::Permissions::from_mode(0o755))
                .unwrap();
            let data = work_files.join("tools/data.txt");
            fs::set_permissions(&data, fs::Permissions::from_mode(0o644)).unwrap();
            fs::write(&data, "edited\n").unwrap();
            fs::write(work_files.join("extra.md"), "not declared\n").unwrap();
            fs::set_permissions(
                work_files.join("work.md"),
                fs::Permissions::from_mode(0o666),
            )
            .unwrap();
        }
    };
    let store = Arc::new(SqliteStore::open(&db).unwrap());
    let engine = engine(
        store,
        check(Arc::default(), tamper),
        Some(files_dir.clone()),
    );
    let key = engine
        .start_artifact(&artifact, Value::Null, "h21")
        .await
        .unwrap()
        .key;
    assert_eq!(finished(&engine, &key).await.status, "completed");

    assert_eq!(
        tree(&work_files),
        BTreeMap::from([
            ("tools/data.txt".into(), 0o444),
            ("tools/work.sh".into(), 0o555),
            ("work.md".into(), 0o444),
        ])
    );
    assert_eq!(mode(&work_files), 0o555);
    let seen: Vec<String> = fs::read_dir(files_dir.join("work").join(&key.run))
        .unwrap()
        .map(|i| i.unwrap().path().join("data.2"))
        .filter(|p| p.exists())
        .map(|p| read(&p))
        .collect();
    assert_eq!(
        seen,
        ["built\n", "built\n"],
        "the second visit saw the built bytes"
    );
}

#[tokio::test]
async fn h22_start_refuses_files_without_a_files_dir() {
    let db = temp_db("files-h22");
    let root = db.parent().unwrap().to_path_buf();
    let artifact = artifact(&root, "loop", LOOP, &settings());
    let store = Arc::new(SqliteStore::open(&db).unwrap());
    let engine = engine(store, check(Arc::default(), |_| {}), None);
    let error = engine
        .start_artifact(&artifact, Value::Null, "h22")
        .await
        .unwrap_err();
    assert_eq!(error.code(), runspore_host::FILES_DIR_MISSING);
    assert!(engine.list_runs(None, 10).await.unwrap().items.is_empty());
    assert_eq!(engine.get_artifact(&artifact.digest).await.unwrap(), None);
}

struct Hide(Arc<AtomicBool>);

#[async_trait::async_trait]
impl Hook for Hide {
    fn hides_artifacts(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn h23_an_unreadable_artifact_quarantines_the_run_until_it_is_repaired() {
    let db = temp_db("files-h23");
    let root = db.parent().unwrap().to_path_buf();
    let files_dir = root.join("store.db.files");
    let artifact = artifact(&root, "loop", LOOP, &settings());
    let hidden = Arc::new(AtomicBool::new(true));
    let store = Arc::new(Hooked {
        inner: Arc::new(SqliteStore::open(&db).unwrap()),
        hook: Hide(hidden.clone()),
    });
    let engine = engine(store, check(Arc::default(), |_| {}), Some(files_dir));
    let key = engine
        .start_artifact(&artifact, Value::Null, "h23")
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    let quarantine = view.quarantine.expect("quarantined");
    assert_eq!(quarantine.code, runspore_host::ARTIFACT_UNREADABLE);
    let invocations = engine.list_invocations(&key).await.unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].owner, None, "no attempt was claimed");
    assert!(results(&engine, &key).await.is_empty());

    hidden.store(false, Ordering::SeqCst);
    engine.release(&key).await.unwrap();
    assert_eq!(finished(&engine, &key).await.status, "completed");
}

#[tokio::test]
async fn h23_an_absent_entry_blob_quarantines_the_run() {
    let db = temp_db("files-h23-blob");
    let root = db.parent().unwrap().to_path_buf();
    let built = artifact(&root, "loop", LOOP, &settings());
    // The same manifest, holding the package and nothing else.
    let bytes = runspore_build::encode_artifact(
        &built.manifest.encode(),
        &[(built.manifest.main.clone(), &built.package)],
    )
    .unwrap();
    let artifact = runspore_wasmtime::load_artifact(&bytes).unwrap();
    let store = Arc::new(SqliteStore::open(&db).unwrap());
    let engine = engine(
        store,
        check(Arc::default(), |_| {}),
        Some(root.join("store.db.files")),
    );
    let key = engine
        .start_artifact(&artifact, Value::Null, "h23-blob")
        .await
        .unwrap()
        .key;
    let view = engine.run_until_parked(&key).await.unwrap();
    assert_eq!(
        view.quarantine.map(|q| q.code),
        Some(runspore_host::ARTIFACT_UNREADABLE.to_string())
    );
    assert!(results(&engine, &key).await.is_empty());
}

#[tokio::test]
async fn h24_the_sweep_removes_step_files_no_unfinished_run_needs() {
    let db = temp_db("files-h24");
    let root = db.parent().unwrap().to_path_buf();
    let files_dir = root.join("store.db.files");
    let finished_artifact = artifact(&root, "loop", LOOP, &settings());
    let waiting_artifact = artifact(&root, "waiting", WAITING, &waiting_settings());
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&db).unwrap());
    let new_engine = || {
        engine(
            store.clone(),
            check(Arc::default(), |_| {}),
            Some(files_dir.clone()),
        )
    };
    let first = new_engine();
    let done = first
        .start_artifact(&finished_artifact, Value::Null, "done")
        .await
        .unwrap()
        .key;
    assert_eq!(finished(&first, &done).await.status, "completed");
    let waiting = first
        .start_artifact(&waiting_artifact, Value::Null, "waiting")
        .await
        .unwrap()
        .key;
    assert_eq!(finished(&first, &waiting).await.status, "waiting");
    let steps = files_dir.join("steps");
    let done_files = steps.join(hex(&finished_artifact));
    let waiting_files = steps.join(hex(&waiting_artifact));
    assert!(done_files.join("work").is_dir() && waiting_files.join("work").is_dir());

    // A try of the finished run's artifact holds its lock: the sweep leaves it.
    let lock_path = steps.join(format!("{}.lock", hex(&finished_artifact)));
    let held = Flock::lock(fs::File::open(&lock_path).unwrap(), FlockArg::LockShared).unwrap();
    new_engine().tick().await.unwrap();
    assert!(done_files.is_dir(), "a held lock keeps the step files");
    drop(held);

    new_engine().tick().await.unwrap();
    assert!(
        !done_files.exists(),
        "no unfinished run pins the finished run's artifact"
    );
    assert!(
        waiting_files.join("work").is_dir(),
        "the waiting run keeps its step files"
    );
    assert!(lock_path.exists(), "lock files are kept");
    let leftovers: Vec<_> = fs::read_dir(&steps)
        .unwrap()
        .map(|i| i.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(
        files_dir.join("work").join(&done.run).is_dir(),
        "work folders are never removed"
    );
}
