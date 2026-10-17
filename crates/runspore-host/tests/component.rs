//! `component` actions (spec/host.md §3, `component`; H41–H43): a step runner sealed
//! into the workflow artifact starts its work detached and hands over a handle; any
//! worker probes and stops it through the runner; every host import is held to the
//! action's grants; `start` refuses a runner it could not run. The real kernel, the
//! SQLite store and the test runner of `crates/runspore-test-runner`.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use runspore_host::{ActivityRegistry, Answer, Engine, EngineConfig, NativeRunner, StepState};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::artifact::{Artifact, Entry, EntryKind, Manifest, Owner, DIALECT, FORMAT};
use runspore_types::canonical;
use runspore_types::digest;
use runspore_types::store::RunKey;
use serde_json::{json, Value};

const DOT: &str = r#"digraph runner {
  dialect = "runspore/0.2";
  start = "work";
  done [kind="complete"];
  work -> done;
}
"#;

/// The settings of a run whose `work` step is the test runner with `grants`, given the
/// run's input.
fn settings(grants: Value, effect: &str) -> Value {
    json!({
        "format": "runspore.workflow/0.2",
        "actions": {
            "work": {"kind": "component", "module": "runner.wasm", "effect": effect,
                     "grants": grants, "retry": {"maxAttempts": 1}}
        },
        "nodes": {"work": {"action": "work", "input": {"$get": ["input"]}}}
    })
}

/// Builds and loads the artifact of a folder under `root` holding the test runner.
fn artifact(root: &Path, settings: &Value) -> Artifact {
    let folder = root.join("workflow");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("workflow.dot"), DOT).unwrap();
    fs::write(folder.join("settings.json"), settings.to_string()).unwrap();
    fs::write(folder.join("runner.wasm"), runspore_wasmtime::TEST_RUNNER).unwrap();
    let sealed = runspore_build::build(&folder).unwrap().seal().unwrap();
    runspore_wasmtime::load_artifact(&sealed.bytes).unwrap()
}

/// A worker on the database at `path`, its files folder beside it.
fn worker(path: &Path) -> Engine {
    let dir = path.parent().unwrap();
    Engine::new(
        Arc::new(SqliteStore::open(path).unwrap()),
        Arc::new(NativeReducer),
        ActivityRegistry::with_builtins(NativeRunner::new()),
        EngineConfig {
            poll_ms: 5,
            heartbeat_ms: 50,
            stop_grace_ms: 200,
            base_dir: dir.to_path_buf(),
            files_dir: Some(dir.join("store.db.files")),
            guard: env!("CARGO_BIN_EXE_runspore-guard").into(),
            ..EngineConfig::default()
        },
    )
}

struct Run {
    path: PathBuf,
    dir: PathBuf,
    engine: Engine,
    key: RunKey,
}

/// Starts a run of the test runner with `grants` and `input`, and works it until it
/// parks.
async fn run(name: &str, grants: Value, effect: &str, input: Value) -> Run {
    let path = temp_db(name);
    let dir = path.parent().unwrap().to_path_buf();
    let artifact = artifact(&dir, &settings(grants, effect));
    let engine = worker(&path);
    let key = engine
        .start_artifact(&artifact, input, name)
        .await
        .unwrap()
        .key;
    engine.run_until_parked(&key).await.unwrap();
    Run {
        path,
        dir,
        engine,
        key,
    }
}

impl Run {
    async fn status(&self) -> String {
        self.engine
            .get_run(&self.key)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn held(&self) -> Option<Value> {
        let view = self.engine.get_run(&self.key).await.unwrap().unwrap();
        let step = self.engine.async_step(&view).await.unwrap()?;
        assert_eq!(step.kind, "component");
        (step.state == StepState::Held).then(|| step.holder.unwrap())
    }

    /// The error of the run's one result.
    async fn error(&self) -> Value {
        let results = results(&self.engine, &self.key).await;
        assert_eq!(results.len(), 1, "{results:?}");
        results[0]["error"].clone()
    }
}

fn alive(pid: i32) -> bool {
    // Neither gone nor a zombie: the process still runs.
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit(')')
            .next()
            .unwrap_or("")
            .trim_start()
            .starts_with('Z')
    })
}

