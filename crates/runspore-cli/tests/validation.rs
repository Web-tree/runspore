//! `spore validate` reports every problem at once (spec/cli.md, Validation): the text
//! blocks, `--short`, `--json`, `--strict`, `--fix` and `spore explain`, through the
//! compiled binary, on the catalogue's example folders.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{run, Dir, Out, BIN};
use serde_json::{json, Value};

/// The catalogue's example folders and their expected text.
fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runspore-build/catalogue/examples")
}

/// A `PATH` holding exactly the programs the examples run, as stubs.
fn programs(dir: &Dir, names: &[&str]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.file("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in names {
        let path = bin.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// `spore <args>` in `cwd`, with `path` as the only `PATH`.
fn spore(cwd: &Path, path: &Path, args: &[&str]) -> Out {
    let mut cmd = Command::new(BIN);
    cmd.current_dir(cwd)
        .env("PATH", path)
        .env_remove("RUNSPORE_DB")
        .args(args);
    run(cmd)
}

/// A copy of an example folder in a scratch directory.
fn copy(dir: &Dir, name: &str) -> PathBuf {
    let from = examples().join(name);
    let to = dir.file(name);
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap().flatten() {
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
    to
}

#[test]
fn nightly_backup_gives_every_problem_in_one_run() {
    let dir = Dir::new("validate-nightly");
    let path = programs(&dir, &["aws", "pg_dump"]);
    copy(&dir, "nightly-backup");
    let out = spore(&dir.path, &path, &["validate", "nightly-backup"]);
    let expected = std::fs::read_to_string(examples().join("nightly-backup.txt")).unwrap();
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    assert_eq!(out.stderr, expected);
    assert_eq!(out.stdout, "");
    assert!(out
        .stderr
        .ends_with("\n4 problems and 1 warning in nightly-backup. Nothing was run.\n"));
    assert!(!dir.file("nightly-backup/runspore.db").exists());
}

#[test]
fn every_example_gives_its_checked_in_text() {
    let dir = Dir::new("validate-catalogue");
    let path = programs(&dir, &runspore_build::EXAMPLE_PROGRAMS);
    let mut names: Vec<String> = std::fs::read_dir(examples())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() > 90, "{names:?}");
    for name in names {
        let out = spore(&examples(), &path, &["validate", &name]);
        let expected = std::fs::read_to_string(examples().join(format!("{name}.txt"))).unwrap();
        let wanted = if expected.contains("Nothing was run.") {
            2
        } else {
            0
        };
        assert_eq!(out.code(), Some(wanted), "{name}: {}", out.stderr);
        assert_eq!(out.stderr, expected, "{name}");
        let verdict = if wanted == 0 {
            format!("{name} is valid\n")
        } else {
            String::new()
        };
        assert_eq!(out.stdout, verdict, "{name}");
        for kernel in [
            "kernel",
            "W01",
            "W07",
            "W12",
            "Failure",
            "InvalidInput",
            "missing field",
        ] {
            assert!(
                !out.stderr.contains(kernel),
                "{name}: {kernel:?} in {}",
                out.stderr
            );
        }
    }
}

/// The validation guide, whose excerpts must be what spore prints.
fn guide() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/validation.md"))
        .unwrap()
}

#[test]
fn short_prints_one_line_per_problem() {
    let dir = Dir::new("validate-short");
    let path = programs(&dir, &["aws", "pg_dump"]);
    copy(&dir, "nightly-backup");
    let out = spore(&dir.path, &path, &["validate", "--short", "nightly-backup"]);
    assert_eq!(out.code(), Some(2));
    assert!(guide().contains(&out.stderr), "the guide shows other lines");
    let lines: Vec<&str> = out.stderr.lines().collect();
    assert_eq!(
        lines,
        [
            "nightly-backup/workflow.dot:1:1: dot.no-start: The workflow does not say which step runs first.",
            "nightly-backup/workflow.dot:5:3: dot.missing-arrow: Step \"upload\" can end with the outcome \"ok\", but no arrow leaves it for \"ok\".",
            "nightly-backup/settings.json:17:5: settings.misplaced-key: Every key under \"nodes\" must be a step in workflow.dot, and there is no step \"outcomes\".",
            "nightly-backup/settings.json:9:5: action.no-effect: Action \"upload\" does not say whether it is safe to run again after a crash.",
            "nightly-backup/settings.json:6:16: command.argv-spaces: \"pg_dump --format=custom --file=backup.dump shop\" is read as one program name. spore does not use a shell, so the words are not split.",
        ]
    );
}

#[test]
fn json_carries_every_field_of_a_diagnostic() {
    let dir = Dir::new("validate-json");
    let path = programs(&dir, &["aws", "pg_dump"]);
    copy(&dir, "nightly-backup");
    let out = spore(&dir.path, &path, &["--json", "validate", "nightly-backup"]);
    assert_eq!(out.code(), Some(2));
    let report = out.json();
    assert_eq!(
        (&report["valid"], &report["problems"], &report["warnings"]),
        (&json!(false), &json!("4"), &json!("1"))
    );
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let fields = [
        "severity", "code", "title", "message", "fix", "why", "file", "range", "path", "args",
        "fixes", "docs",
    ];
    for d in diagnostics {
        for field in fields {
            assert!(d.get(field).is_some(), "no {field} in {d}");
        }
    }
    let by = |code: &str| {
        diagnostics
            .iter()
            .find(|d| d["code"] == code)
            .unwrap()
            .clone()
    };

    let effect = by("action.no-effect");
    assert_eq!(effect["severity"], "problem");
    assert_eq!(effect["title"], "Missing \"effect\"");
    assert_eq!(effect["path"], "/actions/upload");
    assert_eq!(effect["file"], "nightly-backup/settings.json");
    assert_eq!(
        effect["range"],
        json!({"start": {"line": 9, "column": 5}, "end": {"line": 9, "column": 13}})
    );
    assert_eq!(effect["fixes"].as_array().unwrap().len(), 4);
    let first = &effect["fixes"][0];
    assert_eq!(first["applicability"], "needs-choice");
    assert_eq!(
        (
            &first["edits"][0]["op"],
            &first["edits"][0]["at"],
            &first["edits"][0]["value"]
        ),
        (
            &json!("set"),
            &json!("/actions/upload/effect"),
            &json!("idempotent")
        )
    );
    assert_eq!(
        effect["docs"],
        "https://github.com/Web-tree/runspore/blob/main/docs/codes/action.no-effect.md"
    );

    let spaces = by("command.argv-spaces");
    assert_eq!(spaces["severity"], "warning");
    assert_eq!(
        spaces["args"][0],
        json!({"name": "item", "kind": "input", "pointer": "/actions/dump/argv/0",
               "value": "pg_dump --format=custom --file=backup.dump shop"})
    );
    let fix = &spaces["fixes"][0];
    assert_eq!(fix["applicability"], "safe");
    assert_eq!(fix["edits"][0]["at"], "/actions/dump/argv");
    assert_eq!(
        fix["edits"][0]["value"],
        json!(["pg_dump", "--format=custom", "--file=backup.dump", "shop"])
    );
    // The same edit as a text edit, as an editor applies it.
    assert_eq!(
        fix["edits"][0]["text"],
        "[\"pg_dump\", \"--format=custom\", \"--file=backup.dump\", \"shop\"]"
    );
    assert_eq!(
        fix["edits"][0]["range"]["start"],
        json!({"line": 6, "column": 15})
    );

    let arrow = by("dot.missing-arrow");
    assert_eq!(arrow["path"], Value::Null);
    assert_eq!(
        arrow["why"],
        "action \"upload\" lists no outcomes, so it ends with \"ok\", and the run would have nowhere to go."
    );
}

/// The issue's fixed folder, with `dump`'s command line written as one item again.
fn warning_only(dir: &Dir) -> PathBuf {
    let folder = copy(dir, "nightly-backup");
    std::fs::write(
        folder.join("workflow.dot"),
        "digraph \"nightly-backup\" {\n  dialect = \"runspore/0.2\";\n  start   = \"dump\";\n\n  dump   [label=\"Dump the database\"];\n  upload [label=\"Upload the dump\"];\n  done   [kind=\"complete\"];\n\n  dump -> upload;\n  upload -> done;\n}\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("settings.json"),
        "{\n  \"format\": \"runspore.workflow/0.2\",\n  \"actions\": {\n    \"dump\": {\n      \"kind\": \"command\",\n      \"argv\": [\"pg_dump --format=custom --file=backup.dump shop\"],\n      \"effect\": \"idempotent\"\n    },\n    \"upload\": {\n      \"kind\": \"command\",\n      \"argv\": [\"aws\", \"s3\", \"cp\", \"backup.dump\", \"s3://acme-backups/shop/latest.dump\"],\n      \"effect\": \"idempotent\"\n    }\n  },\n  \"nodes\": {\n    \"dump\": { \"action\": \"dump\" },\n    \"upload\": { \"action\": \"upload\" }\n  }\n}\n",
    )
    .unwrap();
    folder
}

#[test]
fn warnings_pass_unless_strict() {
    let dir = Dir::new("validate-strict");
    let path = programs(&dir, &["aws", "pg_dump"]);
    warning_only(&dir);
    let out = spore(&dir.path, &path, &["validate", "nightly-backup"]);
    assert_eq!(out.code(), Some(0), "{}", out.stdout);
    assert!(out.stderr.contains("command.argv-spaces"), "{}", out.stderr);
    assert!(out.stderr.ends_with("1 warning in nightly-backup.\n"));
    assert_eq!(out.stdout, "nightly-backup is valid\n");
    let strict = spore(
        &dir.path,
        &path,
        &["validate", "--strict", "nightly-backup"],
    );
    assert_eq!(strict.code(), Some(2), "{}", strict.stdout);
    assert!(!strict.stdout.contains("is valid"));
    let json = spore(
        &dir.path,
        &path,
        &["--json", "validate", "--strict", "nightly-backup"],
    );
    assert_eq!(json.json()["valid"], false);

    // A program this machine lacks is a warning too: the run may happen elsewhere.
    let bare = programs(&Dir::new("validate-no-aws"), &["pg_dump"]);
    let out = spore(&dir.path, &bare, &["validate", "--short", "nightly-backup"]);
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    assert!(
        out.stderr
            .contains("command.program-not-found: Program \"aws\" of action \"upload\""),
        "{}",
        out.stderr
    );
    let strict = spore(&dir.path, &bare, &["build", "--strict", "nightly-backup"]);
    assert_eq!(strict.code(), Some(2), "{}", strict.stderr);
}

#[test]
fn fix_applies_only_safe_fixes_and_checks_again() {
    let dir = Dir::new("validate-fix");
    let path = programs(&dir, &["aws", "pg_dump"]);
    let folder = copy(&dir, "nightly-backup");
    let before = std::fs::read_to_string(folder.join("settings.json")).unwrap();
    let out = spore(&dir.path, &path, &["validate", "--fix", "nightly-backup"]);
    assert_eq!(out.code(), Some(2), "{}", out.stderr);
    let after = std::fs::read_to_string(folder.join("settings.json")).unwrap();
    assert_eq!(
        after,
        before.replace(
            "[\"pg_dump --format=custom --file=backup.dump shop\"]",
            "[\"pg_dump\", \"--format=custom\", \"--file=backup.dump\", \"shop\"]"
        ),
        "only the safe fix is applied"
    );
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.starts_with(
            "Fixed command.argv-spaces in nightly-backup/settings.json:\n  6 │-      \"argv\": [\"pg_dump --format=custom --file=backup.dump shop\"],\n  6 │+      \"argv\": [\"pg_dump\", \"--format=custom\", \"--file=backup.dump\", \"shop\"],\n"
        ),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .ends_with("\n4 problems in nightly-backup. Nothing was run.\n"),
        "{}",
        out.stderr
    );
    let change = out.stderr.split("\n\n").next().unwrap();
    assert!(guide().contains(change), "the guide shows another change");
    // Nothing safe is left: a second run changes nothing.
    let again = spore(
        &dir.path,
        &path,
        &["--json", "validate", "--fix", "nightly-backup"],
    );
    assert_eq!(again.json()["fixed"], json!([]));
    assert_eq!(
        std::fs::read_to_string(folder.join("settings.json")).unwrap(),
        after
    );
}

