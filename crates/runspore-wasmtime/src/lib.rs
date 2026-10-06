//! `Reducer` implemented by running the kernel component through Wasmtime.
//!
//! The component is compiled once per reducer. Every transition instantiates it in a
//! fresh `Store`, so no guest memory survives between calls. A trap, a fuel stop or a
//! refused memory growth becomes an `InvariantViolation` failure with code
//! `host.trap`, `host.watchdog` or `host.memory`; it never panics the host and is never
//! a business outcome. Rules: `spec/host.md` section 1.

mod convert;

use runspore_types::digest;
use runspore_types::reducer::{
    Decision, Descriptor, Failure, FailureKind, Reducer, TransitionRequest,
};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap, WasmBacktraceDetails};

wasmtime::component::bindgen!({
    path: "../../contracts/wit/machine",
    world: "machine",
});

/// The kernel component, built from this checkout by the build script.
pub const COMPONENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kernel.wasm"));

/// Per-transition resource bounds enforced by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guards {
    /// Wasmtime fuel granted to one transition; roughly one unit per Wasm instruction.
    /// Running out is `host.watchdog`. Fuel is counted, not timed, so the same request
    /// stops at the same point on every machine.
    pub fuel: u64,
    /// Ceiling on the guest's linear memory in bytes. Refused growth is `host.memory`.
    pub max_memory_bytes: usize,
}

impl Default for Guards {
    /// Generous bounds that a kernel within its frozen `Limits` never reaches.
    fn default() -> Self {
        Self {
            fuel: 2_000_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Runs a `machine` component; `Send + Sync` and callable concurrently.
pub struct WasmtimeReducer {
    pre: MachinePre<Bounds>,
    engine: Engine,
    component_bytes: Vec<u8>,
    kernel_digest: String,
    descriptor: Descriptor,
    guards: Guards,
}

impl WasmtimeReducer {
    /// The embedded kernel component with default guards.
    pub fn new() -> wasmtime::Result<Self> {
        Self::from_component(COMPONENT, Guards::default())
    }

    /// Compiles `component_bytes` once and calls its `describe` export, which is cached.
    pub fn from_component(component_bytes: &[u8], guards: Guards) -> wasmtime::Result<Self> {
        let engine = Engine::new(&deterministic_config())?;
        let component = Component::new(&engine, component_bytes)?;
        let linker = Linker::<Bounds>::new(&engine);
        let pre = MachinePre::new(linker.instantiate_pre(&component)?)?;
        let mut store = new_store(&engine, guards)?;
        let descriptor = pre
            .instantiate(&mut store)?
            .runspore_machine_reducer()
            .call_describe(&mut store)?;
        Ok(Self {
            pre,
            engine,
            component_bytes: component_bytes.to_vec(),
            kernel_digest: digest::hash("kernel", &[component_bytes]),
            descriptor: convert::descriptor_from_wit(descriptor),
            guards,
        })
    }

    /// The exact component bytes this reducer executes.
    pub fn component_bytes(&self) -> &[u8] {
        &self.component_bytes
    }

    fn run(
        &self,
        store: &mut Store<Bounds>,
        request: &TransitionRequest,
    ) -> wasmtime::Result<Result<Decision, Failure>> {
        let machine = self.pre.instantiate(&mut *store)?;
        let result = machine
            .runspore_machine_reducer()
            .call_transition(&mut *store, &convert::request_to_wit(request))?;
        Ok(result
            .map(convert::decision_from_wit)
            .map_err(convert::failure_from_wit))
    }
}

impl Reducer for WasmtimeReducer {
    fn describe(&self) -> Descriptor {
        self.descriptor.clone()
    }

    fn kernel_digest(&self) -> String {
        self.kernel_digest.clone()
    }

    fn transition(&self, request: &TransitionRequest) -> Result<Decision, Failure> {
        let mut store =
            new_store(&self.engine, self.guards).map_err(|e| host_failure("host.trap", &e))?;
        self.run(&mut store, request).unwrap_or_else(|error| {
            let code = if store.data().refused {
                "host.memory"
            } else if matches!(error.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) {
                "host.watchdog"
            } else {
                "host.trap"
            };
            Err(host_failure(code, &error))
        })
    }
}

/// Engine settings that remove every Wasmtime nondeterminism source the kernel does
/// not need. Threads are absent because the `threads` cargo feature is not compiled
/// in. See the crate README for the reason behind each setting.
fn deterministic_config() -> Config {
    let mut config = Config::new();
    config
        .wasm_simd(false)
        .wasm_relaxed_simd(false)
        .relaxed_simd_deterministic(true)
        .wasm_multi_memory(false)
        .wasm_memory64(false)
        .cranelift_nan_canonicalization(true)
        .wasm_backtrace_details(WasmBacktraceDetails::Disable)
        .consume_fuel(true);
    config
}

/// A fresh store for one instance: its own memory ceiling, refusal flag and fuel.
fn new_store(engine: &Engine, guards: Guards) -> wasmtime::Result<Store<Bounds>> {
    let mut store = Store::new(
        engine,
        Bounds {
            max_memory_bytes: guards.max_memory_bytes,
            refused: false,
        },
    );
    store.limiter(|bounds| bounds);
    store.set_fuel(guards.fuel)?;
    Ok(store)
}

fn host_failure(code: &str, error: &wasmtime::Error) -> Failure {
    Failure {
        kind: FailureKind::InvariantViolation,
        code: code.to_string(),
        details: format!("{error:#}"),
    }
}

/// Store data: the memory ceiling and whether the guest was ever refused growth.
struct Bounds {
    max_memory_bytes: usize,
    refused: bool,
}

impl ResourceLimiter for Bounds {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let allowed = desired <= self.max_memory_bytes;
        self.refused |= !allowed;
        Ok(allowed)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 10_000)
    }
}