fn gone_within(pid: i32, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn h41_a_detached_process_outlives_its_worker_and_another_worker_stops_it() {
    let run = run(
        "h41",
        json!({"programs": ["sleep"]}),
        "idempotent",
        json!({"spawn": ["sleep", "300"]}),
    )
    .await;
    assert_eq!(run.status().await, "running");
    let holder = run.held().await.expect("the runner handed over a handle");
    let process = &holder["process"];
    assert_eq!(process["program"], "sleep");
    let pid = i32::try_from(process["pgid"].as_i64().unwrap()).unwrap();
    assert!(alive(pid), "the detached process runs");
    // The worker that started it goes away; the process is not its child any more
    // than any job's.
    drop(run.engine);
    assert!(alive(pid), "the process outlives its worker");

    let second = worker(&run.path);
    assert_eq!(
        second.probe_step(&run.key).await.unwrap(),
        Some(Answer::Alive)
    );
    let cancelled = second.cancel(&run.key, "stop", None).await.unwrap();
    assert_eq!(cancelled.stops.len(), 1, "the runner's stop ran");
    assert_eq!(cancelled.stops[0].exit, Some(0));
    assert!(
        gone_within(pid, Duration::from_secs(5)),
        "the runner's stop ended the process"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn h41_a_probe_through_the_runner_frees_a_step_whose_process_ended() {
    let run = run(
        "h41-dead",
        json!({"programs": ["sleep"]}),
        "unsafe",
        json!({"spawn": ["sleep", "0.2"]}),
    )
    .await;
    let pid = run.held().await.unwrap()["process"]["pgid"]
        .as_i64()
        .unwrap();
    assert!(gone_within(
        i32::try_from(pid).unwrap(),
        Duration::from_secs(5)
    ));
    assert_eq!(
        run.engine.probe_step(&run.key).await.unwrap(),
        Some(Answer::Dead)
    );
    run.engine.run_until_parked(&run.key).await.unwrap();
    // An unsafe step whose holder died waits for a person.
    assert_eq!(run.status().await, "needs-intervention");
}

#[tokio::test(flavor = "multi_thread")]
async fn h42_a_call_outside_the_grants_fails_the_step_naming_the_grant() {
    let marker = std::env::temp_dir().join(format!("runspore-h42-{}", std::process::id()));
    let cases = [
        (
            "h42-program",
            json!({"spawn": ["touch", marker.to_string_lossy()]}),
            "programs",
            "touch",
        ),
        (
            "h42-socket",
            json!({"request": {"socket": "/tmp/runspore-none.sock", "body": "hi\n"}}),
            "sockets",
            "/tmp/runspore-none.sock",
        ),
        (
            "h42-folder",
            json!({"read": "/etc/hostname"}),
            "folders",
            "/etc/hostname",
        ),
    ];
    for (name, input, grant, value) in cases {
        let run = run(name, json!({"programs": ["sleep"]}), "idempotent", input).await;
        let error = run.error().await;
        assert_eq!(error["code"], "component.not-granted", "{name}: {error}");
        assert_eq!(error["details"], json!({"grant": grant, "value": value}));
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains(&format!("grants.{grant}")),
            "{error}"
        );
    }
    assert!(!marker.exists(), "nothing ungranted was started");
}

#[tokio::test(flavor = "multi_thread")]
async fn h42_granted_sockets_and_folders_work() {
    let path = temp_db("h42-granted");
    let dir = path.parent().unwrap().to_path_buf();
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(dir.join("data/answer.txt"), "forty-two").unwrap();
    let listener = UnixListener::bind(dir.join("api.sock")).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            reader.read_line(&mut line).unwrap();
            (&stream)
                .write_all(format!("got {}\nmore", line.trim()).as_bytes())
                .unwrap();
        }
    });
    let grants = json!({"sockets": ["api.sock"], "folders": ["data"]});
    let artifact = artifact(&dir, &settings(grants, "idempotent"));
    let engine = worker(&path);
    for (key, input, holder) in [
        (
            "socket",
            json!({"request": {"socket": "./api.sock", "body": "ping\n"}}),
            json!({"reply": "got ping"}),
        ),
        (
            "folder",
            json!({"read": "data/answer.txt"}),
            json!({"read": "forty-two"}),
        ),
        (
            "list",
            json!({"list": format!("{}/data", dir.display())}),
            json!({"list": ["answer.txt"]}),
        ),
    ] {
        let key = engine
            .start_artifact(&artifact, input, key)
            .await
            .unwrap()
            .key;
        engine.run_until_parked(&key).await.unwrap();
        let view = engine.get_run(&key).await.unwrap().unwrap();
        let step = engine.async_step(&view).await.unwrap().expect("held");
        assert_eq!(step.holder, Some(holder));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn h42_an_ungranted_probe_answers_unknown_and_leaves_the_step_held() {
    let handle = json!({"read": "/etc/hostname"}).to_string();
    let run = run(
        "h42-probe",
        json!({}),
        "idempotent",
        json!({ "handle": handle }),
    )
    .await;
    assert!(run.held().await.is_some());
    assert_eq!(
        run.engine.probe_step(&run.key).await.unwrap(),
        Some(Answer::Unknown)
    );
    assert!(
        run.held().await.is_some(),
        "an unknown answer frees nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runner_failure_and_a_bad_handle_end_the_step() {
    let failed = run(
        "fail",
        json!({}),
        "idempotent",
        json!({"fail": {"code": "herdr.down", "message": "no herdr", "retryable": false}}),
    )
    .await;
    let error = failed.error().await;
    assert_eq!(
        (error["code"].as_str(), error["message"].as_str()),
        (Some("herdr.down"), Some("no herdr"))
    );
    let unlabelled = run(
        "fail-code",
        json!({}),
        "idempotent",
        json!({"fail": {"code": "Not A Label", "message": "m"}}),
    )
    .await;
    assert_eq!(unlabelled.error().await["code"], "component.failed");
    let malformed = run(
        "malformed",
        json!({}),
        "idempotent",
        json!({"handle": "{1.5"}),
    )
    .await;
    assert_eq!(
        malformed.error().await["code"],
        "component.malformed-output"
    );
    let trapped = run("trap", json!({}), "unsafe", json!({"trap": true})).await;
    let results = results(&trapped.engine, &trapped.key).await;
    assert_eq!(results[0]["status"], "unknown", "{results:?}");
    assert_eq!(results[0]["error"]["code"], "component.trap");
    assert_eq!(trapped.status().await, "needs-intervention");
    let _ = &trapped.dir;
}

/// An artifact whose `work` action runs `module`, holding `bytes` there, made without
/// the build, which refuses such a module first.
fn hand_made(module: &str, bytes: Option<&[u8]>) -> Artifact {
    let package = canonical::to_vec(&json!({
        "format": "runspore.workflow/0.2", "name": "runner", "start": "work",
        "actions": {"work": {"kind": "component", "module": module, "effect": "idempotent"}},
        "nodes": {"work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
                  "done": {"kind": "complete"}}
    }))
    .unwrap();
    let main = digest::package(&package);
    let mut blobs = vec![(main.clone(), package.clone())];
    let mut entries = Vec::new();
    if let Some(bytes) = bytes {
        let blob = digest::blob(bytes);
        entries.push(Entry {
            path: module.to_string(),
            digest: blob.clone(),
            kind: EntryKind::Asset,
            owner: Some(Owner::Action("work".into())),
            executable: false,
        });
        blobs.push((blob, bytes.to_vec()));
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        dialect: DIALECT.into(),
        name: "runner".into(),
        main,
        entries,
    };
    let manifest = canonical::to_vec(&serde_json::to_value(&manifest).unwrap()).unwrap();
    let blobs: Vec<(String, &[u8])> = blobs.iter().map(|(d, b)| (d.clone(), &b[..])).collect();
    let bytes = runspore_build::encode_artifact(&manifest, &blobs).unwrap();
    runspore_wasmtime::load_artifact(&bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn h43_start_refuses_a_runner_it_could_not_run() {
    let path = temp_db("h43");
    let engine = worker(&path);
    let code = |r: Result<runspore_host::StartOutcome, runspore_host::HostError>| {
        r.expect_err("refused").code().to_string()
    };
    let package = hand_made("runner.wasm", None).package;
    assert_eq!(
        code(engine.start(&package, Value::Null, "plain").await),
        "component.no-artifact"
    );
    assert_eq!(
        code(
            engine
                .start_artifact(&hand_made("runner.wasm", None), Value::Null, "none")
                .await
        ),
        "component.module-missing"
    );
    let kernel = hand_made("runner.wasm", Some(runspore_wasmtime::COMPONENT));
    let refused = engine
        .start_artifact(&kernel, Value::Null, "kernel")
        .await
        .expect_err("the kernel is no step runner");
    assert_eq!(refused.code(), "component.invalid");
    assert!(
        refused.to_string().contains("runspore:runner/runner@0.1.0"),
        "{refused}"
    );
    let mut grants: Value =
        serde_json::from_slice(&hand_made("runner.wasm", None).package).unwrap();
    grants["actions"]["work"]["grants"] = json!({"network": ["*"]});
    assert_eq!(
        code(
            engine
                .start(&serde_json::to_vec(&grants).unwrap(), Value::Null, "grants")
                .await
        ),
        "component.grant-unsupported"
    );
    assert!(
        engine.list_runs(None, 10).await.unwrap().items.is_empty(),
        "no run"
    );
}