#[test]
fn explain_prints_the_page_of_every_code() {
    let dir = Dir::new("explain");
    for entry in &runspore_build::catalogue::catalogue().entries {
        let out = dir.run(&["explain", &entry.code]);
        assert_eq!(out.code(), Some(0), "{}: {}", entry.code, out.stderr);
        assert!(
            out.stdout
                .starts_with(&format!("{}: {}", entry.code, entry.title)),
            "{}",
            out.stdout
        );
    }
    let page = dir.run(&["explain", "action.no-effect"]).stdout;
    for wanted in [
        "action.no-effect: Missing \"effect\" (problem)",
        "\"unsafe\": running it twice could do harm",
        "For example, `spore validate nightly-backup` reports:",
        "── Missing \"effect\" ──",
        "The fix changes:",
        "+      \"effect\": \"idempotent\"",
    ] {
        assert!(page.contains(wanted), "no {wanted:?} in\n{page}");
    }
    let unknown = dir.run(&["--json", "explain", "action.no-efect"]);
    assert_eq!(unknown.code(), Some(2));
    assert_eq!(unknown.json()["error"]["code"], "explain.unknown-code");
    let all = dir.run(&["--json", "explain"]).json();
    assert_eq!(
        all["codes"].as_array().unwrap().len(),
        runspore_build::catalogue::catalogue().entries.len()
    );
}

