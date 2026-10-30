//! The `component` kind (spec/host.md §3, `component`): an action whose start, probe
//! and stop are a step runner, a WebAssembly component of world
//! `runspore:runner/runner@0.1.1` sealed into the workflow artifact. The host runs it
//! through Wasmtime in the kernel's sandbox, a fresh instance per call, and offers it
//! only the imports of `contracts/wit/runner/runner.wit`, each checked against the
//! action's grants.

mod grants;
mod imports;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use runspore_types::artifact::{Artifact, EntryKind, Owner};
use runspore_types::canonical;
use runspore_types::capabilities::RUNNER_WORLD;
use runspore_types::digest;
use runspore_types::failpoint;
use runspore_types::store::MAX_HOLDER_BYTES;
use serde_json::{json, Value};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Engine, Store, Trap};

use crate::activity::{ActivityContext, ActivityOutput, ActivityRunner};
use crate::error::HostError;
use crate::holder::Answer;
use exports::runspore::runner::step;
pub use grants::KINDS as GRANT_KINDS;
use grants::{Grants, Places, Refused};
use imports::{Halt, State};
use runspore_wasmtime::ArtifactReader;

wasmtime::component::bindgen!({
    path: "../../contracts/wit/runner",
    world: "runner",
    imports: { default: trappable },
});

/// The kind of an action whose start, probe and stop are a step runner.
pub const KIND: &str = runspore_types::capabilities::COMPONENT_KIND;
/// How long `start` may run when the action gives no `timeoutMs`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a probe or a stop may run.
const LOOK_TIMEOUT: Duration = Duration::from_secs(10);
/// Wasmtime fuel for one call, as the kernel's: counted, not timed.
const FUEL: u64 = 2_000_000_000;
/// The ceiling on a runner's linear memory.
const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// A `component` action's fields the host acts on.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// The runner's entry path among the action's assets.
    pub module: String,
    grants: Grants,
    timeout: Duration,
    /// Whether the programs `start` starts get the attempt's token.
    pass_token: bool,
}

/// The entry path a `module` names: relative, `/` its separator, no `..`; `.` steps
/// left out.
fn entry_path(module: &str) -> Option<String> {
    if module.is_empty() || module.starts_with('/') || module.contains('\\') {
        return None;
    }
    let parts: Vec<&str> = module
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    (!parts.is_empty() && !parts.contains(&"..")).then(|| parts.join("/"))
}

/// Reads a `component` action. A grant of a kind this host does not offer is
/// `component.grant-unsupported`; any other bad field is `action.invalid`.
pub(crate) fn parse(action_id: &str, action: &Value) -> Result<Spec, HostError> {
    read(action).map_err(|(code, e)| HostError::invalid(code, format!("action `{action_id}`: {e}")))
}

/// [`parse`], its refusal as a code and a message that does not name the action.
pub(crate) fn read(action: &Value) -> Result<Spec, (&'static str, String)> {
    let invalid = |e: String| ("action.invalid", e);
    let module = action
        .get("module")
        .and_then(Value::as_str)
        .and_then(entry_path)
        .ok_or_else(|| invalid("`module` must be a relative path inside the folder".into()))?;
    let grants = Grants::parse(action.get("grants")).map_err(|refused| match refused {
        Refused::Unsupported(kind) => (
            "component.grant-unsupported",
            format!(
                "this host offers no grant `{kind}`; it offers {}",
                GRANT_KINDS.map(|k| format!("`{k}`")).join(", ")
            ),
        ),
        Refused::Invalid(e) => invalid(e),
    })?;
    let timeout = crate::external::millis(action, "timeoutMs", "timeoutMs")
        .map_err(invalid)?
        .map_or(START_TIMEOUT, Duration::from_millis);
    let pass_token = match action.get("passToken") {
        None => true,
        Some(Value::Bool(pass)) => *pass,
        Some(_) => return Err(invalid("`passToken` must be true or false".into())),
    };
    Ok(Spec {
        module,
        grants,
        timeout,
        pass_token,
    })
}

/// The one engine every runner of this process is compiled for.
fn engine() -> Result<&'static Engine, String> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            Engine::new(&runspore_wasmtime::deterministic_config()).map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// A runner compiled and linked, ready to instantiate.
type Loaded = RunnerPre<State>;

