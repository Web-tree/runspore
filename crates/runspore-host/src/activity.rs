use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use runspore_types::model::ErrorInfo;
use runspore_types::store::RunKey;
use serde_json::Value;
use tokio::sync::watch;

/// What an activity reports. The engine maps it to an `ActivityResult` unchanged.
#[derive(Debug, Clone, PartialEq)]
pub enum ActivityOutput {
    Success {
        outcome: String,
        output: Value,
    },
    Failure {
        error: ErrorInfo,
        retryable: bool,
    },
    /// The effect may or may not have happened. Never guess; report this.
    Unknown {
        error: ErrorInfo,
    },
}

impl ActivityOutput {
    /// An `ErrorInfo` with the given code and message and no details.
    pub fn error(code: &str, message: impl Into<String>, details: Value) -> ErrorInfo {
        ErrorInfo {
            code: code.to_string(),
            message: message.into(),
            node_id: None,
            details,
        }
    }
}

/// Everything an attempt knows about itself, plus its stop signal.
#[derive(Debug, Clone)]
pub struct ActivityContext {
    pub key: RunKey,
    pub node_id: String,
    pub action_id: String,
    pub invocation_id: String,
    pub attempt: u32,
    /// Identical on every attempt of one invocation.
    pub effect_key: String,
    /// Base for relative working directories.
    pub base_dir: PathBuf,
    stop: watch::Receiver<bool>,
}

impl ActivityContext {
    /// A context whose stop signal fires when `stop` is set to true.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: RunKey,
        node_id: String,
        action_id: String,
        invocation_id: String,
        attempt: u32,
        effect_key: String,
        base_dir: PathBuf,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            key,
            node_id,
            action_id,
            invocation_id,
            attempt,
            effect_key,
            base_dir,
            stop,
        }
    }

    /// Whether the stop signal has fired.
    pub fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }

    /// Resolves once the lease is lost or the engine shuts down. A runner must then
    /// abandon its work and report `Unknown` unless it knows the effect did not happen.
    pub async fn stopped(&self) {
        let mut stop = self.stop.clone();
        let _ = stop.wait_for(|stopped| *stopped).await;
    }
}

/// Executes one kind of action.
#[async_trait]
pub trait ActivityRunner: Send + Sync {
    /// Checks the action's implementation fields when a workflow is started.
    fn validate(&self, action_id: &str, action: &Value) -> Result<(), String>;
    /// Runs one attempt. Must return promptly once `ctx.stopped()` resolves.
    async fn run(&self, ctx: &ActivityContext, action: &Value, input: &Value) -> ActivityOutput;
}

/// Maps an action's `kind` to its runner.
#[derive(Clone, Default)]
pub struct ActivityRegistry {
    runners: BTreeMap<String, Arc<dyn ActivityRunner>>,
}

impl ActivityRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the built-in `command` runner and the given `native` functions.
    pub fn with_builtins(native: NativeRunner) -> Self {
        Self::new()
            .with("command", Arc::new(crate::CommandRunner))
            .with("native", Arc::new(native))
    }

    /// Registers `runner` for `kind`, replacing any earlier one.
    pub fn with(mut self, kind: &str, runner: Arc<dyn ActivityRunner>) -> Self {
        self.runners.insert(kind.to_string(), runner);
        self
    }

    /// The runner for `kind`.
    pub fn get(&self, kind: &str) -> Option<&Arc<dyn ActivityRunner>> {
        self.runners.get(kind)
    }
}

/// A native activity: an async Rust function of the context and the input.
pub type NativeFn = Arc<
    dyn Fn(ActivityContext, Value) -> Pin<Box<dyn Future<Output = ActivityOutput> + Send>>
        + Send
        + Sync,
>;

/// The `native` runner: `{"kind": "native", "function": "<name>"}` calls the function
/// registered under that name.
#[derive(Clone, Default)]
pub struct NativeRunner {
    functions: BTreeMap<String, NativeFn>,
}

impl NativeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `f` under `name`.
    pub fn function<F, Fut>(mut self, name: &str, f: F) -> Self
    where
        F: Fn(ActivityContext, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ActivityOutput> + Send + 'static,
    {
        self.functions.insert(
            name.to_string(),
            Arc::new(move |ctx, input| Box::pin(f(ctx, input))),
        );
        self
    }

    fn lookup(&self, action: &Value) -> Result<&NativeFn, String> {
        let name = action
            .get("function")
            .and_then(Value::as_str)
            .ok_or("native action needs a string `function`")?;
        self.functions
            .get(name)
            .ok_or_else(|| format!("no native function `{name}` is registered"))
    }
}

#[async_trait]
impl ActivityRunner for NativeRunner {
    fn validate(&self, action_id: &str, action: &Value) -> Result<(), String> {
        self.lookup(action)
            .map(|_| ())
            .map_err(|e| format!("action `{action_id}`: {e}"))
    }

    async fn run(&self, ctx: &ActivityContext, action: &Value, input: &Value) -> ActivityOutput {
        match self.lookup(action) {
            Ok(f) => f(ctx.clone(), input.clone()).await,
            Err(e) => ActivityOutput::Failure {
                error: ActivityOutput::error("native.unknown-function", e, Value::Null),
                retryable: false,
            },
        }
    }
}
