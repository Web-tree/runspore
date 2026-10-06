//! Writes everything the JavaScript conformance runners need into one directory:
//!
//! - `component.wasm`: the kernel component embedded in `runspore-wasmtime`;
//! - `traces/golden/<name>.json`: every `*.json` trace of the golden directory,
//!   compiled with `runspore_trace::compile`;
//! - `traces/generated/<name>.json`: the traces of seeds `GENERATED_SEEDS`,
//!   generated against `NativeReducer` and compiled the same way;
//! - `manifest.json`: the component's SHA-256 and kernel digest, the trace
//!   files in replay order, and the trace and step counts.
//!
//! Before writing, every exported trace is replayed natively and through
//! Wasmtime with `run_compiled`, so the counts printed for those two hosts are
//! for exactly the set the JavaScript runners replay.
//!
//! `cargo run --release -p runspore-conformance-export -- <golden-dir> <out-dir>`

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use runspore_kernel::NativeReducer;
use runspore_trace::generate::DEFAULT_MAX_STEPS;
use runspore_trace::{compile, generate, load, run_compiled, CompiledTrace};
use runspore_types::digest;
use runspore_types::reducer::Reducer;
use runspore_wasmtime::{WasmtimeReducer, COMPONENT};
use sha2::{Digest, Sha256};

/// Fixed seeds of the generated corpus. Changing them changes the corpus.
const GENERATED_SEEDS: std::ops::Range<u64> = 100_000..100_500;

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [golden, out] = args.as_slice() else {
        eprintln!("usage: runspore-conformance-export <golden-dir> <out-dir>");
        return ExitCode::from(2);
    };
    match export(golden, out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("export: {message}");
            ExitCode::FAILURE
        }
    }
}

fn export(golden: &Path, out: &Path) -> Result<(), String> {
    let mut traces: Vec<(String, CompiledTrace)> = Vec::new();
    for path in golden_files(golden)? {
        let trace = load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let compiled = compile(&trace).map_err(|e| format!("{}: {e}", path.display()))?;
        traces.push((format!("traces/golden/{stem}.json"), compiled));
    }
    let golden_count = traces.len();
    if golden_count == 0 {
        return Err(format!("no *.json traces in {}", golden.display()));
    }
    for seed in GENERATED_SEEDS {
        let trace = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS)
            .map_err(|e| format!("seed {seed}: {e}"))?;
        let compiled = compile(&trace).map_err(|e| format!("seed {seed}: {e}"))?;
        traces.push((format!("traces/generated/{}.json", trace.name), compiled));
    }
    let steps: usize = traces.iter().map(|(_, trace)| trace.steps.len()).sum();

    let wasmtime = WasmtimeReducer::new().map_err(|e| format!("wasmtime: {e}"))?;
    let hosts: [(&str, &dyn Reducer); 2] = [("native", &NativeReducer), ("wasmtime", &wasmtime)];
    for (host, reducer) in hosts {
        for (file, trace) in &traces {
            run_compiled(reducer, trace).map_err(|e| format!("{host} {file}: {e}"))?;
        }
    }

    let component_sha256 = format!("sha256:{}", hex::encode(Sha256::digest(COMPONENT)));
    let kernel_digest = digest::hash("kernel", &[COMPONENT]).to_string();
    if out.exists() {
        std::fs::remove_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    write(&out.join("component.wasm"), COMPONENT)?;
    for (file, trace) in &traces {
        let text = serde_json::to_string(trace).map_err(|e| format!("{file}: {e}"))?;
        write(&out.join(file), text.as_bytes())?;
    }
    let manifest = serde_json::json!({
        "componentSha256": component_sha256,
        "kernelDigest": kernel_digest,
        "traces": traces.iter().map(|(file, _)| file).collect::<Vec<_>>(),
        "traceCount": traces.len(),
        "stepCount": steps,
    });
    let manifest = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    write(&out.join("manifest.json"), manifest.as_bytes())?;

    let generated = traces.len() - golden_count;
    for (host, _) in hosts {
        println!(
            "{host}: {} traces ({golden_count} golden, {generated} generated), {steps} steps passed; component {component_sha256}",
            traces.len()
        );
    }
    println!("wrote {}", out.display());
    Ok(())
}

/// The `*.json` files directly inside `dir`, in file name order.
fn golden_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.extension().is_some_and(|ext| ext == "json") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
