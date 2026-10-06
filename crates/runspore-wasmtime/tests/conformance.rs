//! Host conformance: the kernel component through Wasmtime returns exactly
//! what the native kernel returns, on the golden traces and on a generated
//! corpus. Failures are compared in full, `Failure.details` included, which
//! the trace runner itself does not compare.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use runspore_kernel::NativeReducer;
use runspore_trace::compile::CompiledExpect;
use runspore_trace::generate::DEFAULT_MAX_STEPS;
use runspore_trace::{compile, generate, load, run_compiled, run_dir, CompiledTrace};
use runspore_types::reducer::{Decision, Envelope, Failure, Reducer, TransitionRequest};
use runspore_wasmtime::WasmtimeReducer;

/// Seeds of the differential corpus: 20,000 traces, about 170,000 steps.
const CORPUS: Range<u64> = 0..20_000;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/traces")
}

/// The replay loop of `runspore_trace::compile`, returning every result: the
/// snapshot of a step is its own `snapshot` if present, else the snapshot of
/// this reducer's last decision.
fn replay(reducer: &dyn Reducer, trace: &CompiledTrace) -> Vec<Result<Decision, Failure>> {
    let mut snapshot: Option<Vec<u8>> = None;
    let mut results = Vec::with_capacity(trace.steps.len());
    for step in &trace.steps {
        let event = &step.event;
        let request = TransitionRequest {
            identity: step.identity.as_ref().unwrap_or(&trace.identity).into(),
            graph: trace.workflow.as_bytes().to_vec(),
            snapshot: step
                .snapshot
                .as_ref()
                .map(|s| s.as_bytes().to_vec())
                .or_else(|| snapshot.clone()),
            input_event: Envelope {
                event_id: event.event_id.clone(),
                sequence: event.sequence,
                accepted_at_ms: event.accepted_at_ms,
                kind: event.kind.clone(),
                payload: event.payload.as_bytes().to_vec(),
            },
            frozen_limits: trace.limits.into(),
        };
        let result = reducer.transition(&request);
        if let Ok(decision) = &result {
            snapshot = Some(decision.snapshot.clone());
        }
        results.push(result);
    }
    results
}

/// How a step's two results differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Divergence {
    /// Decisions differ, or one side failed, or failure kinds or codes differ.
    Result,
    /// Both failed with the same kind and code; only `details` differs.
    DetailsOnly,
}

#[derive(Debug)]
struct Finding {
    trace: String,
    step: usize,
    divergence: Divergence,
    native: String,
    wasmtime: String,
}

fn show(result: &Result<Decision, Failure>) -> String {
    match result {
        Ok(decision) => format!(
            "decision snapshot={} digest={} commands={:?} diagnostics={:?}",
            String::from_utf8_lossy(&decision.snapshot),
            decision.snapshot_digest,
            decision
                .commands
                .iter()
                .map(|c| (
                    &c.command_id,
                    &c.activation_id,
                    &c.kind,
                    String::from_utf8_lossy(&c.payload)
                ))
                .collect::<Vec<_>>(),
            decision
                .diagnostics
                .iter()
                .map(|d| (&d.code, &d.node_id, String::from_utf8_lossy(&d.details)))
                .collect::<Vec<_>>(),
        ),
        Err(failure) => format!(
            "failure kind={:?} code={} details={:?}",
            failure.kind, failure.code, failure.details
        ),
    }
}

/// The first step at which the two hosts differ, if any.
fn first_divergence(wasmtime: &dyn Reducer, trace: &CompiledTrace) -> Option<Finding> {
    let native = replay(&NativeReducer, trace);
    let guest = replay(wasmtime, trace);
    assert_eq!(native.len(), guest.len());
    native
        .iter()
        .zip(&guest)
        .enumerate()
        .find(|(_, (n, w))| n != w)
        .map(|(step, (n, w))| {
            let divergence = match (n, w) {
                (Err(a), Err(b)) if a.kind == b.kind && a.code == b.code => Divergence::DetailsOnly,
                _ => Divergence::Result,
            };
            Finding {
                trace: trace.name.clone(),
                step,
                divergence,
                native: show(n),
                wasmtime: show(w),
            }
        })
}

