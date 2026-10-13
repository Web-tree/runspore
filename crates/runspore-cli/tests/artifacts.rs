//! Workflow artifacts through the compiled binary (spec/artifact.md, cli.md): `build`
//! writes one immutable file, a run pins the artifact it started from, and nothing done
//! to the folder afterwards reaches the run.

mod common;

use common::{effect_argv, Dir};
use runspore_types::artifact::{Entry, EntryKind, Manifest, DIALECT, FORMAT};
use runspore_types::digest;
use serde_json::json;

const FAST: [&str; 7] = [
    "--native-kernel",
    "--poll-ms",
    "20",
    "--lease-ms",
    "500",
    "--heartbeat-ms",
    "100",
];

const DOT: &str = r#"digraph pinned {
  dialect = "runspore/0.2";
  start = "prep";
  approve [kind="await-signal"];
  done [kind="complete"];
  prep -> approve;
  approve -> done [on="approved"];
}
"#;

fn settings(result: &str) -> String {
    let argv = effect_argv("prep", "true");
    format!(
        r#"{{
  "format": "runspore.workflow/0.2",
  "actions": {{
    "prep": {{"kind": "command", "effect": "idempotent", "argv": {argv},
             "assets": ["tools/prep.sh"]}}
  }},
  "nodes": {{
    "prep": {{"action": "prep"}},
    "approve": {{"signal": "approval", "instructions": "approve.md"}},
    "done": {{"output": {{"$literal": "{result}"}}}}
  }}
}}
"#
    )
}

/// A scratch directory holding the workflow folder `flow`.
fn folder(name: &str) -> Dir {
    let dir = Dir::new(name);
    let flow = dir.file("flow");
    std::fs::create_dir_all(flow.join("tools")).unwrap();
    std::fs::write(flow.join("workflow.dot"), DOT).unwrap();
    std::fs::write(flow.join("settings.json"), settings("v1")).unwrap();
    std::fs::write(flow.join("approve.md"), "Approve when it looks right.\n").unwrap();
    std::fs::write(flow.join("tools/prep.sh"), "#!/bin/sh\n").unwrap();
    dir
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut all = FAST.to_vec();
    all.extend_from_slice(extra);
    all
}

#[test]
fn build_writes_the_artifact_beside_the_folder_and_inspect_reads_it() {
    let dir = folder("artifact-build");
    let built = dir.run(&["--json", "build", "flow"]);
    assert_eq!(built.code(), Some(0), "{}{}", built.stdout, built.stderr);
    let out = built.json();
    let wasm = dir.file("flow.wasm");
    assert_eq!(out["artifact"], json!("flow.wasm"));
    let bytes = std::fs::read(&wasm).expect("flow.wasm beside the folder");
    assert_eq!(out["artifactDigest"], json!(digest::artifact(&bytes)));
    assert!(!dir.file("flow/flow.wasm").exists());
    assert!(!dir.file("runspore.db").exists(), "build opens no database");

    let again = dir.run(&["--json", "build", "flow/workflow.dot", "-o", "copy.wasm"]);
    assert_eq!(again.code(), Some(0), "{}", again.stdout);
    assert_eq!(
        std::fs::read(dir.file("copy.wasm")).unwrap(),
        bytes,
        "the same folder builds the same bytes"
    );

    let inspect = dir.run(&["--json", "inspect", "flow.wasm"]);
    assert_eq!(inspect.code(), Some(0), "{}", inspect.stdout);
    let shown = inspect.json();
    let manifest: Manifest = serde_json::from_value(shown["manifest"].clone()).unwrap();
    let source = |path: &str, text: &str| Entry {
        path: path.into(),
        digest: digest::blob(text.as_bytes()),
        kind: EntryKind::Source,
        owner: None,
        executable: false,
    };
    assert_eq!(
        (manifest.format.as_str(), manifest.dialect.as_str()),
        (FORMAT, DIALECT)
    );
    assert_eq!(manifest.name, "pinned");
    let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "approve.md",
            "settings.json",
            "tools/prep.sh",
            "workflow.dot"
        ]
    );
    assert_eq!(
        manifest.entry("workflow.dot"),
        Some(&source("workflow.dot", DOT))
    );
    assert_eq!(shown["package"]["name"], json!("pinned"));
    assert_eq!(
        shown["package"]["nodes"]["approve"].get("instructions"),
        None
    );
    assert_eq!(shown["artifactDigest"], out["artifactDigest"]);
    assert_eq!(manifest.main, out["packageDigest"].as_str().unwrap());

    let text = dir.run(&["inspect", "flow.wasm"]);
    assert_eq!(text.code(), Some(0));
    assert!(text.stdout.contains("\"manifest\""), "{}", text.stdout);

    std::fs::write(dir.file("doc.json"), "{}").unwrap();
    let not_folder = dir.run(&["--json", "build", "doc.json"]);
    assert_eq!(
        (
            not_folder.code(),
            not_folder.json()["error"]["code"].clone()
        ),
        (Some(2), json!("workflow.folder"))
    );
    let not_artifact = dir.run(&["--json", "inspect", "doc.json"]);
    assert_eq!(
        (
            not_artifact.code(),
            not_artifact.json()["error"]["code"].clone()
        ),
        (Some(2), json!("artifact.invalid"))
    );
}

