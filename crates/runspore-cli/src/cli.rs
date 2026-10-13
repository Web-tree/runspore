//! Argument parsing and the commands of spec/cli.md.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{ArgGroup, Args, Parser, Subcommand};
use runspore_host::{
    validate_workflow, ActivityRegistry, Engine, EngineConfig, HostError, NativeRunner,
    StartOutcome,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::artifact::{Artifact, ArtifactError, Manifest};
use runspore_types::digest;
use runspore_types::model::{ErrorInfo, Resolution, State};
use runspore_types::reducer::Reducer;
use runspore_types::store::{Disposition, RunKey, RunView, StoreFailureKind};
use runspore_wasmtime::{load_artifact, ArtifactReader, WasmtimeReducer};
use serde_json::{json, Value};
use tokio::signal::unix::{signal, Signal, SignalKind};

use crate::{project, view};

pub const FORMAT: &str = "runspore.cli/0.2";

/// Exit code after a graceful shutdown on SIGINT, SIGTERM or SIGHUP: for `worker` always, for
/// `run` and `worker --until-parked` when the run had not parked yet.
const EXIT_INTERRUPTED: u8 = 130;

#[derive(Parser)]
#[command(name = "spore", version, about = "Embedded durable workflow runtime")]
pub struct Cli {
    /// Database file. Without it and `$RUNSPORE_DB`, the `.spore/runspore.db` of the
    /// nearest project folder at or above the current folder, else `./runspore.db`.
    #[arg(long, global = true, env = "RUNSPORE_DB")]
    pub db: Option<PathBuf>,
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
    #[arg(long, global = true)]
    pub stop_grace_ms: Option<u64>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create the project folder `.spore/` at the repository root.
    Init,
    /// Build a workflow and check it and its actions without touching the database.
    Validate { workflow: PathBuf },
    /// Build a workflow folder into its workflow artifact.
    Build {
        /// A workflow folder or its workflow.dot.
        folder: PathBuf,
        /// The artifact file; by default `<folder>.wasm` beside the folder, or
        /// `<folder>-composed.wasm` with --composed.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write the composed artifact: the kernel this spore runs and the workflow
        /// artifact in one file, exporting both worlds.
        #[arg(long)]
        composed: bool,
    },
    /// Print an artifact's manifest and package, or a file a run's artifact holds. For
    /// debugging; not a stable interface.
    #[command(group(ArgGroup::new("target").required(true).args(["artifact", "run"])))]
    Inspect {
        artifact: Option<PathBuf>,
        #[arg(long, requires = "file")]
        run: Option<String>,
        /// A path in the run's artifact, such as `workflow.dot`.
        #[arg(long, requires = "run")]
        file: Option<String>,
    },
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
    /// Status, position and the arrow into it, result and the end node, quarantine
    /// reason, outstanding invocation.
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
    /// Answer a stuck step, or an invocation whose outcome is unknown. Without an
    /// invocation ID, the run's outstanding one.
    #[command(group(ArgGroup::new("resolution").required(true).args(["complete", "retry", "fail"])))]
    Resolve {
        run_id: String,
        invocation_id: Option<String>,
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
    /// A workflow folder, its workflow.dot, a workflow artifact, or the name of a
    /// workflow in the project folder.
    workflow: PathBuf,
    /// The run's base folder, where its commands run. Default: the folder holding the
    /// database's project folder, else the current folder.
    #[arg(long)]
    base_dir: Option<PathBuf>,
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
    /// Structured details for JSON mode, such as the errors of `output.invalid`.
    pub details: Option<Value>,
}

impl Fail {
    pub fn usage(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 2,
            code: code.to_string(),
            message: message.into(),
            details: None,
        }
    }

    pub fn internal(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 10,
            code: code.to_string(),
            message: message.into(),
            details: None,
        }
    }
}

