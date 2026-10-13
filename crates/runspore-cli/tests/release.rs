//! The release cycle (spec/release.md section 8): release-please bumps every version
//! `spore --version` and `cargo --locked` read, and the PR title check refuses a title
//! that is not a conventional commit only when `PR_TITLE_STRICT` is true.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(file: &str) -> String {
    std::fs::read_to_string(repo().join(file)).unwrap()
}

/// The packages of this workspace in `Cargo.lock`: those with no `source`.
fn local_packages() -> Vec<String> {
    read("Cargo.lock")
        .split("[[package]]")
        .skip(1)
        .filter(|block| !block.lines().any(|l| l.starts_with("source = ")))
        .map(|block| {
            let name = block
                .lines()
                .find_map(|l| l.strip_prefix("name = "))
                .unwrap();
            name.trim_matches('"').to_string()
        })
        .collect()
}

#[test]
fn release_please_bumps_the_workspace_version_and_every_local_package_in_cargo_lock() {
    let config: serde_json::Value =
        serde_json::from_str(&read("release-please-config.json")).unwrap();
    let files = config["packages"]["."]["extra-files"].as_array().unwrap();
    let paths = |file: &str| -> Vec<String> {
        files
            .iter()
            .filter(|f| f["path"] == file)
            .map(|f| f["jsonpath"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(paths("Cargo.toml"), ["$.workspace.package.version"]);
    let mut want: Vec<String> = local_packages()
        .iter()
        .map(|name| format!("$.package[?(@.name.value === '{name}')].version"))
        .collect();
    let mut got = paths("Cargo.lock");
    want.sort();
    got.sort();
    assert!(want.len() > 1, "no workspace packages found in Cargo.lock");
    assert_eq!(
        got, want,
        "release-please-config.json must bump each workspace package in Cargo.lock"
    );
}

#[test]
fn the_manifest_version_is_the_workspace_version() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read(".release-please-manifest.json")).unwrap();
    assert_eq!(manifest["."], env!("CARGO_PKG_VERSION"));
}

fn check_title(title: &str, strict: bool) -> Output {
    let mut cmd = Command::new("sh");
    cmd.arg(repo().join("scripts/check-pr-title.sh")).arg(title);
    if strict {
        cmd.env("PR_TITLE_STRICT", "true");
    } else {
        cmd.env_remove("PR_TITLE_STRICT");
    }
    cmd.output().unwrap()
}

#[test]
fn conventional_titles_pass() {
    for title in [
        "feat(R4): workflow artifacts: spore build, load, pin and inspect",
        "fix: a lease that runs out twice",
        "feat!: drop the 0.1 store format",
        "refactor(cli)!: one flag parser",
        "chore(main): release 0.2.0",
        "revert: \"feat: x\"",
    ] {
        let out = check_title(title, true);
        assert!(
            out.status.success(),
            "{title}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn other_titles_fail_when_strict_and_only_warn_otherwise() {
    for title in [
        "R5: loop limits",
        "Feat: x",
        "feat:x",
        "feat(): x",
        "feature: x",
        "feat x",
        "",
    ] {
        let out = check_title(title, true);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{title:?} passed: {stdout}");
        assert!(stdout.starts_with("::error::"), "{stdout}");
        assert!(
            stdout.contains("'<type>: <subject>' or '<type>(<scope>): <subject>'"),
            "{stdout}"
        );

        let out = check_title(title, false);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{title:?} failed without PR_TITLE_STRICT: {stdout}"
        );
        assert!(stdout.starts_with("::warning::"), "{stdout}");
    }
}
