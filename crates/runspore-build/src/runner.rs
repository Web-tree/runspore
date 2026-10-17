//! A `component` action's step runner, checked as the host loads it (spec/host.md §3,
//! `component`): a component that imports only the `host` interface of world
//! `runspore:runner/runner@0.1.0` and exports its `step` interface. The host also
//! checks every function's type when it loads the runner; the build checks the names.

use wasmparser::{Encoding, Parser, Payload, Validator, WasmFeatures};

/// The one interface a runner may import.
pub const HOST_IMPORT: &str = "runspore:runner/host@0.1.0";
/// The interface a runner exports.
pub const STEP_EXPORT: &str = "runspore:runner/step@0.1.0";

/// Why a file is not a step runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Not a valid component, or one without the `step` export.
    Invalid(String),
    /// It imports something outside the world.
    Import(String),
}

/// Checks `bytes` as a step runner.
pub fn check(bytes: &[u8]) -> Result<(), Problem> {
    if !bytes.starts_with(b"\0asm") {
        return Err(Problem::Invalid("it is not a WebAssembly file".into()));
    }
    let mut depth = 0usize;
    let mut exports = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|e| Problem::Invalid(e.message().to_string()))?;
        match payload {
            Payload::Version { encoding, .. } if depth == 0 => {
                if encoding != Encoding::Component {
                    return Err(Problem::Invalid(
                        "it is a core WebAssembly module, not a component".into(),
                    ));
                }
            }
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(imports) if depth == 0 => {
                for import in imports {
                    let import = import.map_err(|e| Problem::Invalid(e.message().to_string()))?;
                    if import.name.name != HOST_IMPORT {
                        return Err(Problem::Import(import.name.name.to_string()));
                    }
                }
            }
            Payload::ComponentExportSection(list) if depth == 0 => {
                for export in list {
                    let export = export.map_err(|e| Problem::Invalid(e.message().to_string()))?;
                    exports.push(export.name.name.to_string());
                }
            }
            _ => {}
        }
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(bytes)
        .map_err(|e| Problem::Invalid(e.message().to_string()))?;
    if !exports.iter().any(|e| e == STEP_EXPORT) {
        return Err(Problem::Invalid(format!(
            "it does not export `{STEP_EXPORT}`"
        )));
    }
    Ok(())
}
