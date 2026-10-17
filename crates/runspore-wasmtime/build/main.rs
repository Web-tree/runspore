//! Builds the kernel component (`kernel.wasm`), the test-only echo component
//! (`echo.wasm`) and the test-only step runner (`test-runner.wasm`) into `OUT_DIR`.
//! See `pipeline.rs`.

use std::path::PathBuf;

#[path = "pipeline.rs"]
mod pipeline;

/// Inputs whose change must rebuild the embedded component, relative to the workspace.
const GUEST_INPUTS: &[&str] = &[
    "crates/runspore-component",
    "crates/runspore-kernel",
    "crates/runspore-types",
    "crates/runspore-test-runner",
    "contracts/wit/machine",
    "contracts/wit/runner",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .expect("crate lives at <workspace>/crates/<name>")
        .to_path_buf();
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));
    for input in GUEST_INPUTS {
        println!("cargo:rerun-if-changed={}", workspace.join(input).display());
    }
    println!("cargo:rerun-if-changed=build");

    let target_dir = out_dir.join("guest-target");
    for (name, package, features) in [
        ("kernel", "runspore-component", &[][..]),
        ("echo", "runspore-component", &["echo"][..]),
        ("test-runner", "runspore-test-runner", &[][..]),
    ] {
        let component = pipeline::build_guest(&workspace, &target_dir, package, features)
            .unwrap_or_else(|e| panic!("building the {name} component: {e}"));
        std::fs::write(out_dir.join(format!("{name}.wasm")), component)
            .unwrap_or_else(|e| panic!("writing the {name} component: {e}"));
    }
}