/// Compiles `bytes` as a component of world `runner` and links it to the host, once per
/// process by digest. The error is why it is not a runner: not a component, an import
/// outside `host`, an export missing.
pub(crate) fn load(bytes: &[u8]) -> Result<Loaded, String> {
    static LOADED: OnceLock<Mutex<HashMap<String, Loaded>>> = OnceLock::new();
    let key = digest::blob(bytes);
    let cache = LOADED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(pre) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(pre.clone());
    }
    let engine = engine()?;
    let component = Component::new(engine, bytes).map_err(|e| format!("{e:#}"))?;
    let mut linker = Linker::<State>::new(engine);
    Runner::add_to_linker::<State, HasSelf<State>>(&mut linker, |state| state)
        .map_err(|e| format!("{e:#}"))?;
    let pre = linker
        .instantiate_pre(&component)
        .map_err(|e| format!("{e:#}"))?;
    let pre = RunnerPre::new(pre).map_err(|e| format!("{e:#}"))?;
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, pre.clone());
    Ok(pre)
}

/// The checks `start` makes of the `component` actions of `workflow` (spec/host.md §3,
/// `component`): without an artifact, none can find its runner
/// (`component.no-artifact`); with one, each runner is an `asset` entry of its action
/// (`component.module-missing`) that loads as a component of world `runner`
/// (`component.invalid`).
pub(crate) fn check_runners(workflow: &[u8], artifact: Option<&Artifact>) -> Result<(), HostError> {
    let Ok(workflow) = canonical::parse(workflow) else {
        // Not this check's to refuse: the workflow's own checks name it.
        return Ok(());
    };
    let actions = workflow.get("actions").and_then(Value::as_object);
    let runners = actions
        .into_iter()
        .flatten()
        .filter(|(_, action)| action.get("kind").and_then(Value::as_str) == Some(KIND));
    let mut reader = None;
    for (action_id, action) in runners {
        let spec = parse(action_id, action)?;
        let Some(artifact) = artifact else {
            return Err(HostError::invalid(
                "component.no-artifact",
                format!(
                    "action `{action_id}` runs the step runner `{}`, which only a workflow \
                     artifact holds: start the workflow folder or its artifact",
                    spec.module
                ),
            ));
        };
        let entry = artifact.manifest.entries.iter().find(|e| {
            e.kind == EntryKind::Asset
                && e.path == spec.module
                && e.owner == Some(Owner::Action(action_id.clone()))
        });
        let Some(entry) = entry else {
            return Err(HostError::invalid(
                "component.module-missing",
                format!(
                    "action `{action_id}`: the artifact holds no runner `{}` of the action",
                    spec.module
                ),
            ));
        };
        let reader = match &mut reader {
            Some(reader) => reader,
            None => reader.insert(
                ArtifactReader::open(&artifact.bytes)
                    .map_err(|e| HostError::invalid(e.code, e.message))?,
            ),
        };
        let bytes = reader
            .entry(&entry.digest)
            .map_err(|e| HostError::invalid(e.code, e.message))?;
        load(&bytes).map_err(|e| {
            HostError::invalid(
                "component.invalid",
                format!(
                    "action `{action_id}`: `{}` is not a step runner of world `{RUNNER_WORLD}`: {e}",
                    spec.module
                ),
            )
        })?;
    }
    Ok(())
}

