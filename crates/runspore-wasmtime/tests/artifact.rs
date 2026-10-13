//! Workflow artifacts (spec/artifact.md): what the build seals is what the sandboxed
//! loader reads, and the loader refuses what it must not trust.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use runspore_types::artifact::{Entry, EntryKind, Manifest, Owner, DIALECT, FORMAT};
use runspore_types::digest;
use runspore_types::reducer::{Envelope, Identity, Limits, Reducer, TransitionRequest};
use runspore_wasmtime::{load_artifact, ArtifactReader, Guards, WasmtimeReducer, COMPONENT};
use wasmparser::{Parser, Payload};

const DOT: &str = r#"digraph deploy {
  dialect = "runspore/0.2";
  start = "check";
  check -> review;
  review [kind="await-signal"];
  review -> done [on="approved"];
  done [kind="complete"];
}
"#;

const SETTINGS: &str = r#"{
  "format": "runspore.workflow/0.2",
  "actions": {
    "check": {"kind": "command", "effect": "read-only", "argv": ["./assets/check.sh"],
              "assets": ["assets/check.sh", "./assets/../assets/data.txt"],
              "async": {"probe": {"command": {"argv": ["./probes/alive"]}},
                        "watch": {"command": {"argv": ["sleep", "1"]}}}}
  },
  "nodes": {
    "check": {"action": "check", "instructions": "instructions/check.md"},
    "review": {"signal": "review", "instructions": "instructions/check.md",
               "outputSchemas": {"approved": "schemas/approved.json"}}
  }
}
"#;

/// A scratch workflow folder, removed on drop.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("runspore-artifact-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let write = |rel: &str, text: &str| {
            let file = path.join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        write("workflow.dot", DOT);
        write("settings.json", SETTINGS);
        write("instructions/check.md", "Check it.\n");
        write("instructions/review.md", "Review it.\n");
        write("assets/check.sh", "#!/bin/sh\nexit 0\n");
        write("assets/data.txt", "data\n");
        write("probes/alive", "#!/bin/sh\necho '{\"holder\":\"alive\"}'\n");
        write("schemas/approved.json", "{\"type\": \"object\"}\n");
        write("stray.md", "never built in\n");
        for exe in ["assets/check.sh", "probes/alive"] {
            std::fs::set_permissions(path.join(exe), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entry(path: &str, kind: EntryKind, owner: Option<Owner>, executable: bool, text: &str) -> Entry {
    Entry {
        path: path.into(),
        digest: digest::blob(text.as_bytes()),
        kind,
        owner,
        executable,
    }
}

fn top_level_names(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let (mut imports, mut exports, mut depth) = (Vec::new(), Vec::new(), 0usize);
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.expect("component parses") {
            Payload::Version { .. } => depth += 1,
            Payload::End(_) => depth -= 1,
            Payload::ComponentImportSection(s) if depth == 1 => {
                imports.extend(
                    s.into_iter()
                        .map(|i| i.unwrap().name.full_name().into_owned()),
                );
            }
            Payload::ComponentExportSection(s) if depth == 1 => {
                exports.extend(
                    s.into_iter()
                        .map(|e| e.unwrap().name.full_name().into_owned()),
                );
            }
            _ => {}
        }
    }
    (imports, exports)
}

/// The bytes of each component nested directly in the top-level component.
fn nested_components(bytes: &[u8]) -> Vec<Vec<u8>> {
    let (mut nested, mut depth) = (Vec::new(), 0usize);
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.expect("component parses") {
            Payload::Version { .. } => depth += 1,
            Payload::End(_) => depth -= 1,
            Payload::ComponentSection {
                unchecked_range, ..
            } if depth == 1 => {
                let range = unchecked_range.start as usize..unchecked_range.end as usize;
                nested.push(bytes[range].to_vec());
            }
            _ => {}
        }
    }
    nested
}

