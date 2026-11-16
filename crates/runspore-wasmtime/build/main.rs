//! Builds the kernel component (`kernel.wasm`), the test-only echo component
//! (`echo.wasm`) and the test-only step runner (`test-runner.wasm`) into `OUT_DIR`.
//! See `pipeline.rs`. A published crate copies the first and last from `prebuilt/`.

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

/// The components a published crate carries in `prebuilt/`, made by
/// `scripts/stage-crates.sh` from the build of the release commit.
const PREBUILT: &[&str] = &["kernel", "test-runner"];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));

    // `cargo package` writes `.cargo_vcs_info.json` into the crates it makes, and a
    // published crate has no workspace to build the guests from. It carries their
    // bytes instead: the guest build is reproducible, so they are the bytes a build
    // from the tagged checkout makes (spec/release.md section 7).
    if manifest_dir.join(".cargo_vcs_info.json").exists() {
        println!("cargo:rerun-if-changed=prebuilt");
        for name in PREBUILT {
            let from = manifest_dir.join("prebuilt").join(format!("{name}.wasm"));
            std::fs::copy(&from, out_dir.join(format!("{name}.wasm"))).unwrap_or_else(|e| {
                panic!(
                    "copying {} (a published crate carries it): {e}",
                    from.display()
                )
            });
        }
        return;
    }

    let workspace = manifest_dir
        .ancestors()
        .nth(2)
        .expect("crate lives at <workspace>/crates/<name>")
        .to_path_buf();
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