/// A run's status with what differs between two runs of one workflow removed.
fn decisions(mut status: serde_json::Value) -> serde_json::Value {
    let fields = status.as_object_mut().unwrap();
    for own in ["runId", "startKey", "artifactDigest", "db"] {
        fields.remove(own);
    }
    status
}

#[test]
fn build_composed_writes_one_file_that_runs_like_the_plain_artifact() {
    let dir = folder("artifact-composed");
    let plain = dir.run(&["--json", "build", "flow"]).json();
    let built = dir.run(&["--json", "build", "flow", "--composed"]);
    assert_eq!(built.code(), Some(0), "{}{}", built.stdout, built.stderr);
    let out = built.json();
    assert_eq!(out["artifact"], json!("flow-composed.wasm"));
    let bytes = std::fs::read(dir.file("flow-composed.wasm")).expect("beside the folder");
    assert_eq!(out["artifactDigest"], json!(digest::artifact(&bytes)));
    assert_ne!(out["artifactDigest"], plain["artifactDigest"]);
    assert_eq!(out["packageDigest"], plain["packageDigest"]);
    assert_eq!(out["composed"], json!(true));
    assert_eq!(
        out["kernelDigest"],
        json!(digest::hash("kernel", &[runspore_wasmtime::COMPONENT])),
        "the kernel inside is the one this spore runs"
    );
    assert_eq!(plain.get("composed"), None);
    assert!(
        bytes.len() > std::fs::read(dir.file("flow.wasm")).unwrap().len(),
        "the composed file carries the kernel too"
    );

    let named = dir.run(&["--json", "build", "flow", "--composed", "-o", "one.wasm"]);
    assert_eq!(named.code(), Some(0), "{}", named.stdout);
    assert_eq!(std::fs::read(dir.file("one.wasm")).unwrap(), bytes);

    let inspect = dir.run(&["--json", "inspect", "flow-composed.wasm"]);
    assert_eq!(inspect.code(), Some(0), "{}", inspect.stdout);
    let plain_inspect = dir.run(&["--json", "inspect", "flow.wasm"]).json();
    assert_eq!(inspect.json()["manifest"], plain_inspect["manifest"]);
    assert_eq!(inspect.json()["package"], plain_inspect["package"]);

    // The same decisions from either file.
    std::fs::remove_dir_all(dir.file("flow")).unwrap();
    let from_plain = dir.run(&args(&["--json", "run", "flow.wasm", "--key", "plain"]));
    let from_composed = dir.run(&args(&[
        "--json",
        "run",
        "flow-composed.wasm",
        "--key",
        "composed",
    ]));
    assert_eq!(from_plain.code(), Some(3), "{}", from_plain.stdout);
    assert_eq!(from_composed.code(), Some(3), "{}", from_composed.stdout);
    assert_eq!(
        from_composed.json()["artifactDigest"],
        out["artifactDigest"],
        "a run from the composed file pins that file"
    );
    assert_eq!(
        decisions(from_composed.json()),
        decisions(from_plain.json())
    );
    let mut ended = Vec::new();
    for parked in [&from_plain, &from_composed] {
        let run = parked.json()["runId"].as_str().unwrap().to_string();
        let signal = ["signal", run.as_str(), "approval", "--outcome", "approved"];
        assert_eq!(dir.run(&args(&signal)).code(), Some(0));
        let done = dir.run(&args(&[
            "--json",
            "worker",
            "--run",
            &run,
            "--until-parked",
        ]));
        assert_eq!(done.code(), Some(0), "{}", done.stdout);
        ended.push(decisions(done.json()));
    }
    assert_eq!(ended[0], ended[1]);
    assert_eq!(ended[0]["result"]["output"], json!("v1"));
}

