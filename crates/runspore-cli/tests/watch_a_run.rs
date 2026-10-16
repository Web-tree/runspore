//! The example in `examples/watch-a-run/`, driven as docs/guide/watch-a-run.md drives
//! it: one `spore worker` works a `release` run and the two `ship` runs it starts and
//! prints each change, `list` shows the tree, `status` explains a wait and a stuck
//! step, and `onIntervention` sends a notification through a fake `osascript` and
//! `notify-send` when a deploy gets stuck and when a ship run is rejected.

mod common;

use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Dir, Out, BIN};
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

/// Logs its arguments, one call per line, to `notified.log`.
const NOTIFIER: &str = "#!/bin/sh\necho \"$*\" >> notified.log\n";

/// A project made by `spore init` holding the two example workflows, with `spore` and
/// the fake notifiers in `bin/`.
fn project(name: &str) -> Dir {
    let dir = Dir::new(name);
    let bin = dir.file("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(BIN, bin.join("spore")).unwrap();
    for name in ["osascript", "notify-send"] {
        let path = bin.join(name);
        std::fs::write(&path, NOTIFIER).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let init = spore(&dir, &["init"]);
    assert_eq!(init.code(), Some(0), "{}", init.stderr);
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/watch-a-run");
    for workflow in ["ship", "release"] {
        let to = dir.file(".spore/workflows").join(workflow);
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(example.join(workflow)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
    dir
}

/// `spore` in the project, on the project's own database under a scratch
/// `SPORE_HOME`, with `bin/` first on `PATH`.
fn command(dir: &Dir, extra: &[&str]) -> Command {
    let path = format!(
        "{}:{}",
        dir.file("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let mut cmd = Command::new(BIN);
    cmd.current_dir(&dir.path)
        .env("PATH", path)
        .env("SPORE_HOME", dir.file("spore-home"))
        .env_remove("RUNSPORE_DB")
        .env_remove("RUNSPORE_RUN_ID")
        .env_remove("RUNSPORE_NODE_ID")
        .env_remove("RUNSPORE_FAILPOINTS")
        .args(FAST)
        .args(extra)
        .stdin(Stdio::null());
    cmd
}

/// Runs `spore` to exit and prints the command and its output, which the guide shows.
fn spore(dir: &Dir, extra: &[&str]) -> Out {
    let out = common::run(command(dir, extra));
    eprintln!(
        "$ spore {}\n{}{}(exit {:?})\n",
        extra.join(" "),
        out.stdout,
        out.stderr,
        out.code()
    );
    out
}

fn runs(dir: &Dir) -> Value {
    common::run(command(dir, &["--json", "list"])).json()["runs"].clone()
}

fn row<'a>(runs: &'a Value, key: &str) -> &'a Value {
    runs.as_array()
        .unwrap()
        .iter()
        .find(|r| r["startKey"] == key)
        .unwrap_or_else(|| panic!("{key} not in {runs}"))
}

fn run_id(dir: &Dir, key: &str) -> String {
    row(&runs(dir), key)["runId"].as_str().unwrap().to_owned()
}

/// Waits until `done` holds for the runs `list` shows.
fn wait_for(dir: &Dir, done: impl Fn(&Value) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let runs = runs(dir);
        if done(&runs) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the worker never got there: {runs}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn status_of(runs: &Value, key: &str) -> String {
    row(runs, key)["status"].as_str().unwrap().to_owned()
}

/// The worker's lines for one run, without the time.
fn changes_of(log: &str, key: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| line.split_once("  ").map(|(_, change)| change))
        .filter(|change| change.starts_with(&format!("{key}  ")))
        .map(str::to_owned)
        .collect()
}

fn notified(dir: &Dir) -> Vec<String> {
    std::fs::read_to_string(dir.file("notified.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_guide_watches_a_release_and_is_told_when_a_run_needs_a_person() {
    let dir = project("watch-a-run");
    let worker = command(&dir, &["worker"])
        .stdout(Stdio::null())
        .stderr(File::create(dir.file("worker.log")).unwrap())
        .spawn()
        .unwrap();

    // The release starts one ship run per service, which build and wait at approve.
    let out = spore(
        &dir,
        &[
            "start",
            "release",
            "--key",
            "release-1.4",
            "--input",
            r#"{"version": "1.4"}"#,
        ],
    );
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    wait_for(&dir, |runs| {
        runs.as_array().unwrap().len() == 3
            && row(runs, "release-1.4")["nodeId"] == "released"
            && ["api-1.4", "web-1.4"]
                .iter()
                .all(|key| row(runs, key)["nodeId"] == "approve")
    });
    let all = runs(&dir);
    let release = row(&all, "release-1.4")["runId"].as_str().unwrap();
    for key in ["api-1.4", "web-1.4"] {
        assert_eq!(
            row(&all, key)["parent"],
            json!({"runId": release, "nodeId": "start_ships"})
        );
        assert_eq!(row(&all, key)["depth"], 1);
    }
    let list = spore(&dir, &["list"]).stdout;
    assert!(list.contains("api-1.4  from start_ships"), "{list}");

    // status says what api waits for and how to move it on.
    let api = run_id(&dir, "api-1.4");
    let status = spore(&dir, &["--json", "status", &api]).json();
    assert_eq!(status["waiting"]["signal"], "approval");
    assert_eq!(
        status["waiting"]["commands"],
        json!([
            format!("spore signal {api} approval --outcome approved"),
            format!("spore signal {api} approval --outcome rejected"),
        ])
    );
    spore(&dir, &["status", &api]);
    assert!(notified(&dir).is_empty());

    // With the target down, the approved deploy fails twice and the run stops for a
    // person; onIntervention sends one notification.
    std::fs::write(dir.file("down"), "").unwrap();
    let out = spore(&dir, &["signal", &api, "approval", "--outcome", "approved"]);
    assert_eq!(out.code(), Some(0));
    wait_for(&dir, |runs| {
        status_of(runs, "api-1.4") == "needs-intervention"
    });
    let status = spore(&dir, &["--json", "status", &api]).json();
    assert_eq!(status["intervention"]["reason"], "stuck");
    assert_eq!(status["intervention"]["error"]["code"], "command.exit");
    assert_eq!(
        status["intervention"]["commands"][0],
        format!("spore resolve {api} --retry")
    );
    spore(&dir, &["status", &api]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while notified(&dir).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let told = format!("api: {api} stopped at deploy");
    assert_eq!(notified(&dir).len(), 1, "{:?}", notified(&dir));
    assert!(notified(&dir)[0].contains(&told), "{:?}", notified(&dir));
    assert_eq!(
        std::fs::read_to_string(dir.file("notifications.log")).unwrap(),
        format!("{told}\n")
    );

    // The target is back: a retry deploys, and a completed run tells nobody.
    std::fs::remove_file(dir.file("down")).unwrap();
    let out = spore(&dir, &["resolve", &api, "--retry"]);
    assert_eq!(out.code(), Some(0));
    wait_for(&dir, |runs| status_of(runs, "api-1.4") == "completed");

    // A rejected web ends failed, which tells someone too.
    let web = run_id(&dir, "web-1.4");
    spore(&dir, &["signal", &web, "approval", "--outcome", "rejected"]);
    wait_for(&dir, |runs| status_of(runs, "web-1.4") == "failed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while notified(&dir).len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(notified(&dir).len(), 2, "{:?}", notified(&dir));
    assert!(
        notified(&dir)[1].contains(&format!("web: {web} stopped at stopped")),
        "{:?}",
        notified(&dir)
    );
    spore(&dir, &["list"]);

    let pid = nix::unistd::Pid::from_raw(worker.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::SIGINT).unwrap();
    let out = worker.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130));
    let log = std::fs::read_to_string(dir.file("worker.log")).unwrap();
    eprintln!("$ spore worker\n{log}");
    assert_eq!(
        changes_of(&log, "release-1.4"),
        [
            "release-1.4  step start_ships started, attempt 1",
            "release-1.4  start_ships returned ok -> released",
            "release-1.4  waiting at released for signal released",
        ]
    );
    assert_eq!(
        changes_of(&log, "api-1.4"),
        [
            "api-1.4  step build started, attempt 1",
            "api-1.4  build returned ok -> approve",
            "api-1.4  waiting at approve for signal approval",
            "api-1.4  approve returned approved -> deploy",
            "api-1.4  step deploy started, attempt 1",
            "api-1.4  step deploy attempt 1 failed: command.exit: exit code 1",
            "api-1.4  step deploy started, attempt 2",
            "api-1.4  step deploy attempt 2 failed: command.exit: exit code 1",
            "api-1.4  needs intervention at deploy: stuck: command.exit: exit code 1",
            "api-1.4  step deploy started, attempt 3",
            "api-1.4  deploy returned ok -> done",
            "api-1.4  completed at done",
        ]
    );
}
