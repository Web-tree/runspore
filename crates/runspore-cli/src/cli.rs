//! Argument parsing and the commands of spec/cli.md.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{ArgGroup, Args, Parser, Subcommand};
use runspore_host::{
    validate_workflow, ActivityRegistry, Engine, EngineConfig, HostError, NativeRunner,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::model::{ErrorInfo, Resolution};
use runspore_types::reducer::Reducer;
use runspore_types::store::{Disposition, RunKey, RunView, StoreFailureKind};
use runspore_wasmtime::WasmtimeReducer;
use serde_json::{json, Value};
use tokio::signal::unix::{signal, Signal, SignalKind};

use crate::view;

pub const FORMAT: &str = "runspore.cli/0.1";

/// Exit code after a graceful shutdown on SIGINT or SIGTERM: for `worker` always, for
/// `run` and `worker --until-parked` when the run had not parked yet.
const EXIT_INTERRUPTED: u8 = 130;

#[derive(Parser)]
#[command(name = "spore", version, about = "Embedded durable workflow runtime")]
pub struct Cli {
    /// Database file.
    #[arg(
        long,
        global = true,
        env = "RUNSPORE_DB",
        default_value = "./runspore.db"
    )]
    pub db: PathBuf,
    /// Print one JSON object on stdout instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    /// Call the kernel natively instead of through Wasmtime; for debugging.
    #[arg(long, global = true)]
    pub native_kernel: bool,
    #[arg(long, global = true)]
    pub lease_ms: Option<u64>,
    #[arg(long, global = true)]
    pub heartbeat_ms: Option<u64>,
    #[arg(long, global = true)]
    pub poll_ms: Option<u64>,
    #[arg(long, global = true)]
    pub grace_ms: Option<u64>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Check a workflow document and its actions without touching the database.
    Validate { workflow: PathBuf },
    /// Create a run and return.
    Start(StartArgs),
    /// Start, then work until the run is terminal or parked.
    Run(StartArgs),
    /// Work on all runs until interrupted, or on one run until it parks.
    Worker {
        #[arg(long, requires = "until_parked")]
        run: Option<String>,
        #[arg(long, requires = "run")]
        until_parked: bool,
    },
    /// Status, position, result, quarantine reason, outstanding invocation.
    Status { run_id: String },
    /// All runs with status.
    List,
    /// The run's events in order, with the revision that consumed each.
    Events { run_id: String },
    /// Deliver a signal. Repeating a message ID is a no-op.
    Signal {
        run_id: String,
        name: String,
        #[arg(long)]
        outcome: Option<String>,
        #[arg(long)]
        data: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Answer an invocation whose outcome is unknown.
    #[command(group(ArgGroup::new("resolution").required(true).args(["complete", "retry", "fail"])))]
    Resolve {
        run_id: String,
        invocation_id: String,
        #[arg(long)]
        complete: Option<String>,
        #[arg(long, requires = "complete")]
        output: Option<String>,
        #[arg(long)]
        retry: bool,
        #[arg(long)]
        fail: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// Lift a quarantine.
    Release { run_id: String },
}

#[derive(Args)]
pub struct StartArgs {
    workflow: PathBuf,
    #[arg(long, conflicts_with = "input_file")]
    input: Option<String>,
    #[arg(long)]
    input_file: Option<PathBuf>,
    #[arg(long)]
    key: Option<String>,
}

/// A failed command: exit code, stable code, message.
pub struct Fail {
    pub exit: u8,
    pub code: String,
    pub message: String,
}

impl Fail {
    pub fn usage(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 2,
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn internal(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 10,
            code: code.to_string(),
            message: message.into(),
        }
    }
}

/// Rejections of the caller's request exit 2; store and internal failures exit 10.
impl From<HostError> for Fail {
    fn from(e: HostError) -> Self {
        let usage = match &e {
            HostError::Invalid { .. } | HostError::Kernel(_) => true,
            HostError::Store(f) => {
                f.kind == StoreFailureKind::Conflict || f.code == "run.not-found"
            }
        };
        let fail = if usage { Fail::usage } else { Fail::internal };
        fail(e.code(), e.to_string())
    }
}

/// What a successful command prints, and its exit code.
pub struct Output {
    pub json: Value,
    pub text: String,
    pub exit: u8,
}

fn done(json: Value, text: String) -> Result<Output, Fail> {
    Ok(Output {
        json,
        text,
        exit: 0,
    })
}

pub async fn execute(cli: &Cli) -> Result<Output, Fail> {
    if let Command::Validate { workflow } = &cli.command {
        return validate(cli, workflow);
    }
    let engine = engine(cli)?;
    match &cli.command {
        Command::Validate { .. } => unreachable!("handled above"),
        Command::Start(args) => {
            let (key, created) = start(&engine, args).await?;
            let text = format!(
                "{} run {} (start key {key})",
                started(created),
                key_run(&engine, &key)
            );
            done(
                json!({"runId": key_run(&engine, &key), "startKey": key, "created": created}),
                text,
            )
        }
        Command::Run(args) => {
            let (start_key, created) = start(&engine, args).await?;
            let key = engine_key(&engine, &key_run(&engine, &start_key));
            let mut out = until_parked(&engine, &key).await?;
            if let Value::Object(map) = &mut out.json {
                map.insert("created".into(), json!(created));
            }
            Ok(out)
        }
        Command::Worker { run: Some(run), .. } => {
            let key = existing(&engine, run).await?.key;
            until_parked(&engine, &key).await
        }
        Command::Worker { run: None, .. } => {
            let mut stop = Interrupts::install()?;
            engine.serve(stop.wait()).await?;
            Ok(Output {
                json: json!({"interrupted": true}),
                text: "worker stopped".into(),
                exit: EXIT_INTERRUPTED,
            })
        }
        Command::Status { run_id } => {
            let view = existing(&engine, run_id).await?;
            done(view::status_json(&view), view::status_text(&view))
        }
        Command::List => list(&engine).await,
        Command::Events { run_id } => events(&engine, run_id).await,
        Command::Signal {
            run_id,
            name,
            outcome,
            data,
            id,
        } => {
            let key = existing(&engine, run_id).await?.key;
            let data = data
                .as_deref()
                .map(|d| parse_json("--data", d))
                .transpose()?;
            let message_id = id.clone().unwrap_or_else(|| generated("msg"));
            let receipt = engine
                .signal(
                    &key,
                    &message_id,
                    name,
                    outcome.clone(),
                    data.unwrap_or(Value::Null),
                )
                .await?;
            let disposition = disposition(&receipt.disposition);
            done(
                json!({"runId": key.run, "messageId": message_id, "disposition": disposition}),
                format!("signal {name} {disposition} (message ID {message_id})"),
            )
        }
        Command::Resolve {
            run_id,
            invocation_id,
            complete,
            output,
            retry: _,
            fail,
            id,
        } => {
            let key = existing(&engine, run_id).await?.key;
            let resolution = match (complete, fail) {
                (Some(outcome), _) => Resolution::Complete {
                    outcome: outcome.clone(),
                    output: output
                        .as_deref()
                        .map(|o| parse_json("--output", o))
                        .transpose()?
                        .unwrap_or(Value::Null),
                },
                (None, Some(message)) => Resolution::Fail {
                    error: ErrorInfo {
                        code: "operator.failed".into(),
                        message: message.clone(),
                        node_id: None,
                        details: Value::Null,
                    },
                },
                (None, None) => Resolution::Retry,
            };
            let request_id = id.clone().unwrap_or_else(|| generated("req"));
            let receipt = engine
                .resolve(&key, &request_id, invocation_id, resolution)
                .await?;
            let disposition = disposition(&receipt.disposition);
            done(
                json!({"runId": key.run, "invocationId": invocation_id,
                       "requestId": request_id, "disposition": disposition}),
                format!("resolution {disposition} (request ID {request_id})"),
            )
        }
        Command::Release { run_id } => {
            let key = existing(&engine, run_id).await?.key;
            let receipt = engine.release(&key).await?;
            let disposition = disposition(&receipt.disposition);
            done(
                json!({"runId": key.run, "disposition": disposition}),
                format!("release {disposition}"),
            )
        }
    }
}

fn reducer(cli: &Cli) -> Result<Arc<dyn Reducer>, Fail> {
    if cli.native_kernel {
        return Ok(Arc::new(NativeReducer));
    }
    let reducer =
        WasmtimeReducer::new().map_err(|e| Fail::internal("kernel.load", e.to_string()))?;
    Ok(Arc::new(reducer))
}

fn registry() -> ActivityRegistry {
    ActivityRegistry::with_builtins(NativeRunner::new())
}

/// Validation opens no database: it runs the checks of `start` against no store.
fn validate(cli: &Cli, workflow: &Path) -> Result<Output, Fail> {
    let bytes = read_file(workflow)?;
    let config = EngineConfig::default();
    let package = validate_workflow(&*reducer(cli)?, &registry(), config.limits, &bytes)?;
    done(
        json!({"valid": true, "packageBytes": package.len().to_string()}),
        format!("{} is valid", workflow.display()),
    )
}

fn engine(cli: &Cli) -> Result<Engine, Fail> {
    let store =
        SqliteStore::open(&cli.db).map_err(|f| Fail::internal(&f.code, f.message.clone()))?;
    let mut config = EngineConfig::default();
    config.lease_ms = cli.lease_ms.unwrap_or(config.lease_ms);
    config.heartbeat_ms = cli.heartbeat_ms.unwrap_or(config.heartbeat_ms);
    config.poll_ms = cli.poll_ms.unwrap_or(config.poll_ms);
    config.shutdown_grace_ms = cli.grace_ms.unwrap_or(config.shutdown_grace_ms);
    Ok(Engine::new(
        Arc::new(store),
        reducer(cli)?,
        registry(),
        config,
    ))
}

fn engine_key(engine: &Engine, run: &str) -> RunKey {
    RunKey::new(&engine.config().tenant, run)
}

fn key_run(engine: &Engine, start_key: &str) -> String {
    runspore_types::digest::ids::run_from_start_key(&engine.config().tenant, start_key)
}

fn started(created: bool) -> &'static str {
    if created {
        "created"
    } else {
        "existing"
    }
}