#[test]
fn the_repositorys_own_workflows_have_no_problem() {
    let workflows = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.spore/workflows");
    let dir = Dir::new("validate-own");
    let mut seen = 0;
    for entry in std::fs::read_dir(&workflows).unwrap().flatten() {
        let folder = entry.path();
        if !folder.join("workflow.dot").is_file() {
            continue;
        }
        seen += 1;
        let out = dir.run(&["--json", "validate", folder.to_str().unwrap()]);
        let report = out.json();
        assert_eq!(report["problems"], "0", "{}: {report}", folder.display());
    }
    assert!(seen > 0);
}

#[test]
fn settings_may_name_their_schema() {
    let dir = Dir::new("validate-schema");
    let path = programs(&dir, &["aws", "pg_dump"]);
    let folder = warning_only(&dir);
    let settings = std::fs::read_to_string(folder.join("settings.json")).unwrap();
    let settings = settings
        .replacen(
            "{\n",
            &format!(
                "{{\n  \"$schema\": \"{}\",\n",
                runspore_build::settings::SCHEMA_ADDRESS
            ),
            1,
        )
        .replace(
            "[\"pg_dump --format=custom --file=backup.dump shop\"]",
            "[\"pg_dump\", \"shop\"]",
        );
    std::fs::write(folder.join("settings.json"), settings).unwrap();
    let out = spore(&dir.path, &path, &["validate", "nightly-backup"]);
    assert_eq!(out.code(), Some(0), "{}", out.stderr);
    assert_eq!(out.stdout, "nightly-backup is valid\n");
    assert_eq!(out.stderr, "");
}
