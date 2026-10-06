//! Properties of the embedded kernel component bytes.

use std::path::Path;

use runspore_wasmtime::COMPONENT;
use wasmparser::{Encoding, Parser, Payload};

#[path = "../build/pipeline.rs"]
mod pipeline;

/// Top-level import and export names of a component, ignoring nested modules.
fn top_level_names(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut depth = 0usize;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.expect("component parses") {
            Payload::Version { encoding, .. } => {
                depth += 1;
                if depth == 1 {
                    assert_eq!(encoding, Encoding::Component, "artifact is a component");
                }
            }
            Payload::End(_) => depth -= 1,
            Payload::ComponentImportSection(section) if depth == 1 => {
                for import in section {
                    imports.push(import.unwrap().name.full_name().into_owned());
                }
            }
            Payload::ComponentExportSection(section) if depth == 1 => {
                for export in section {
                    exports.push(export.unwrap().name.full_name().into_owned());
                }
            }
            _ => {}
        }
    }
    (imports, exports)
}

#[test]
fn component_has_no_imports_and_exports_only_the_reducer() {
    let (imports, exports) = top_level_names(COMPONENT);
    assert_eq!(imports, Vec::<String>::new());
    assert_eq!(exports, vec!["runspore:machine/reducer@0.1.0".to_string()]);
}

/// Rebuilds the kernel component from scratch into a different target directory
/// and requires the same bytes as the embedded one.
#[test]
fn component_build_is_reproducible() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <workspace>/crates/<name>");
    let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reproducibility");
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).expect("previous run's directory is removable");
    }
    let rebuilt = pipeline::build_component(workspace, &target_dir, &[]).expect("guest builds");
    assert_eq!(
        runspore_types::digest::hash("kernel", &[&rebuilt]),
        runspore_types::digest::hash("kernel", &[COMPONENT])
    );
}