/// An artifact that does not load is the caller's to fix: exit 2.
impl From<ArtifactError> for Fail {
    fn from(e: ArtifactError) -> Self {
        Fail::usage(e.code, e.message)
    }
}

/// Rejections of the caller's request exit 2; store and internal failures exit 10.
impl From<HostError> for Fail {
    fn from(e: HostError) -> Self {
        // A misfit prints its summary, then one error per line, and carries the
        // errors as details (spec/cli.md, output and input schemas).
        let refused = |message: String, details: Value| {
            let mut fail = Fail::usage(e.code(), message);
            fail.details = Some(details);
            fail
        };
        let usage = match &e {
            HostError::OutputInvalid(misfit) => {
                return refused(misfit.to_string(), misfit.details())
            }
            HostError::InputInvalid(misfit) => {
                return refused(misfit.to_string(), misfit.details())
            }
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
    /// Bytes printed as they are instead of `text`.
    pub raw: Option<Vec<u8>>,
    pub exit: u8,
}

fn done(json: Value, text: String) -> Result<Output, Fail> {
    Ok(Output {
        json,
        text,
        raw: None,
        exit: 0,
    })
}

pub async fn execute(cli: &Cli) -> Result<Output, Fail> {
    let cwd = std::env::current_dir()
        .map_err(|e| Fail::internal("cwd", format!("current folder: {e}")))?;
    let given = cli.db.as_deref().filter(|db| !db.as_os_str().is_empty());
    let db = project::database(given, &cwd);
    // A start is read in full before the database is opened, so a bad one creates nothing.
    let request = match &cli.command {
        Command::Start(args) | Command::Run(args) => Some(StartRequest::read(args, &db, &cwd)?),
        _ => None,
    };
    match &cli.command {
        Command::Init => return project::init(&cwd),
        Command::Validate { workflow } => {
            return validate(cli, &project::workflow(workflow, &cwd)?)
        }
        Command::Build {
            folder,
            output,
            composed,
        } => return build(cli, folder, output.as_deref(), *composed),
        Command::Inspect {
            artifact: Some(path),
            ..
        } => return inspect_artifact(path),
        Command::Status { .. }
        | Command::List
        | Command::Events { .. }
        | Command::Inspect { .. }
            if !db.exists() =>
        {
            return missing_database(&cli.command, &db);
        }
        Command::Status { .. }
        | Command::List
        | Command::Events { .. }
        | Command::Inspect { .. } => {}
        _ => project::before_write(&db)?,
    }
    let base_dir = match &request {
        Some(request) => request.base_dir.clone(),
        None => project::base_dir(None, &db, &cwd)?,
    };
    let engine = engine(cli, &db, base_dir)?;
    if let Some(request) = &request {
        let outcome = request.start(&engine).await?;
        if let Command::Run(_) = &cli.command {
            let mut out = until_parked(&engine, &outcome.key, &db).await?;
            out.json["created"] = json!(outcome.created);
            return Ok(out);
        }
        let (run, start_key) = (&outcome.key.run, &request.start_key);
        let text = format!(
            "{} run {run} (start key {start_key})",
            started(outcome.created)
        );
        return done(
            json!({"runId": run, "startKey": start_key, "created": outcome.created}),
            text,
        );
    }
    match &cli.command {
        Command::Init
        | Command::Validate { .. }
        | Command::Build { .. }
        | Command::Start(_)
        | Command::Run(_) => unreachable!("handled above"),
        Command::Inspect { run, file, .. } => {
            let (Some(run), Some(file)) = (run, file) else {
                unreachable!("clap requires an artifact, or --run with --file")
            };
            inspect_file(&engine, run, file).await
        }
        Command::Worker { run: Some(run), .. } => {
            let key = existing(&engine, run).await?.key;
            until_parked(&engine, &key, &db).await
        }
        Command::Worker { run: None, .. } => {
            let mut stop = Interrupts::install()?;
            engine.serve(stop.wait()).await?;
            Ok(Output {
                json: json!({"interrupted": true}),
                text: "worker stopped".into(),
                raw: None,
                exit: EXIT_INTERRUPTED,
            })
        }
        Command::Status { run_id } => {
            let view = existing(&engine, run_id).await?;
            let arrival = engine.arrival(&view).await?;
            let package = package(&engine, &view).await?;
            let invocations = engine.list_invocations(&view.key).await?;
            let mut json = view::status_json(&view, &arrival, package.as_ref(), &invocations);
            json["db"] = json!(db);
            done(
                json,
                view::status_text(&view, &arrival, package.as_ref(), &invocations),
            )
        }
        Command::List => list(&engine, &db).await,
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
            let view = existing(&engine, run_id).await?;
            let invocation_id = match invocation_id {
                Some(id) => id.clone(),
                None => outstanding(&view)?,
            };
            let invocation_id = &invocation_id;
            let key = view.key;
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
    let source = Source::read(workflow)?;
    let package = source.validate(cli)?;
    done(
        json!({"valid": true, "packageBytes": package.len().to_string()}),
        format!("{} is valid", workflow.display()),
    )
}

/// A read-only command on a database that does not exist sees no runs, and leaves no
/// file behind.
fn missing_database(command: &Command, db: &Path) -> Result<Output, Fail> {
    match command {
        Command::List => done(json!({"db": db, "runs": []}), String::new()),
        Command::Status { run_id }
        | Command::Events { run_id }
        | Command::Inspect {
            run: Some(run_id), ..
        } => Err(Fail::usage(
            "run.not-found",
            format!(
                "run {run_id} does not exist ({} does not exist)",
                db.display()
            ),
        )),
        _ => unreachable!("only read-only commands"),
    }
}

/// Builds a folder, checks it as `validate` does, and writes its artifact: the
/// workflow artifact, or with `composed` the composed artifact holding the kernel too
/// (spec/artifact.md §7).
fn build(cli: &Cli, folder: &Path, output: Option<&Path>, composed: bool) -> Result<Output, Fail> {
    let folder = match runspore_build::entry(folder).map_err(build_fail)? {
        runspore_build::Entry::Folder(folder) => folder,
        runspore_build::Entry::Document(path) => {
            return Err(Fail::usage(
                "workflow.folder",
                format!(
                    "{}: `build` takes a workflow folder or its `{}`",
                    path.display(),
                    runspore_build::DOT_FILE
                ),
            ))
        }
    };
    let source = Source::folder(&folder)?;
    source.validate(cli)?;
    let artifact = source.artifact;
    // The kernel a composed artifact carries: the one this binary runs.
    let kernel = composed.then_some(runspore_wasmtime::COMPONENT);
    let (artifact, suffix) = match kernel {
        Some(kernel) => {
            let bytes = runspore_build::compose(kernel, &artifact.bytes)
                .map_err(|message| Fail::internal("artifact.encode", message))?;
            // The composition is checked the way any artifact is before it is written.
            (load(&bytes, &folder)?, "-composed.wasm")
        }
        None => (artifact, ".wasm"),
    };
    let target = match output {
        Some(path) => path.to_path_buf(),
        None => beside(&folder, suffix)?,
    };
    std::fs::write(&target, &artifact.bytes)
        .map_err(|e| Fail::internal("file.unwritable", format!("{}: {e}", target.display())))?;
    let mut json = json!({"artifact": target.display().to_string(), "artifactDigest": artifact.digest,
                          "packageDigest": artifact.manifest.main,
                          "bytes": artifact.bytes.len().to_string()});
    if let Some(kernel) = kernel {
        json["composed"] = json!(true);
        json["kernelDigest"] = json!(digest::hash("kernel", &[kernel]));
    }
    done(
        json,
        format!("built {} ({})", target.display(), artifact.digest),
    )
}

/// `<folder><suffix>` next to the folder: a sibling, never inside it.
fn beside(folder: &Path, suffix: &str) -> Result<PathBuf, Fail> {
    let named = match folder.file_name() {
        Some(_) => folder.to_path_buf(),
        None => std::fs::canonicalize(folder)
            .map_err(|e| Fail::usage("workflow.folder", format!("{}: {e}", folder.display())))?,
    };
    let name = named.file_name().ok_or_else(|| {
        Fail::usage(
            "workflow.folder",
            format!(
                "{} has no name to give its artifact; use -o",
                folder.display()
            ),
        )
    })?;
    Ok(named.with_file_name(format!("{}{suffix}", name.to_string_lossy())))
}

/// The digest, manifest and package of an artifact file. Opens no database.
fn inspect_artifact(path: &Path) -> Result<Output, Fail> {
    let artifact = load(&read_file(path)?, path)?;
    let package: Value = serde_json::from_slice(&artifact.package)
        .map_err(|e| Fail::internal("json.decode", e.to_string()))?;
    let json = json!({"artifactDigest": artifact.digest, "manifest": artifact.manifest,
                      "package": package});
    let text = serde_json::to_string_pretty(&json)
        .map_err(|e| Fail::internal("json.encode", e.to_string()))?;
    done(json, text)
}

/// A file of the run's stored artifact, as raw bytes; never the source folder.
async fn inspect_file(engine: &Engine, run: &str, path: &str) -> Result<Output, Fail> {
    let view = existing(engine, run).await?;
    let digest = view.artifact_digest.ok_or_else(|| {
        Fail::usage(
            "artifact.none",
            format!("run {run} was started without a workflow artifact"),
        )
    })?;
    let unreadable =
        |e: ArtifactError| Fail::internal("artifact.unreadable", format!("artifact {digest}: {e}"));
    let bytes = engine
        .get_artifact(&digest)
        .await?
        .ok_or_else(|| unreadable(ArtifactError::invalid("not in the store")))?;
    let mut reader = ArtifactReader::open(&bytes).map_err(unreadable)?;
    let manifest = Manifest::decode(&reader.manifest().map_err(unreadable)?).map_err(unreadable)?;
    let Some(entry) = manifest.entry(path) else {
        let held: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        return Err(Fail::usage(
            "artifact.no-file",
            format!(
                "the artifact of run {run} holds no file {path}; it holds {}",
                held.join(", ")
            ),
        ));
    };
    let content = reader.entry(&entry.digest).map_err(unreadable)?;
    Ok(Output {
        json: json!({"runId": run, "path": entry.path, "kind": entry.kind.as_str(),
                     "digest": entry.digest, "text": String::from_utf8(content.clone()).ok()}),
        text: String::new(),
        raw: Some(content),
        exit: 0,
    })
}

/// Opens the database, or refuses one another `spore` wrote with exit 10 and the fix
/// (spec/cli.md, databases from another version), and builds the engine on it.
fn engine(cli: &Cli, db: &Path, base_dir: PathBuf) -> Result<Engine, Fail> {
    let store = SqliteStore::open(db)
        .map_err(|f| Fail::internal(&f.code, format!("{}: {}", db.display(), f.message)))?;
    let mut config = EngineConfig {
        base_dir,
        db_path: Some(db.to_path_buf()),
        ..EngineConfig::default()
    };
    config.lease_ms = cli.lease_ms.unwrap_or(config.lease_ms);
    config.heartbeat_ms = cli.heartbeat_ms.unwrap_or(config.heartbeat_ms);
    config.poll_ms = cli.poll_ms.unwrap_or(config.poll_ms);
    config.shutdown_grace_ms = cli.grace_ms.unwrap_or(config.shutdown_grace_ms);
    config.stop_grace_ms = cli.stop_grace_ms.unwrap_or(config.stop_grace_ms);
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

fn started(created: bool) -> &'static str {
    if created {
        "created"
    } else {
        "existing"
    }
}

/// What `start` and `run` were asked to start.
struct StartRequest {
    source: Source,
    input: Value,
    start_key: String,
    base_dir: PathBuf,
}

impl StartRequest {
    fn read(args: &StartArgs, db: &Path, cwd: &Path) -> Result<Self, Fail> {
        let source = Source::read(&project::workflow(&args.workflow, cwd)?)?;
        let input = match (&args.input, &args.input_file) {
            (Some(text), _) => parse_json("--input", text)?,
            (None, Some(path)) => {
                let bytes = read_file(path)?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| Fail::usage("json.invalid", format!("{}: {e}", path.display())))?
            }
            (None, None) => json!({}),
        };
        // The input against the workflow's inputSchema before the database is opened,
        // so a refused start leaves no file behind (spec/cli.md). The engine checks it
        // again, after the schema itself.
        if let Some(schema) = source.input_schema() {
            runspore_schema::fit_input(&schema, &input)
                .map_err(|misfit| Fail::from(HostError::InputInvalid(misfit)))?;
        }
        Ok(Self {
            source,
            input,
            start_key: args.key.clone().unwrap_or_else(|| generated("key")),
            base_dir: project::base_dir(args.base_dir.as_deref(), db, cwd)?,
        })
    }

    async fn start(&self, engine: &Engine) -> Result<StartOutcome, Fail> {
        let (input, start_key) = (self.input.clone(), &self.start_key);
        engine
            .start_artifact(&self.source.artifact, input, start_key)
            .await
            .map_err(|e| self.source.fail(e))
    }
}

/// `run_until_parked`, or on SIGINT/SIGTERM/SIGHUP `Engine::shutdown`, which applies the
/// results of the attempts it drains and leaves the rest of the run to the next worker.
async fn until_parked(engine: &Engine, key: &RunKey, db: &Path) -> Result<Output, Fail> {
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
    let arrival = engine.arrival(&view).await?;
    let package = package(engine, &view).await?;
    let invocations = engine.list_invocations(&view.key).await?;
    let mut json = view::status_json(&view, &arrival, package.as_ref(), &invocations);
    json["db"] = json!(db);
    if exit == EXIT_INTERRUPTED {
        json["interrupted"] = json!(true);
    }
    Ok(Output {
        text: view::status_text(&view, &arrival, package.as_ref(), &invocations),
        json,
        raw: None,
        exit,
    })
}

/// The ID of the run's outstanding invocation, which `resolve` answers by default.
fn outstanding(view: &RunView) -> Result<String, Fail> {
    let state: Option<State> = view
        .snapshot
        .as_deref()
        .and_then(|bytes| serde_json::from_slice(bytes).ok());
    state
        .and_then(|state| state.invocation)
        .map(|invocation| invocation.invocation_id)
        .ok_or_else(|| {
            Fail::usage(
                "run.no-invocation",
                format!("run {} has no step to resolve", view.key.run),
            )
        })
}

/// The run's package as JSON, `None` if it is not stored.
async fn package(engine: &Engine, view: &RunView) -> Result<Option<Value>, Fail> {
    let bytes = engine.get_package(&view.package_digest).await?;
    Ok(bytes.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
}

async fn existing(engine: &Engine, run: &str) -> Result<RunView, Fail> {
    engine
        .get_run(&engine_key(engine, run))
        .await?
        .ok_or_else(|| Fail::usage("run.not-found", format!("run {run} does not exist")))
}

async fn list(engine: &Engine, db: &Path) -> Result<Output, Fail> {
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
    done(json!({"db": db, "runs": json}), text.join("\n"))
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

fn build_fail(e: runspore_build::BuildError) -> Fail {
    Fail::usage(e.code, e.to_string())
}

/// Loads an artifact (spec/artifact.md §5); a refusal names where it came from.
fn load(bytes: &[u8], origin: &Path) -> Result<Artifact, Fail> {
    load_artifact(bytes).map_err(|e| {
        let mut fail = Fail::from(e);
        fail.message = format!("{}: {}", origin.display(), fail.message);
        fail
    })
}

/// The WebAssembly magic bytes that start every artifact.
const WASM_MAGIC: &[u8] = b"\0asm";

/// The workflow a command was given, as the artifact it builds to or is, and the
/// folder or artifact file it came from.
struct Source {
    artifact: Artifact,
    origin: PathBuf,
}

impl Source {
    /// A folder, its `workflow.dot`, or an artifact file. Any other file is refused:
    /// `spore` 0.2 reads no JSON workflow document (spec/cli.md).
    fn read(path: &Path) -> Result<Self, Fail> {
        match runspore_build::entry(path).map_err(build_fail)? {
            runspore_build::Entry::Folder(folder) => Self::folder(&folder),
            runspore_build::Entry::Document(path) => {
                let bytes = read_file(&path)?;
                if !bytes.starts_with(WASM_MAGIC) {
                    return Err(Fail::usage(
                        "workflow.folder",
                        format!(
                            "{}: not a workflow folder or artifact. A workflow is a folder \
                             holding `{}` and `settings.json` (spec/dot.md), or an artifact \
                             `spore build` made from one; spore 0.2 reads no JSON workflow \
                             document",
                            path.display(),
                            runspore_build::DOT_FILE
                        ),
                    ));
                }
                Ok(Self {
                    artifact: load(&bytes, &path)?,
                    origin: path,
                })
            }
        }
    }

    /// Builds the folder in memory and loads its artifact like a prebuilt one.
    fn folder(folder: &Path) -> Result<Self, Fail> {
        let sealed = runspore_build::build(folder)
            .and_then(|built| built.seal())
            .map_err(build_fail)?;
        Ok(Self {
            artifact: load(&sealed.bytes, folder)?,
            origin: folder.to_path_buf(),
        })
    }

    /// The package's `inputSchema`, when it has one that passes the schema checks; a
    /// schema that does not is left for the engine to refuse as `workflow.invalid-schema`.
    fn input_schema(&self) -> Option<Value> {
        let package: Value = serde_json::from_slice(&self.artifact.package).ok()?;
        let schema = package.get("inputSchema")?.clone();
        runspore_schema::check_input(&schema).ok().map(|()| schema)
    }

    /// The checks of `start`, against no store; returns the canonical package bytes.
    fn validate(&self, cli: &Cli) -> Result<Vec<u8>, Fail> {
        let limits = EngineConfig::default().limits;
        validate_workflow(&*reducer(cli)?, &registry(), limits, &self.artifact.package)
            .map_err(|e| self.fail(e))
    }

    /// A rejection of a package names the folder or artifact file it came from. A
    /// rejection of the run's input (`input.invalid`) is about the input, not the
    /// workflow, and names neither.
    fn fail(&self, e: HostError) -> Fail {
        let about_input = matches!(e, HostError::InputInvalid(_));
        let mut fail = Fail::from(e);
        if fail.exit == 2 && !about_input {
            fail.message = format!("{}: {}", self.origin.display(), fail.message);
        }
        fail
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

/// SIGINT, SIGTERM and SIGHUP, installed before work starts so none kills the process.
/// A closed terminal (SIGHUP) shuts down like Ctrl-C.
struct Interrupts {
    int: Signal,
    term: Signal,
    hup: Signal,
}

impl Interrupts {
    fn install() -> Result<Self, Fail> {
        let install = |kind| signal(kind).map_err(|e| Fail::internal("signal", e.to_string()));
        Ok(Self {
            int: install(SignalKind::interrupt())?,
            term: install(SignalKind::terminate())?,
            hup: install(SignalKind::hangup())?,
        })
    }

    async fn wait(&mut self) {
        tokio::select! {
            _ = self.int.recv() => {}
            _ = self.term.recv() => {}
            _ = self.hup.recv() => {}
        }
    }
}
