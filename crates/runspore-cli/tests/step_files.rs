//! Step files and work folders through the compiled binary (cli.md §Project folder,
//! host.md §3): the files folder is `<database>.files`, a bundled program runs as
//! `./<asset>`, and a run whose stored artifact is lost waits quarantined until the
//! database is repaired and the run released.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::Dir;
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

const DOT: &str = r#"digraph show {
  dialect = "runspore/0.2";
  start = "show";
  done [kind="complete"];
  show -> done;
}
"#;

const SETTINGS: &str = r#"{
  "format": "runspore.workflow/0.2",
  "actions": {
    "show": {"kind": "command", "effect": "idempotent", "argv": ["./tools/show.sh"],
             "assets": ["tools/show.sh"], "output": "json"}
  },
  "nodes": {
    "show": {"action": "show", "instructions": "show.md"},
    "done": {"output": {"$get": ["nodes", "show", "output"]}}
  }
}
"#;

/// Prints the variables and the instructions as one JSON object.
const SCRIPT: &str = r#"#!/bin/sh
printf '{"program":"%s","files":"%s","instructions":"%s","work":"%s","text":"%s"}' \
  "$0" "$RUNSPORE_STEP_FILES" "$RUNSPORE_INSTRUCTIONS" "$RUNSPORE_WORK_DIR" \
  "$(cat "$RUNSPORE_INSTRUCTIONS")"
"#;

fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(flow.join("tools")).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    std::fs::write(flow.join("settings.json"), SETTINGS).unwrap();
    std::fs::write(flow.join("show.md"), "Show the files.").unwrap();
    let script = flow.join("tools/show.sh");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

#[test]
fn a_step_gets_its_files_beside_the_database() {
    let dir = folder("step-files-cli");
    let out = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    assert_eq!(out.code(), Some(0), "{}{}", out.stdout, out.stderr);
    let run = out.json()["runId"].as_str().unwrap().to_string();
    let status = dir.run(&["--json", "status", &run]).json();
    let artifact = status["artifactDigest"].as_str().unwrap();
    let hex = artifact.strip_prefix("sha256:").unwrap();
    let files_dir = std::fs::canonicalize(dir.path.clone())
        .unwrap()
        .join("runspore.db.files");
    let steps = files_dir.join("steps").join(hex).join("show");
    let seen = &status["result"]["output"];
    let canonical = |v: &Value| {
        std::fs::canonicalize(v.as_str().unwrap())
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(canonical(&seen["files"]), steps.to_string_lossy());
    assert_eq!(
        canonical(&seen["program"]),
        steps.join("tools/show.sh").to_string_lossy()
    );
    assert_eq!(
        canonical(&seen["instructions"]),
        steps.join("show.md").to_string_lossy()
    );
    assert!(
        canonical(&seen["work"]).starts_with(&*files_dir.join("work").join(&run).to_string_lossy())
    );
    assert_eq!(seen["text"], json!("Show the files."));
}

#[test]
fn a_lost_artifact_quarantines_the_run_until_it_is_restored_and_released() {
    let dir = folder("step-files-unreadable");
    let started = dir.run(&args(&["--json", "start", "flow", "--key", "k"]));
    assert_eq!(started.code(), Some(0), "{}", started.stderr);
    let run = started.json()["runId"].as_str().unwrap().to_string();

    let db = rusqlite::Connection::open(dir.file("runspore.db")).unwrap();
    let (digest, body): (String, Vec<u8>) = db
        .query_row("SELECT digest, body FROM artifacts", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    db.execute("DELETE FROM artifacts", []).unwrap();

    let worked = dir.run(&args(&[
        "--json",
        "worker",
        "--run",
        &run,
        "--until-parked",
    ]));
    assert_eq!(worked.code(), Some(5), "{}{}", worked.stdout, worked.stderr);
    let status = dir.run(&["--json", "status", &run]).json();
    assert_eq!(status["quarantine"]["code"], json!("artifact.unreadable"));

    db.execute(
        "INSERT INTO artifacts (digest, body) VALUES (?1, ?2)",
        rusqlite::params![digest, body],
    )
    .unwrap();
    assert_eq!(dir.run(&["release", &run]).code(), Some(0));
    let worked = dir.run(&args(&[
        "--json",
        "worker",
        "--run",
        &run,
        "--until-parked",
    ]));
    assert_eq!(worked.code(), Some(0), "{}{}", worked.stdout, worked.stderr);
    let status = dir.run(&["--json", "status", &run]).json();
    assert_eq!(status["result"]["output"]["text"], json!("Show the files."));
}
