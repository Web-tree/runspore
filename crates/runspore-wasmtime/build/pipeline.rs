//! Builds `runspore-component` for `wasm32-unknown-unknown` with a nested cargo
//! and turns the core module into a component with `wit-component`.
//!
//! Shared by the build script and the reproducibility test. The nested build runs
//! in release with the workspace profile, under `--locked`, with paths remapped so
//! that neither the checkout location, the cargo home nor the target directory
//! reaches the bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const GUEST_TARGET: &str = "wasm32-unknown-unknown";

/// Environment a parent cargo sets that must not leak into the nested build.
const SCRUBBED_ENV: &[&str] = &[
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_PRIMARY_PACKAGE",
];

/// Builds the guest with `features` into `target_dir` and returns component bytes.
pub fn build_component(
    workspace: &Path,
    target_dir: &Path,
    features: &[&str],
) -> Result<Vec<u8>, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let flags = [
        remap(workspace, "/runspore"),
        remap(&cargo_home(), "/cargo"),
        remap(target_dir, "/target"),
    ]
    .join("\x1f");
    let mut command = Command::new(cargo);
    command
        .current_dir(workspace)
        .args(["build", "--locked", "--release", "-p", "runspore-component"])
        .args(["--target", GUEST_TARGET])
        .arg("--target-dir")
        .arg(target_dir);
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    for name in SCRUBBED_ENV {
        command.env_remove(name);
    }
    command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    let status = command
        .status()
        .map_err(|e| format!("spawning cargo for the guest: {e}"))?;
    if !status.success() {
        return Err(format!("guest build failed: {status}"));
    }
    let module_path = target_dir
        .join(GUEST_TARGET)
        .join("release")
        .join("runspore_component.wasm");
    let module = std::fs::read(&module_path)
        .map_err(|e| format!("reading {}: {e}", module_path.display()))?;
    componentise(&module)
}

/// Wraps a core module carrying its embedded WIT into a validated component.
pub fn componentise(module: &[u8]) -> Result<Vec<u8>, String> {
    wit_component::ComponentEncoder::default()
        .validate(true)
        .module(module)
        .and_then(|encoder| encoder.encode())
        .map_err(|e| format!("componentising the guest: {e:#}"))
}

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .unwrap_or_default()
}

fn remap(from: &Path, to: &str) -> String {
    format!("--remap-path-prefix={}={to}", from.display())
}