#[test]
fn a_run_pins_its_artifact_and_never_reads_the_folder_again() {
    let dir = folder("artifact-pin");
    let built = dir.run(&["--json", "build", "flow"]).json();

    let parked = dir.run(&args(&["--json", "run", "flow", "--key", "k"]));
    let status = parked.json();
    assert_eq!(parked.code(), Some(3), "{status}");
    assert_eq!(status["artifactDigest"], built["artifactDigest"]);
    assert_eq!(status["packageDigest"], built["packageDigest"]);
    let run = status["runId"].as_str().unwrap().to_string();

    // Edit, then delete, the folder: the run sees neither.
    std::fs::write(dir.file("flow/settings.json"), settings("v2")).unwrap();
    std::fs::write(dir.file("flow/workflow.dot"), "digraph broken {").unwrap();
    std::fs::remove_dir_all(dir.file("flow")).unwrap();

    let dot = dir.run(&["inspect", "--run", &run, "--file", "workflow.dot"]);
    assert_eq!(
        (dot.code(), dot.stdout.as_str()),
        (Some(0), DOT),
        "{}",
        dot.stderr
    );
    let settings_shown = dir.run(&[
        "--json",
        "inspect",
        "--run",
        &run,
        "--file",
        "settings.json",
    ]);
    assert_eq!(settings_shown.json()["text"], json!(settings("v1")));
    assert_eq!(settings_shown.json()["kind"], json!("source"));
    let missing = dir.run(&["--json", "inspect", "--run", &run, "--file", "nope.md"]);
    assert_eq!(
        (missing.code(), missing.json()["error"]["code"].clone()),
        (Some(2), json!("artifact.no-file"))
    );

    let signal = ["signal", run.as_str(), "approval", "--outcome", "approved"];
    assert_eq!(dir.run(&args(&signal)).code(), Some(0));
    let done = dir.run(&args(&[
        "--json",
        "worker",
        "--run",
        &run,
        "--until-parked",
    ]));
    assert_eq!(
        (done.code(), done.json()["result"]["output"].clone()),
        (Some(0), json!("v1")),
        "{}",
        done.stdout
    );
}

#[test]
fn a_prebuilt_artifact_validates_starts_and_runs() {
    let dir = folder("artifact-start");
    assert_eq!(dir.run(&["build", "flow"]).code(), Some(0));
    std::fs::remove_dir_all(dir.file("flow")).unwrap();
    let digest = digest::artifact(&std::fs::read(dir.file("flow.wasm")).unwrap());

    let valid = dir.run(&["--json", "validate", "flow.wasm"]);
    assert_eq!(valid.json()["valid"], json!(true), "{}", valid.stdout);
    let parked = dir.run(&args(&["--json", "run", "flow.wasm", "--key", "k"]));
    assert_eq!(parked.code(), Some(3), "{}", parked.stdout);
    assert_eq!(parked.json()["artifactDigest"], json!(digest));
    let run = parked.json()["runId"].as_str().unwrap().to_string();
    let text = dir.run(&["status", &run]);
    assert!(
        text.stdout.contains(&format!("artifact   {digest}")),
        "{}",
        text.stdout
    );
}

#[test]
fn an_artifact_with_an_unsafe_path_is_refused() {
    let dir = Dir::new("artifact-forged");
    let package = br#"{"format":"runspore.workflow/0.2","name":"f","nodes":{"done":{"kind":"complete"}},"start":"done"}"#;
    let manifest = Manifest {
        format: FORMAT.into(),
        dialect: DIALECT.into(),
        name: "f".into(),
        main: digest::package(package),
        entries: vec![Entry {
            path: "../../escape.sh".into(),
            digest: digest::blob(b""),
            kind: EntryKind::Source,
            owner: None,
            executable: false,
        }],
    };
    let bytes = runspore_build::encode_artifact(
        &manifest.encode(),
        &[(manifest.main.clone(), package.as_slice())],
    )
    .unwrap();
    std::fs::write(dir.file("forged.wasm"), bytes).unwrap();
    for command in [["validate", "forged.wasm"], ["start", "forged.wasm"]] {
        let mut argv = vec!["--json"];
        argv.extend(command);
        let out = dir.run(&argv);
        let error = out.json()["error"].clone();
        assert_eq!(
            (out.code(), error["code"].clone()),
            (Some(2), json!("artifact.invalid-path")),
            "{error}"
        );
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .starts_with("forged.wasm: "),
            "{error}"
        );
    }
    let list = dir.run(&["--json", "list"]);
    assert_eq!(
        list.json()["runs"],
        json!([]),
        "a refused artifact starts nothing"
    );
}
