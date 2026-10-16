//! The version `spore --version` and `spore capabilities` print (spec/release.md
//! section 1). A build of the commit the tag `v<version>` names is that release and
//! prints `<version>`. Any other build is a pre-release of the next minor version,
//! named by its commit and dated by it, as rustc names a nightly:
//! `0.4.0-dev+a65d8d1 (2026-10-16)`. Without git it is `0.4.0-dev`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What `git <args>` prints in `dir`, trimmed, or `None` when it fails.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// `0.3.0` → `0.4.0`.
fn next_minor(version: &str) -> String {
    let mut parts = version.split('.').map(|p| p.parse::<u64>().unwrap());
    let (major, minor) = (parts.next().unwrap(), parts.next().unwrap());
    format!("{major}.{}.0", minor + 1)
}

/// Reruns this script when HEAD moves or a tag is made, so the version follows the
/// checkout.
fn watch(dir: &Path) {
    let path = |args: &[&str]| git(dir, args).map(|p| dir.join(p));
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(git_dir) = path(&["rev-parse", "--git-dir"]) {
        files.push(git_dir.join("HEAD"));
    }
    if let Some(common) = path(&["rev-parse", "--git-common-dir"]) {
        if let Some(head) = git(dir, &["symbolic-ref", "-q", "HEAD"]) {
            files.push(common.join(head));
        }
        files.push(common.join("packed-refs"));
        files.push(common.join("refs/tags"));
    }
    for file in files.iter().filter(|f| f.exists()) {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    watch(&dir);

    let release = git(&dir, &["tag", "--points-at", "HEAD"])
        .is_some_and(|tags| tags.lines().any(|t| t == format!("v{version}")));
    let commit = git(&dir, &["rev-parse", "--short=7", "HEAD"]);
    let date = git(&dir, &["log", "-1", "--format=%cs", "HEAD"]);
    let (semver, text) = match (release, commit, date) {
        (true, _, _) => (version.clone(), version),
        (false, Some(commit), Some(date)) => {
            let semver = format!("{}-dev+{commit}", next_minor(&version));
            let text = format!("{semver} ({date})");
            println!("cargo:rustc-env=SPORE_BUILD_DATE={date}");
            (semver, text)
        }
        _ => {
            let semver = format!("{}-dev", next_minor(&version));
            (semver.clone(), semver)
        }
    };
    println!("cargo:rustc-env=SPORE_BUILD_VERSION={semver}");
    println!("cargo:rustc-env=SPORE_BUILD_VERSION_TEXT={text}");
}