#[test]
fn the_composed_artifact_holds_the_kernel_unchanged_and_exports_both_worlds() {
    let folder = Folder::new("composed");
    let sealed = runspore_build::build(folder.path())
        .unwrap()
        .seal()
        .unwrap();
    let composed =
        runspore_build::compose(COMPONENT, &sealed.bytes).expect("the composition encodes");

    let (imports, exports) = top_level_names(&composed);
    assert_eq!(imports, Vec::<String>::new());
    assert_eq!(
        exports,
        [
            "runspore:machine/reducer@0.1.0",
            "runspore:artifact/reader@0.1.0"
        ]
    );
    assert_eq!(
        nested_components(&composed),
        [COMPONENT.to_vec(), sealed.bytes.clone()],
        "the certified kernel and the workflow artifact, byte for byte"
    );
    assert_eq!(
        runspore_build::compose(COMPONENT, &sealed.bytes).unwrap(),
        composed,
        "the same inputs compose to the same bytes"
    );

    // A host with its own kernel reads the composed file like the plain artifact.
    let plain = load_artifact(&sealed.bytes).unwrap();
    let loaded = load_artifact(&composed).expect("the composed artifact loads");
    assert_eq!(loaded.manifest, plain.manifest);
    assert_eq!(loaded.package, plain.package);
    assert_eq!(loaded.digest, digest::artifact(&composed));
    assert_ne!(
        loaded.digest, plain.digest,
        "provenance names the file started from"
    );

    // A host with no kernel of its own runs the one inside, and gets the same decisions.
    let kernel = WasmtimeReducer::from_component(&composed, Guards::default())
        .expect("the composed file runs as a kernel");
    assert_eq!(kernel.describe(), runspore_kernel::describe());
    let request = TransitionRequest {
        identity: Identity {
            package_digest: plain.manifest.main.clone(),
            kernel_digest: digest::hash("kernel", &[COMPONENT]),
            semantics_version: "0.2".to_string(),
            codec_version: "jcs-1".to_string(),
        },
        graph: plain.package.clone(),
        snapshot: None,
        input_event: Envelope {
            event_id: "evt-start".to_string(),
            sequence: 1,
            accepted_at_ms: 1_759_752_000_123,
            kind: "run.started".to_string(),
            payload: br#"{"input":{}}"#.to_vec(),
        },
        frozen_limits: Limits::default(),
    };
    assert_eq!(
        kernel.transition(&request),
        runspore_kernel::transition(&request)
    );
}

#[test]
fn a_built_folder_loads_with_every_declared_file_and_nothing_else() {
    let folder = Folder::new("loads");
    let built = runspore_build::build(folder.path()).expect("the folder builds");
    let sealed = built.seal().expect("the artifact seals");
    let artifact = load_artifact(&sealed.bytes).expect("the artifact loads");

    assert_eq!(artifact.digest, digest::artifact(&sealed.bytes));
    assert_eq!(artifact.package, sealed.package);
    assert_eq!(artifact.manifest, sealed.manifest);
    let action = || Some(Owner::Action("check".into()));
    let expected = Manifest {
        format: FORMAT.into(),
        dialect: DIALECT.into(),
        name: "deploy".into(),
        main: digest::package(&sealed.package),
        entries: vec![
            entry(
                "assets/check.sh",
                EntryKind::Asset,
                action(),
                true,
                "#!/bin/sh\nexit 0\n",
            ),
            entry(
                "assets/data.txt",
                EntryKind::Asset,
                action(),
                false,
                "data\n",
            ),
            entry(
                "instructions/check.md",
                EntryKind::Instructions,
                Some(Owner::Node("check".into())),
                false,
                "Check it.\n",
            ),
            entry(
                "instructions/check.md",
                EntryKind::Instructions,
                Some(Owner::Node("review".into())),
                false,
                "Check it.\n",
            ),
            entry(
                "probes/alive",
                EntryKind::Asset,
                action(),
                true,
                "#!/bin/sh\necho '{\"holder\":\"alive\"}'\n",
            ),
            entry(
                "schemas/approved.json",
                EntryKind::Source,
                None,
                false,
                "{\"type\": \"object\"}\n",
            ),
            entry("settings.json", EntryKind::Source, None, false, SETTINGS),
            entry("workflow.dot", EntryKind::Source, None, false, DOT),
        ],
    };
    assert_eq!(artifact.manifest, expected);

    let package: serde_json::Value = serde_json::from_slice(&artifact.package).unwrap();
    assert!(
        package["actions"]["check"].get("assets").is_none(),
        "{package}"
    );
    assert!(
        package["nodes"]["check"].get("instructions").is_none(),
        "{package}"
    );

    let mut reader = ArtifactReader::open(&sealed.bytes).unwrap();
    for e in &artifact.manifest.entries {
        let bytes = reader
            .entry(&e.digest)
            .expect("every entry's blob is there");
        assert_eq!(digest::blob(&bytes), e.digest, "{}", e.path);
    }
    assert_eq!(reader.blob("sha256:nope").unwrap(), None);
    assert_eq!(reader.blob("").unwrap(), None);
}