async fn start(engine: &Engine, args: &StartArgs) -> Result<(String, bool), Fail> {
    let workflow = read_file(&args.workflow)?;
    let input = match (&args.input, &args.input_file) {
        (Some(text), _) => parse_json("--input", text)?,
        (None, Some(path)) => {
            let bytes = read_file(path)?;
            serde_json::from_slice(&bytes)
                .map_err(|e| Fail::usage("json.invalid", format!("{}: {e}", path.display())))?
        }
        (None, None) => json!({}),
    };
    let start_key = args.key.clone().unwrap_or_else(|| generated("key"));
    let outcome = engine.start(&workflow, input, &start_key).await?;
    Ok((start_key, outcome.created))
}

/// `run_until_parked`, or on SIGINT/SIGTERM `Engine::shutdown`, which applies the
/// results of the attempts it drains and leaves the rest of the run to the next worker.
async fn until_parked(engine: &Engine, key: &RunKey) -> Result<Output, Fail> {
    let mut stop = Interrupts::install()?;
    let parked = tokio::select! {
        view = engine.run_until_parked(key) => Some(view?),
        () = stop.wait() => None,
    };
    let (view, exit) = match parked {
        Some(view) => {
            let exit = view::exit_for(&view);
            (view, exit)
        }
        None => {
            engine.shutdown().await;
            (existing(engine, &key.run).await?, EXIT_INTERRUPTED)
        }
    };
    let mut json = view::status_json(&view);
    if exit == EXIT_INTERRUPTED {
        json["interrupted"] = json!(true);
    }
    Ok(Output {
        text: view::status_text(&view),
        json,
        exit,
    })
}