/// How a call ended when it did not return.
#[derive(Debug, Clone, PartialEq)]
enum Ended {
    /// A host call outside the grants.
    Denied(grants::Denied),
    /// Any other ending, by its code: `component.trap`, `component.watchdog`,
    /// `component.memory`, `component.timeout`, `component.stopped`,
    /// `component.module-missing` or `component.invalid`.
    Code(&'static str, String),
}

impl Ended {
    fn message(&self, action_id: &str) -> String {
        match self {
            Self::Denied(denied) => denied.message(action_id),
            Self::Code(_, message) => message.clone(),
        }
    }
}

/// The runner's bytes, from the step files' copy of its entry.
fn module_bytes(ctx: &ActivityContext, spec: &Spec) -> Result<Vec<u8>, Ended> {
    let missing = |why: String| Ended::Code("component.module-missing", why);
    let files = ctx.files.as_ref().ok_or_else(|| {
        missing(format!(
            "the step has no step files, so its runner `{}` cannot be read",
            spec.module
        ))
    })?;
    if !files.assets.contains(&spec.module) {
        return Err(missing(format!(
            "the step files hold no runner `{}`",
            spec.module
        )));
    }
    std::fs::read(files.dir.join(&spec.module))
        .map_err(|e| missing(format!("reading the runner `{}`: {e}", spec.module)))
}

/// The `context` record of the try `ctx` names.
fn context(ctx: &ActivityContext, input: &Value, start: bool) -> step::Context {
    let json =
        |v: &Value| String::from_utf8(canonical::to_vec(v).unwrap_or_default()).unwrap_or_default();
    let path = |p: Option<&PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    step::Context {
        run_id: ctx.key.run.clone(),
        tenant: ctx.key.tenant.clone(),
        node_id: ctx.node_id.clone(),
        action_id: ctx.action_id.clone(),
        invocation_id: ctx.invocation_id.clone(),
        attempt: ctx.attempt,
        visit: ctx.visit,
        effect_key: ctx.effect_key.clone(),
        input: json(input),
        params: json(&ctx.params),
        base_dir: ctx.base_dir.to_string_lossy().into_owned(),
        work_dir: path(ctx.work_dir.as_ref()),
        step_files: path(ctx.files.as_ref().map(|f| &f.dir)),
        token: ctx.token.clone().filter(|_| start),
        result_channel: ctx.result_channel.clone(),
    }
}

/// Runs one export of the runner on a blocking thread, in a fresh store with its own
/// fuel, memory ceiling and deadline.
async fn call<T: Send + 'static>(
    ctx: &ActivityContext,
    spec: &Spec,
    start: bool,
    timeout: Duration,
    f: impl FnOnce(&mut Store<State>, &Runner) -> wasmtime::Result<T> + Send + 'static,
) -> Result<T, Ended> {
    let (ctx, spec) = (ctx.clone(), spec.clone());
    let joined = tokio::task::spawn_blocking(move || {
        let bytes = module_bytes(&ctx, &spec)?;
        let pre = load(&bytes).map_err(|e| Ended::Code("component.invalid", e))?;
        let places = Places {
            base: ctx.base_dir.clone(),
            home: std::env::var_os("HOME").map(PathBuf::from),
            work: ctx.work_dir.clone(),
        };
        let state = State {
            ctx,
            grants: spec.grants.clone(),
            places,
            deadline: Instant::now() + timeout,
            start,
            pass_token: spec.pass_token,
            halt: None,
            max_memory_bytes: MAX_MEMORY_BYTES,
            refused: false,
        };
        let mut store = Store::new(pre.engine(), state);
        store.limiter(|state| state);
        store
            .set_fuel(FUEL)
            .map_err(|e| Ended::Code("component.trap", format!("{e:#}")))?;
        let called = pre
            .instantiate(&mut store)
            .and_then(|runner| f(&mut store, &runner));
        called.map_err(|error| {
            let data = store.data();
            match &data.halt {
                Some(Halt::Denied(denied)) => Ended::Denied(denied.clone()),
                Some(Halt::Timeout) => Ended::Code(
                    "component.timeout",
                    format!("the runner ran past its {} ms", timeout.as_millis()),
                ),
                Some(Halt::Stopped) => Ended::Code(
                    "component.stopped",
                    "the attempt was told to stop while the runner ran".into(),
                ),
                None if data.refused => Ended::Code("component.memory", format!("{error:#}")),
                None if matches!(error.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) => {
                    Ended::Code("component.watchdog", format!("{error:#}"))
                }
                None => Ended::Code("component.trap", format!("{error:#}")),
            }
        })
    })
    .await;
    joined.unwrap_or_else(|e| Err(Ended::Code("component.trap", e.to_string())))
}

/// Runs the runner's `start` for the try `ctx` names: a handle is `Held`.
pub(crate) async fn start(ctx: &ActivityContext, spec: &Spec, input: &Value) -> ActivityOutput {
    let record = context(ctx, input, true);
    let called = call(ctx, spec, true, spec.timeout, move |store, runner| {
        runner.runspore_runner_step().call_start(store, &record)
    })
    .await;
    let failure = |code: &str, message: String, details: Value| ActivityOutput::Failure {
        error: ActivityOutput::error(code, message, details),
        retryable: false,
    };
    match called {
        Ok(Ok(handle)) => {
            if handle.len() > MAX_HOLDER_BYTES {
                let message = format!(
                    "the holder handle is {} bytes; at most {MAX_HOLDER_BYTES}",
                    handle.len()
                );
                return failure("component.output-too-large", message, Value::Null);
            }
            match canonical::parse(handle.as_bytes()) {
                Ok(holder) => {
                    failpoint::hit("host.component.after-start");
                    ActivityOutput::Held { holder }
                }
                Err(e) => failure(
                    "component.malformed-output",
                    format!("the runner's handle is not canonical-domain JSON: {e}"),
                    Value::Null,
                ),
            }
        }
        Ok(Err(step::Failure {
            code,
            message,
            retryable,
        })) => {
            let code = if crate::external::is_label(&code) {
                code
            } else {
                "component.failed".to_string()
            };
            ActivityOutput::Failure {
                error: ActivityOutput::error(&code, message, Value::Null),
                retryable,
            }
        }
        Err(Ended::Denied(denied)) => failure(
            "component.not-granted",
            denied.message(&ctx.action_id),
            denied.details(),
        ),
        Err(Ended::Code(code @ ("component.module-missing" | "component.invalid"), message)) => {
            failure(code, message, Value::Null)
        }
        // A process may have started before it trapped.
        Err(Ended::Code(code, message)) => ActivityOutput::Unknown {
            error: ActivityOutput::error(code, message, Value::Null),
        },
    }
}