#[test]
fn the_component_imports_nothing_and_exports_only_the_reader() {
    let folder = Folder::new("world");
    let sealed = runspore_build::build(folder.path())
        .unwrap()
        .seal()
        .unwrap();
    let (imports, exports) = top_level_names(&sealed.bytes);
    assert_eq!(imports, Vec::<String>::new());
    assert_eq!(exports, ["runspore:artifact/reader@0.1.0"]);
}

#[test]
fn the_same_folder_seals_to_the_same_bytes() {
    let folder = Folder::new("same");
    let first = runspore_build::build(folder.path())
        .unwrap()
        .seal()
        .unwrap();
    let second = runspore_build::build(folder.path())
        .unwrap()
        .seal()
        .unwrap();
    assert_eq!(first.bytes, second.bytes);
    std::fs::write(folder.path().join("stray.md"), "changed, never read\n").unwrap();
    let third = runspore_build::build(folder.path())
        .unwrap()
        .seal()
        .unwrap();
    assert_eq!(
        first.bytes, third.bytes,
        "an undeclared file is not part of the artifact"
    );
}

/// An artifact whose `manifest()` returns `manifest` and whose `blob(main)` returns
/// `package`.
fn forged(manifest: &Manifest, package: &[u8]) -> Vec<u8> {
    runspore_build::encode_artifact(&manifest.encode(), &[(manifest.main.clone(), package)])
        .unwrap()
}

fn base(package: &[u8]) -> Manifest {
    Manifest {
        format: FORMAT.into(),
        dialect: DIALECT.into(),
        name: "w".into(),
        main: digest::package(package),
        entries: vec![entry("workflow.dot", EntryKind::Source, None, false, "")],
    }
}

#[test]
fn the_loader_refuses_what_it_must_not_trust() {
    let package = br#"{"format":"runspore.workflow/0.2"}"#;
    let code = |bytes: &[u8]| load_artifact(bytes).unwrap_err().code;

    assert!(load_artifact(&forged(&base(package), package)).is_ok());
    for path in [
        "/etc/passwd",
        "../outside",
        "a/../../b",
        "a\\b",
        "",
        "a//b",
        "./a",
    ] {
        let mut m = base(package);
        m.entries[0].path = path.into();
        assert_eq!(
            code(&forged(&m, package)),
            "artifact.invalid-path",
            "{path:?}"
        );
    }
    let mut twice = base(package);
    twice.entries.push(twice.entries[0].clone());
    assert_eq!(code(&forged(&twice, package)), "artifact.invalid-path");

    let mut old = base(package);
    old.dialect = "runspore/0.1".into();
    assert_eq!(code(&forged(&old, package)), "artifact.unsupported");

    let other = br#"{"format":"runspore.workflow/0.2","name":"x"}"#;
    assert_eq!(
        code(&forged(&base(package), other)),
        "artifact.digest-mismatch"
    );
    let missing = runspore_build::encode_artifact(&base(package).encode(), &[]).unwrap();
    assert_eq!(code(&missing), "artifact.invalid");

    assert_eq!(
        code(COMPONENT),
        "artifact.invalid",
        "the kernel is not an artifact"
    );
    assert_eq!(code(b"not wasm"), "artifact.invalid");
    let not_canonical = runspore_build::encode_artifact(b"{ }", &[]).unwrap();
    assert_eq!(code(&not_canonical), "artifact.invalid");
}