async fn existing(engine: &Engine, run: &str) -> Result<RunView, Fail> {
    engine
        .get_run(&engine_key(engine, run))
        .await?
        .ok_or_else(|| Fail::usage("run.not-found", format!("run {run} does not exist")))
}

async fn list(engine: &Engine) -> Result<Output, Fail> {
    let mut runs = Vec::new();
    let mut cursor = None;
    loop {
        let page = engine.list_runs(cursor, 100).await?;
        runs.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let text: Vec<String> = runs
        .iter()
        .map(|r| {
            let quarantined = if r.quarantine.is_some() {
                " quarantined"
            } else {
                ""
            };
            format!(
                "{}  {:<18}{quarantined}  {}",
                r.key.run, r.status, r.start_key
            )
        })
        .collect();
    let json: Vec<Value> = runs
        .iter()
        .map(|r| {
            json!({"runId": r.key.run, "startKey": r.start_key, "status": r.status,
                   "revision": r.revision.to_string(), "quarantined": r.quarantine.is_some()})
        })
        .collect();
    done(json!({"runs": json}), text.join("\n"))
}

async fn events(engine: &Engine, run: &str) -> Result<Output, Fail> {
    let key = existing(engine, run).await?.key;
    let mut records = Vec::new();
    loop {
        let after = records
            .last()
            .map_or(0, |r: &runspore_types::store::EventRecord| r.event.sequence);
        let page = engine.list_events(&key, after, 100).await?;
        let more = page.next.is_some() && !page.items.is_empty();
        records.extend(page.items);
        if !more {
            break;
        }
    }
    let text: Vec<String> = records.iter().map(view::event_text).collect();
    let json: Vec<Value> = records.iter().map(view::event_json).collect();
    done(json!({"runId": key.run, "events": json}), text.join("\n"))
}

fn disposition(d: &Disposition) -> &'static str {
    match d {
        Disposition::Applied => "applied",
        Disposition::Duplicate => "duplicate",
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>, Fail> {
    std::fs::read(path)
        .map_err(|e| Fail::usage("file.unreadable", format!("{}: {e}", path.display())))
}

fn parse_json(option: &str, text: &str) -> Result<Value, Fail> {
    serde_json::from_str(text).map_err(|e| Fail::usage("json.invalid", format!("{option}: {e}")))
}

/// A random identifier for a start key, message ID or request ID the caller omitted.
fn generated(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(nanos);
    hasher.write_u32(std::process::id());
    format!("{prefix}_{:016x}", hasher.finish())
}

/// SIGINT and SIGTERM, installed before work starts so neither kills the process.
struct Interrupts {
    int: Signal,
    term: Signal,
}

impl Interrupts {
    fn install() -> Result<Self, Fail> {
        let install = |kind| signal(kind).map_err(|e| Fail::internal("signal", e.to_string()));
        Ok(Self {
            int: install(SignalKind::interrupt())?,
            term: install(SignalKind::terminate())?,
        })
    }

    async fn wait(&mut self) {
        tokio::select! {
            _ = self.int.recv() => {}
            _ = self.term.recv() => {}
        }
    }
}