/// Runs the runner's `probe` on `handle`. Anything but an answer reads as unknown.
pub(crate) async fn probe(
    ctx: &ActivityContext,
    spec: &Spec,
    input: &Value,
    handle: &[u8],
) -> Answer {
    let record = context(ctx, input, false);
    let handle = String::from_utf8_lossy(handle).into_owned();
    let called = call(ctx, spec, false, LOOK_TIMEOUT, move |store, runner| {
        runner
            .runspore_runner_step()
            .call_probe(store, &record, &handle)
    })
    .await;
    match called {
        Ok(step::Holder::Alive) => Answer::Alive,
        Ok(step::Holder::Dead) => Answer::Dead,
        Ok(step::Holder::Unknown) | Err(_) => Answer::Unknown,
    }
}

/// Runs the runner's `stop` on `handle`: exit 0 with its text when it returned `ok`,
/// exit 1 when `err`, none when it did not return, with why as the text.
pub(crate) async fn stop(
    ctx: &ActivityContext,
    spec: &Spec,
    input: &Value,
    handle: &[u8],
) -> (Option<i32>, Vec<u8>) {
    let record = context(ctx, input, false);
    let handle = String::from_utf8_lossy(handle).into_owned();
    let called = call(ctx, spec, false, LOOK_TIMEOUT, move |store, runner| {
        runner
            .runspore_runner_step()
            .call_stop(store, &record, &handle)
    })
    .await;
    match called {
        Ok(Ok(text)) => (Some(0), text.into_bytes()),
        Ok(Err(text)) => (Some(1), text.into_bytes()),
        Err(ended) => (None, ended.message(&ctx.action_id).into_bytes()),
    }
}

/// The `component` runner: runs an action's step runner, whose `start` hands the
/// attempt to the holder it names.
#[derive(Clone, Default)]
pub struct ComponentRunner;

#[async_trait]
impl ActivityRunner for ComponentRunner {
    fn validate(&self, action_id: &str, action: &Value) -> Result<(), String> {
        read(action)
            .map(|_| ())
            .map_err(|(_, e)| format!("action `{action_id}`: {e}"))
    }

    async fn run(&self, ctx: &ActivityContext, action: &Value, input: &Value) -> ActivityOutput {
        match parse(&ctx.action_id, action) {
            Ok(spec) => start(ctx, &spec, input).await,
            Err(e) => ActivityOutput::Failure {
                error: ActivityOutput::error("action.invalid", e.to_string(), json!(null)),
                retryable: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_is_a_relative_path() {
        assert_eq!(entry_path("runner.wasm").as_deref(), Some("runner.wasm"));
        assert_eq!(entry_path("./bin/./r.wasm").as_deref(), Some("bin/r.wasm"));
        for bad in [
            "",
            "/abs.wasm",
            "../up.wasm",
            "a/../b.wasm",
            "a\\b.wasm",
            ".",
        ] {
            assert_eq!(entry_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_test_runner_is_a_runner_and_the_kernel_is_not() {
        assert!(load(runspore_wasmtime::TEST_RUNNER).is_ok());
        let kernel = load(runspore_wasmtime::COMPONENT)
            .err()
            .expect("not a runner");
        assert!(kernel.contains("step"), "{kernel}");
        assert!(load(b"not a component").is_err());
    }

    #[test]
    fn grants_of_another_kind_have_their_own_code() {
        let action = json!({"kind": "component", "module": "r.wasm",
                            "grants": {"network": ["*"]}});
        let refused = parse("review", &action).unwrap_err();
        assert_eq!(refused.code(), "component.grant-unsupported");
        let refused =
            parse("review", &json!({"kind": "component", "module": "/r.wasm"})).unwrap_err();
        assert_eq!(refused.code(), "action.invalid");
    }

    #[test]
    fn the_token_is_passed_on_unless_the_action_says_not() {
        let action =
            |pass: Value| json!({"kind": "component", "module": "r.wasm", "passToken": pass});
        assert!(
            parse("review", &json!({"kind": "component", "module": "r.wasm"}))
                .unwrap()
                .pass_token
        );
        assert!(!parse("review", &action(json!(false))).unwrap().pass_token);
        let refused = parse("review", &action(json!("no"))).unwrap_err();
        assert_eq!(refused.code(), "action.invalid");
        assert!(refused.to_string().contains("passToken"), "{refused}");
    }
}