/// Fails with every finding, smallest reproducing trace first, result
/// divergences listed apart from details-only ones.
fn assert_no_findings(mut findings: Vec<Finding>) {
    if findings.is_empty() {
        return;
    }
    findings.sort_by_key(|f| (f.divergence == Divergence::DetailsOnly, f.step));
    let mut report = String::new();
    for kind in [Divergence::Result, Divergence::DetailsOnly] {
        let of_kind: Vec<&Finding> = findings.iter().filter(|f| f.divergence == kind).collect();
        report.push_str(&format!("{kind:?}: {} trace(s)\n", of_kind.len()));
        for f in of_kind.iter().take(5) {
            report.push_str(&format!(
                "  {} step {}\n    native:   {}\n    wasmtime: {}\n",
                f.trace, f.step, f.native, f.wasmtime
            ));
        }
    }
    panic!("native and Wasmtime diverge\n{report}");
}

#[test]
fn every_golden_trace_passes_through_wasmtime() {
    let wasmtime = WasmtimeReducer::new().expect("kernel compiles");
    let count = run_dir(&wasmtime, golden_dir()).unwrap_or_else(|e| panic!("{e}"));
    assert!(count > 0, "no golden traces in {}", golden_dir().display());
}

#[test]
fn golden_traces_are_byte_identical_including_failure_details() {
    let wasmtime = WasmtimeReducer::new().expect("kernel compiles");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(golden_dir())
        .expect("golden directory")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    let findings = paths
        .iter()
        .filter_map(|path| {
            let trace = load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            first_divergence(&wasmtime, &compile(&trace).expect("golden trace compiles"))
        })
        .collect();
    assert_no_findings(findings);
}

/// Generates each trace with `NativeReducer`, replays it through both hosts
/// independently, and requires identical results at every step. Also replays
/// it through the trace runner so digests are checked the runner's way.
#[test]
fn generated_corpus_is_byte_identical_native_and_wasmtime() {
    let wasmtime = WasmtimeReducer::new().expect("kernel compiles");
    let next = AtomicU64::new(CORPUS.start);
    let steps = AtomicU64::new(0);
    let refused = AtomicU64::new(0);
    let findings = Mutex::new(Vec::new());
    let work = || loop {
        let seed = next.fetch_add(1, Ordering::Relaxed);
        if seed >= CORPUS.end {
            return;
        }
        let trace = generate(seed, &NativeReducer, DEFAULT_MAX_STEPS)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let compiled = compile(&trace).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        steps.fetch_add(compiled.steps.len() as u64, Ordering::Relaxed);
        refused.fetch_add(
            compiled
                .steps
                .iter()
                .filter(|s| matches!(s.expect, CompiledExpect::Failure(_)))
                .count() as u64,
            Ordering::Relaxed,
        );
        if let Some(finding) = first_divergence(&wasmtime, &compiled) {
            findings.lock().expect("lock").push(finding);
        } else if let Err(error) = run_compiled(&wasmtime, &compiled) {
            panic!("seed {seed}: trace runner disagrees: {error}");
        }
    };
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(work);
        }
    });
    assert_no_findings(findings.into_inner().expect("lock"));
    let steps = steps.into_inner();
    let refused = refused.into_inner();
    println!(
        "differential corpus: {} traces, {steps} steps ({refused} refused) identical",
        CORPUS.end - CORPUS.start
    );
    assert!(steps >= 5 * (CORPUS.end - CORPUS.start));
    assert!(refused > 0, "the corpus never exercised a failure");
}

/// The native kernel with one alteration applied to every result, to show the
/// comparison classifies what it is meant to catch.
struct Altered(fn(&mut Result<Decision, Failure>));

impl Reducer for Altered {
    fn describe(&self) -> runspore_types::reducer::Descriptor {
        NativeReducer.describe()
    }

    fn kernel_digest(&self) -> String {
        NativeReducer.kernel_digest()
    }

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        let mut result = NativeReducer.transition(request);
        (self.0)(&mut result);
        result
    }
}

#[test]
fn the_comparison_separates_result_and_details_divergences() {
    let trace = (0..)
        .map(|seed| compile(&generate(seed, &NativeReducer, DEFAULT_MAX_STEPS).unwrap()).unwrap())
        .find(|t| {
            t.steps
                .iter()
                .any(|s| matches!(s.expect, CompiledExpect::Failure(_)))
        })
        .expect("some seed refuses an event");

    assert!(first_divergence(&NativeReducer, &trace).is_none());

    let details = Altered(|result| {
        if let Err(failure) = result {
            failure.details.push('.');
        }
    });
    let finding = first_divergence(&details, &trace).expect("details differ");
    assert_eq!(finding.divergence, Divergence::DetailsOnly);

    let snapshot = Altered(|result| {
        if let Ok(decision) = result {
            decision.snapshot.push(b' ');
        }
    });
    let finding = first_divergence(&snapshot, &trace).expect("snapshots differ");
    assert_eq!((finding.divergence, finding.step), (Divergence::Result, 0));
}
