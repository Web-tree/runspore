//! Argument parsing and the commands of spec/cli.md.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{ArgGroup, Args, Parser, Subcommand};
use runspore_build::{Applicability, Machine, Report};
use runspore_host::{
    validate_workflow, ActivityRegistry, Engine, EngineConfig, HostError, NativeRunner, Observer,
    StartOutcome, StepClaim, StepReport, ARTIFACT_UNREADABLE, FILES_DIR_MISSING, FILES_UNWRITABLE,
};
use runspore_kernel::NativeReducer;
use runspore_store_sqlite::SqliteStore;
use runspore_types::artifact::{Artifact, ArtifactError, Manifest};
use runspore_types::digest;
use runspore_types::model::{event_kind, ErrorInfo, Resolution, RunStatus, State};
use runspore_types::reducer::Reducer;
use runspore_types::store::{Disposition, RunKey, RunParent, RunView, StoreFailureKind};
use runspore_wasmtime::{load_artifact, ArtifactReader, WasmtimeReducer};
use serde_json::{json, Value};
use tokio::signal::unix::{signal, Signal, SignalKind};

use crate::view::{Row, Shown, Tree};
use crate::{project, view};

pub const FORMAT: &str = "runspore.cli/0.2";

/// The version of each contract this `spore` implements (spec/cli.md, Capabilities).
const CONTRACTS: [(&str, &str); 7] = [
    ("artifact", "0.1"),
    ("cli", "0.2"),
    ("dot", "0.2"),
    ("host", "0.2"),
    ("kernel", "0.2"),
    ("schema", "0.2"),
    ("store", "0.2"),
];

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
    /// Watches of held async steps one worker runs at once, apart from its slots.
    #[arg(long, global = true)]
    pub max_watches: Option<usize>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create the project folder `.spore/` at the repository root.
    Init,
    /// Build a workflow and check it and its actions without touching the database.
    /// Reports every problem at once.
    Validate {
        workflow: PathBuf,
        /// One line per problem: `file:line:col: code: message`.
        #[arg(long)]
        short: bool,
        /// Fail on warnings too.
        #[arg(long)]
        strict: bool,
        /// Apply the fixes marked safe, write the files, and check again.
        #[arg(long)]
        fix: bool,
    },
    /// Print the help page of a validation code, or list every code.
    Explain { code: Option<String> },
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
        /// Fail on warnings too.
        #[arg(long)]
        strict: bool,
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
    /// reason, outstanding invocation. Probes a held async step first.
    Status {
        run_id: String,
        /// Read the store only: run no probe and change nothing.
        #[arg(long)]
        no_probe: bool,
    },
    /// All runs with status, as a tree, each with its workflow and what it needs.
    List {
        /// Only the unfinished runs, newest first.
        #[arg(long)]
        unfinished: bool,
    },
    /// Do the next thing a run needs, then print what a person must type: the run's,
    /// or the only unfinished one.
    Continue {
        run_id: Option<String>,
        /// Follow the run past held async steps until it needs a person or has ended.
        #[arg(long)]
        wait: bool,
    },
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
    /// End an unfinished run as cancelled: stop its running command and run its held
    /// step's stop. A run that has ended is left as it is.
    Cancel {
        run_id: String,
        /// Why, kept with the run.
        #[arg(long)]
        reason: Option<String>,
        /// The request ID; repeating it is a no-op.
        #[arg(long)]
        id: Option<String>,
    },
    /// Claim, renew, complete, fail or abandon an external step: one an actor outside
    /// the worker does.
    #[command(subcommand)]
    Step(StepCommand),
    /// What this spore runs: its version, the contract versions, the action kinds, those
    /// it runs async, and its result channels.
    Capabilities,
}

#[derive(Subcommand)]
pub enum StepCommand {
    /// Claim a ready external step: the run's, or the oldest in the database. Prints the
    /// token, the input and the instructions.
    Claim {
        run_id: Option<String>,
        /// Take only steps with this actor label; without it, only unlabelled steps.
        #[arg(long)]
        actor: Option<String>,
        /// The holder handle: JSON kept with the attempt, shown by `status`.
        #[arg(long)]
        holder: Option<String>,
    },
    /// Extend the claimed attempt's lease, never past its deadline.
    Renew { token: String },
    /// Report the step done with an outcome.
    Complete {
        token: String,
        #[arg(long)]
        outcome: String,
        #[arg(long)]
        output: Option<String>,
    },
    /// Report the step failed.
    Fail {
        token: String,
        #[arg(long)]
        error: String,
        #[arg(long)]
        code: Option<String>,
        /// Another try could help: the kernel may retry, up to `maxAttempts`.
        #[arg(long)]
        retryable: bool,
    },
    /// Free the run's held async step at once.
    Abandon { run_id: String },
    /// Describe the step a run is at, the only unfinished one by default, without
    /// claiming or changing anything.
    Show { run_id: Option<String> },
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
    /// Text mode prints this instead of the one line `spore: <code>: <message>`: the
    /// report of a workflow that did not build.
    pub report: Option<String>,
}

impl Fail {
    pub fn usage(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 2,
            code: code.to_string(),
            message: message.into(),
            details: None,
            report: None,
        }
    }

    pub fn internal(code: &str, message: impl Into<String>) -> Self {
        Self {
            exit: 10,
            code: code.to_string(),
            message: message.into(),
            details: None,
            report: None,
        }
    }

    /// A workflow folder with problems: `workflow.invalid`, the report in text mode,
    /// its diagnostics as `details` in JSON mode.
    fn rejected(report: &Report) -> Self {
        Self {
            exit: 2,
            code: "workflow.invalid".to_string(),
            message: report.count(),
            details: Some(report.json()),
            report: Some(report.text(&short_command(report))),
        }
    }
}

/// How the report says to list every problem.
fn short_command(report: &Report) -> String {
    format!("spore validate --short {}", report.folder)
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
            HostError::OutcomeUnrouted(unrouted) => {
                return refused(unrouted.to_string(), unrouted.details())
            }
            HostError::Invalid { .. } | HostError::Kernel(_) => true,
            HostError::Store(f) => {
                f.kind == StoreFailureKind::Conflict
                    || f.code == "run.not-found"
                    || f.code == "lease.lost"
                    || f.code == "run.cancelled"
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
    /// Text mode prints this on stderr first: `validate`'s diagnostics.
    pub stderr: String,
    pub exit: u8,
}

fn done(json: Value, text: String) -> Result<Output, Fail> {
    Ok(Output {
        json,
        text,
        raw: None,
        stderr: String::new(),
        exit: 0,
    })
}

/// Text mode shows a workflow's warnings on stderr before a command goes on.
fn warn(cli: &Cli, report: Option<&Report>) {
    if let (false, Some(report)) = (cli.json, report) {
        if report.warnings() > 0 {
            eprint!("{}", report.text(&short_command(report)));
        }
    }
}

pub async fn execute(cli: &Cli) -> Result<Output, Fail> {
    let cwd = std::env::current_dir()
        .map_err(|e| Fail::internal("cwd", format!("current folder: {e}")))?;
    let given = cli.db.as_deref().filter(|db| !db.as_os_str().is_empty());
    let db = project::database(given, &cwd);
    // A start is read in full before the database is opened, so a bad one creates nothing.
    let request = match &cli.command {
        Command::Start(args) => Some(StartRequest::read("start", args, &db, &cwd)?),
        Command::Run(args) => Some(StartRequest::read("run", args, &db, &cwd)?),
        _ => None,
    };
    if let Some(request) = &request {
        warn(cli, request.source.report.as_ref());
    }
    match &cli.command {
        Command::Init => return project::init(&cwd),
        Command::Validate {
            workflow,
            short,
            strict,
            fix,
        } => {
            let options = Check {
                short: *short,
                strict: *strict,
                fix: *fix,
            };
            return validate(cli, &project::workflow(workflow, &cwd)?, options);
        }
        Command::Explain { code } => return explain(code.as_deref()),
        Command::Capabilities => return capabilities(),
        Command::Build {
            folder,
            output,
            composed,
            strict,
        } => return build(cli, folder, output.as_deref(), *composed, *strict),
        Command::Inspect {
            artifact: Some(path),
            ..
        } => return inspect_artifact(path),
        Command::Status { .. }
        | Command::List { .. }
        | Command::Events { .. }
        | Command::Inspect { .. }
        | Command::Step(StepCommand::Show { .. })
        | Command::Continue { .. }
            if !db.exists() =>
        {
            return missing_database(&cli.command, &db, &cwd);
        }
        Command::Status { .. }
        | Command::List { .. }
        | Command::Events { .. }
        | Command::Inspect { .. }
        | Command::Step(StepCommand::Show { .. }) => {}
        _ => project::before_write(&db)?,
    }
    let base_dir = match &request {
        Some(request) => request.base_dir.clone(),
        None => project::base_dir(None, &db, &cwd)?,
    };
    let parent = request.as_ref().and_then(|r| r.parent.clone());
    let engine = engine(cli, &db, base_dir, parent)?;
    if let Command::Run(_) | Command::Worker { .. } | Command::Continue { .. } = &cli.command {
        engine.observe(progress(cli.json));
    }
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
        | Command::Explain { .. }
        | Command::Capabilities
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
                stderr: String::new(),
                exit: EXIT_INTERRUPTED,
            })
        }
        Command::Status { run_id, no_probe } => {
            let mut view = existing(&engine, run_id).await?;
            // A dead holder is expired before the run is read.
            if !no_probe && engine.probe_step(&view.key).await?.is_some() {
                view = existing(&engine, run_id).await?;
            }
            let shown = Shown::read(&engine, &view).await?;
            let mut json = shown.json(&view);
            json["db"] = json!(db);
            done(json, shown.text(&view))
        }
        Command::List { unfinished } => list(&engine, &db, *unfinished).await,
        Command::Continue { run_id, wait } => {
            continue_run(&engine, &db, &cwd, run_id.as_deref(), *wait).await
        }
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
                .await
                .map_err(|e| id_reused(e, "signal", &key.run, &message_id))?;
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
                .await
                .map_err(|e| id_reused(e, "resolve", &key.run, &request_id))?;
            let disposition = disposition(&receipt.disposition);
            done(
                json!({"runId": key.run, "invocationId": invocation_id,
                       "requestId": request_id, "disposition": disposition}),
                format!("resolution {disposition} (request ID {request_id})"),
            )
        }
        Command::Step(StepCommand::Show { run_id }) => {
            step_show(&engine, &db, run_id.as_deref()).await
        }
        Command::Step(command) => step(&engine, command).await,
        Command::Cancel { run_id, reason, id } => {
            let key = existing(&engine, run_id).await?.key;
            let request_id = id.clone().unwrap_or_else(|| generated("cancel"));
            let cancelled = engine
                .cancel(&key, &request_id, reason.as_deref())
                .await
                .map_err(|e| id_reused(e, "cancel", &key.run, &request_id))?;
            done(
                view::cancelled_json(&cancelled, &request_id),
                view::cancelled_text(&cancelled, &request_id),
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

/// Prints each change of a run the engine commits on stderr, one line each: text, or
/// with `json` one object (spec/cli.md, Progress). Stdout keeps the command's output.
fn progress(json: bool) -> Observer {
    Arc::new(move |progress| {
        if json {
            let mut object = view::progress_json(progress);
            object["format"] = json!(FORMAT);
            eprintln!("{object}");
        } else {
            eprintln!("{}", view::progress_text(progress));
        }
    })
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

/// This machine, as the build checks what a run will need, with what this spore's
/// engine offers.
fn machine() -> Machine {
    Machine::here().offering(registry().capabilities())
}

/// `spore capabilities`: the version, the contract versions, and the capability set
/// (spec/cli.md, Capabilities). Reads no database.
fn capabilities() -> Result<Output, Fail> {
    let set = registry().capabilities();
    let contracts: serde_json::Map<String, Value> = CONTRACTS
        .iter()
        .map(|(name, version)| (name.to_string(), json!(version)))
        .collect();
    let version = env!("CARGO_PKG_VERSION");
    let line = |label: &str, items: &[String]| format!("{label:<15} {}\n", items.join(", "));
    let contract_text: Vec<String> = CONTRACTS
        .iter()
        .map(|(name, version)| format!("{name} {version}"))
        .collect();
    let text = format!("{:<15} {version}\n", "version")
        + &line("contracts", &contract_text)
        + &line("action kinds", &set.action_kinds)
        + &line("async kinds", &set.async_kinds)
        + &line("result channels", &set.result_channels);
    done(
        json!({"version": version, "contracts": contracts, "actionKinds": set.action_kinds,
               "asyncKinds": set.async_kinds, "resultChannels": set.result_channels}),
        text,
    )
}

/// The options of `validate`.
struct Check {
    short: bool,
    strict: bool,
    fix: bool,
}

/// Validation opens no database: it runs the checks of `start` against no store. A
/// folder gets every problem at once, on stderr; stdout says only that it is valid. An
/// artifact is checked as `start` checks it, and any other file is refused as `start`
/// refuses it.
fn validate(cli: &Cli, workflow: &Path, options: Check) -> Result<Output, Fail> {
    let folder = match runspore_build::entry(workflow).map_err(build_fail)? {
        runspore_build::Entry::Folder(folder) => folder,
        runspore_build::Entry::Document(_) => {
            let source = Source::read(workflow)?;
            let package = source.validate(cli)?;
            return done(
                json!({"valid": true, "problems": "0", "warnings": "0", "diagnostics": [],
                       "packageBytes": package.len().to_string()}),
                format!("{} is valid", workflow.display()),
            );
        }
    };
    let machine = machine();
    let mut checked = runspore_build::check(&folder, &machine).map_err(build_fail)?;
    let mut fixed = Vec::new();
    let mut messages = String::new();
    if options.fix {
        let applied = checked.report.apply(Applicability::Safe);
        for (path, content) in [
            (runspore_build::DOT_FILE, &applied.dot),
            (runspore_build::SETTINGS_FILE, &applied.settings),
        ] {
            if let Some(content) = content {
                let target = folder.join(path);
                std::fs::write(&target, content).map_err(|e| {
                    Fail::internal("file.unwritable", format!("{}: {e}", target.display()))
                })?;
            }
        }
        for (code, file, _) in &applied.changes {
            fixed.push(json!({"code": code, "file": checked.report.path_of(*file)}));
        }
        if !applied.changes.is_empty() {
            messages.push_str(&applied.text(&checked.report));
            checked = runspore_build::check(&folder, &machine).map_err(build_fail)?;
        }
    }
    let (report, package) = kernel_check(cli, &folder, checked)?;
    let failed = report.problems() > 0 || (options.strict && report.warnings() > 0);
    let mut json = report.json();
    json["valid"] = json!(!failed);
    if options.fix {
        json["fixed"] = json!(fixed);
    }
    if let (false, Some(package)) = (failed, &package) {
        json["packageBytes"] = json!(package.len().to_string());
    }
    if options.short {
        messages.push_str(&report.short());
    } else if !report.diagnostics.is_empty() {
        messages.push_str(&report.text(&short_command(&report)));
    }
    let text = if failed {
        String::new()
    } else {
        format!("{} is valid", workflow.display())
    };
    Ok(Output {
        json,
        text,
        raw: None,
        stderr: messages,
        exit: if failed { 2 } else { 0 },
    })
}

/// The kernel's own checks of a folder the build accepted. A refusal means the build
/// missed a check: the report says so and asks for a bug report, in place of the
/// kernel's own words.
fn kernel_check(
    cli: &Cli,
    folder: &Path,
    checked: runspore_build::Checked,
) -> Result<(Report, Option<Vec<u8>>), Fail> {
    let mut report = checked.report;
    let Some(built) = checked.built else {
        return Ok((report, None));
    };
    let sealed = built.seal().map_err(build_fail)?;
    let artifact = load(&sealed.bytes, folder)?;
    let limits = EngineConfig::default().limits;
    match validate_workflow(&*reducer(cli)?, &registry(), limits, &artifact.package) {
        Ok(package) => Ok((report, Some(package))),
        Err(HostError::Store(f)) => Err(Fail::internal(&f.code, f.message.clone())),
        Err(_) => {
            report.add(missed_check());
            Ok((report, None))
        }
    }
}

/// The diagnostic of a package the build accepted and the kernel refused.
fn missed_check() -> runspore_build::Diagnostic {
    runspore_build::Diagnostic::new(
        "spore.missed-check",
        runspore_build::diagnostic::At::dot_line(1),
    )
}

/// `spore explain [<code>]`: one code's page, or every code with its title.
fn explain(code: Option<&str>) -> Result<Output, Fail> {
    let catalogue = runspore_build::catalogue::catalogue();
    let Some(code) = code else {
        let mut entries: Vec<_> = catalogue.entries.iter().collect();
        entries.sort_by(|a, b| a.code.cmp(&b.code));
        let text: String = entries
            .iter()
            .map(|e| format!("{:<32} {:<8} {}\n", e.code, e.severity.as_str(), e.title))
            .collect();
        let json: Vec<Value> = entries
            .iter()
            .map(|e| json!({"code": e.code, "severity": e.severity.as_str(), "title": e.title}))
            .collect();
        return done(json!({"codes": json}), text);
    };
    let entry = catalogue.get(code).ok_or_else(|| {
        Fail::usage(
            "explain.unknown-code",
            format!("spore gives no code {code}; `spore explain` lists the codes it gives"),
        )
    })?;
    let page = runspore_build::explain(code).unwrap_or_default();
    done(
        json!({"code": entry.code, "severity": entry.severity.as_str(), "title": entry.title,
               "page": page,
               "docs": format!("{}/{}.md", runspore_build::catalogue::DOCS, entry.code)}),
        page,
    )
}

/// A read-only command, or `continue`, on a database that does not exist sees no runs,
/// and leaves no file behind.
fn missing_database(command: &Command, db: &Path, cwd: &Path) -> Result<Output, Fail> {
    match command {
        Command::List { .. } => done(json!({"db": db, "runs": []}), String::new()),
        Command::Continue { run_id: None, .. } => Ok(nothing_to_continue(cwd)),
        Command::Step(StepCommand::Show { run_id: None }) => Err(none_unfinished("step show")),
        Command::Status { run_id, .. }
        | Command::Events { run_id }
        | Command::Continue {
            run_id: Some(run_id),
            ..
        }
        | Command::Step(StepCommand::Show {
            run_id: Some(run_id),
        })
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
fn build(
    cli: &Cli,
    folder: &Path,
    output: Option<&Path>,
    composed: bool,
    strict: bool,
) -> Result<Output, Fail> {
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
    if let Some(report) = &source.report {
        if strict && report.warnings() > 0 {
            return Err(Fail::rejected(report));
        }
    }
    warn(cli, source.report.as_ref());
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
        stderr: String::new(),
        exit: 0,
    })
}

/// Opens the database, or refuses one another `spore` wrote with exit 10 and the fix
/// (spec/cli.md, databases from another version), and builds the engine on it.
fn engine(
    cli: &Cli,
    db: &Path,
    base_dir: PathBuf,
    parent: Option<RunParent>,
) -> Result<Engine, Fail> {
    let settings = project::settings(db)?;
    let store = SqliteStore::open(db)
        .map_err(|f| Fail::internal(&f.code, format!("{}: {}", db.display(), f.message)))?;
    let mut config = EngineConfig {
        base_dir,
        db_path: Some(db.to_path_buf()),
        files_dir: Some(project::files_dir(db)),
        parent,
        ..EngineConfig::default()
    };
    config.lease_ms = cli.lease_ms.unwrap_or(config.lease_ms);
    config.heartbeat_ms = cli.heartbeat_ms.unwrap_or(config.heartbeat_ms);
    config.poll_ms = cli.poll_ms.unwrap_or(config.poll_ms);
    config.shutdown_grace_ms = cli.grace_ms.unwrap_or(config.shutdown_grace_ms);
    config.stop_grace_ms = cli.stop_grace_ms.unwrap_or(config.stop_grace_ms);
    config.max_watches = cli.max_watches.unwrap_or(config.max_watches);
    config.max_concurrent_activities = settings.slots.unwrap_or(config.max_concurrent_activities);
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
    /// `start` or `run`, and the workflow as given, to repeat them in a hint.
    command: &'static str,
    workflow: String,
    source: Source,
    input: Value,
    start_key: String,
    base_dir: PathBuf,
    /// The step this start runs inside, from `RUNSPORE_RUN_ID` and `RUNSPORE_NODE_ID`.
    parent: Option<RunParent>,
}

impl StartRequest {
    fn read(command: &'static str, args: &StartArgs, db: &Path, cwd: &Path) -> Result<Self, Fail> {
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
            command,
            workflow: args.workflow.display().to_string(),
            source,
            input,
            start_key: args.key.clone().unwrap_or_else(|| generated("key")),
            base_dir: project::base_dir(args.base_dir.as_deref(), db, cwd)?,
            parent: parent_step(),
        })
    }

    async fn start(&self, engine: &Engine) -> Result<StartOutcome, Fail> {
        let (input, start_key) = (self.input.clone(), &self.start_key);
        match engine
            .start_artifact(&self.source.artifact, input, start_key)
            .await
        {
            Err(HostError::Store(f))
                if f.kind == StoreFailureKind::Conflict
                    && (f.code == "request.digest-mismatch" || f.code == "run.exists") =>
            {
                Err(match self.key_reused(engine).await? {
                    Some(fail) => fail,
                    None => self.source.fail(HostError::Store(f)),
                })
            }
            other => other.map_err(|e| self.source.fail(e)),
        }
    }

    /// The start key names a run started with another input or from another version
    /// of the workflow: `run.key-reused`, with that run, what differs and the commands
    /// that go on with it (spec/cli.md, Keys used again). `None` when the key names no
    /// such run.
    async fn key_reused(&self, engine: &Engine) -> Result<Option<Fail>, Fail> {
        let Some(key) = engine.find_run(&self.start_key).await? else {
            return Ok(None);
        };
        let Some(view) = engine.get_run(&key).await? else {
            return Ok(None);
        };
        let started = engine.list_events(&key, 0, 1).await?;
        let input = started
            .items
            .first()
            .and_then(|r| serde_json::from_slice::<Value>(&r.event.body).ok())
            .map_or(Value::Null, |body| body["input"].clone());
        let mut differs = Vec::new();
        if input != self.input {
            differs.push("input");
        }
        if view.package_digest != self.source.artifact.manifest.main {
            differs.push("workflow");
        }
        let (run, start_key) = (&key.run, &self.start_key);
        let mut commands = Vec::new();
        let how = match differs[..] {
            [] => return Ok(None),
            ["input"] => {
                commands.push(format!(
                    "spore {} {} --key {} --input {}",
                    self.command,
                    quoted(&self.workflow),
                    quoted(start_key),
                    quoted(&input.to_string())
                ));
                format!("with another input: {input}")
            }
            ["workflow"] => "from another version of the workflow, which it keeps".into(),
            _ => format!(
                "with another input, {input}, and from another version of the workflow, \
                 which it keeps"
            ),
        };
        commands.push(format!("spore worker --run {run} --until-parked"));
        let mut message = format!(
            "start key {start_key} already names run {run}, started {how}.\n\
             To go on with it: {}",
            commands[0]
        );
        for command in &commands[1..] {
            message += &format!("\n              or: {command}");
        }
        message += "\nTo start a new run instead, use another --key.";
        let mut fail = Fail::usage("run.key-reused", message);
        fail.details = Some(json!({
            "runId": run, "startKey": start_key, "differs": differs,
            "input": input, "commands": commands,
        }));
        Ok(Some(fail))
    }
}

/// A `--id` of `signal`, `resolve` or `cancel` used on this run before, for another request:
/// `<command>.id-reused` (spec/cli.md, Keys used again). Any other failure as it is.
fn id_reused(e: HostError, command: &str, run: &str, id: &str) -> Fail {
    let reused = matches!(&e, HostError::Store(f)
        if f.kind == StoreFailureKind::Conflict && f.code == "request.digest-mismatch");
    if !reused {
        return e.into();
    }
    let other = match command {
        "signal" => "a signal with another name, outcome or data",
        "cancel" => "a cancel with another reason",
        _ => "another resolution",
    };
    let mut fail = Fail::usage(
        &format!("{command}.id-reused"),
        format!(
            "ID {id} was already used on run {run} for {other}; a repeated ID must repeat \
             the same request. Send it with a new --id, or without --id for a generated one"
        ),
    );
    fail.details = Some(json!({"runId": run, "id": id}));
    fail
}

/// `text` as one shell word: as it is when plain, else in single quotes.
fn quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// The step a `spore` call runs inside: the run and node the host gives every command
/// it starts (spec/host.md §3), or `None` outside a step.
fn parent_step() -> Option<RunParent> {
    let var = |name| std::env::var(name).ok().filter(|v| !v.is_empty());
    Some(RunParent {
        run_id: var("RUNSPORE_RUN_ID")?,
        node_id: var("RUNSPORE_NODE_ID")?,
    })
}

/// `run_until_parked`, or on SIGINT/SIGTERM/SIGHUP `Engine::shutdown`, which applies the
/// results of the attempts it drains and leaves the rest of the run to the next worker.
async fn until_parked(engine: &Engine, key: &RunKey, db: &Path) -> Result<Output, Fail> {
    let (view, interrupted) = work(engine, key, false).await?;
    let shown = Shown::read(engine, &view).await?;
    Ok(parked(&view, &shown, db, interrupted))
}

/// Works the run until it parks, or with `follow` until it needs a person or has ended
/// (`Engine::follow`); with `true` when an interrupt stopped it first.
async fn work(engine: &Engine, key: &RunKey, follow: bool) -> Result<(RunView, bool), Fail> {
    let mut stop = Interrupts::install()?;
    let parked = tokio::select! {
        view = async {
            match follow {
                true => engine.follow(key).await,
                false => engine.run_until_parked(key).await,
            }
        } => Some(view?),
        () = stop.wait() => None,
    };
    match parked {
        Some(view) => Ok((view, false)),
        None => {
            engine.shutdown().await;
            Ok((existing(engine, &key.run).await?, true))
        }
    }
}

/// The run's `status` and the exit code of a parked run, as `run` prints them.
fn parked(view: &RunView, shown: &Shown, db: &Path, interrupted: bool) -> Output {
    let exit = if interrupted {
        EXIT_INTERRUPTED
    } else {
        view::exit_for(view, shown.step.as_ref())
    };
    let mut json = shown.json(view);
    json["db"] = json!(db);
    if interrupted {
        json["interrupted"] = json!(true);
    }
    Output {
        text: shown.text(view),
        json,
        raw: None,
        stderr: String::new(),
        exit,
    }
}

/// `spore continue [<runId>] [--wait]` (spec/cli.md, What a run needs): with nothing
/// unfinished the workflows to start; with several, the runs and exit 2; with one, the
/// run worked until it parks (with `wait`, until it needs a person or has ended), unless
/// it is quarantined or finished, then what a person must type.
async fn continue_run(
    engine: &Engine,
    db: &Path,
    cwd: &Path,
    run: Option<&str>,
    wait: bool,
) -> Result<Output, Fail> {
    let view = match run {
        Some(run) => existing(engine, run).await?,
        None => match only_unfinished(engine, db, "continue").await? {
            Some(view) => view,
            None => return Ok(nothing_to_continue(cwd)),
        },
    };
    let (view, interrupted) = match view::needs(&view, None) {
        Some("quarantined") | None => (view, false),
        Some(_) => work(engine, &view.key, wait).await?,
    };
    let shown = Shown::read(engine, &view).await?;
    // A holder that waits for a person: its watch stops with its process group, and the
    // attempt stays held for the next driver.
    if shown.step.as_ref().is_some_and(|s| s.waits.is_some()) {
        engine.shutdown().await;
    }
    let mut out = parked(&view, &shown, db, interrupted);
    if !interrupted {
        let next = shown.next(&view);
        out.json["needs"] = json!(next.needs);
        out.json["commands"] = json!(next.commands);
        out.text = next.text;
    }
    Ok(out)
}

/// `continue` with no unfinished run: the workflows of the project folder to start.
fn nothing_to_continue(cwd: &Path) -> Output {
    let workflows = project::workflows(cwd);
    let mut text = "nothing to continue".to_string();
    if workflows.is_empty() {
        text += "; no workflows in a project folder (.spore/workflows/) here";
    } else {
        text += "; workflows to start with spore start <name>:";
        for (name, folder) in &workflows {
            text += &format!("\n  {name}  {}", folder.display());
        }
    }
    let workflows: Vec<Value> = workflows
        .iter()
        .map(|(name, folder)| json!({"name": name, "folder": folder}))
        .collect();
    Output {
        json: json!({"runs": [], "workflows": workflows}),
        text,
        raw: None,
        stderr: String::new(),
        exit: 0,
    }
}

/// The one unfinished run; `None` when there is none. Several are `run.several`, exit
/// 2, listed as `list --unfinished` lists them, so that `command` is given one.
async fn only_unfinished(
    engine: &Engine,
    db: &Path,
    command: &str,
) -> Result<Option<RunView>, Fail> {
    let tree = unfinished(engine).await?;
    match tree.rows.len() {
        0 => Ok(None),
        1 => Ok(Some(tree.runs[tree.rows[0].index].clone())),
        n => {
            let message = format!("{n} runs are unfinished; name one: spore {command} <runId>");
            let mut fail = Fail::usage("run.several", message.clone());
            fail.report = Some(format!(
                "{}\nspore: run.several: {message}\n",
                tree.text(0, now_ms())
            ));
            fail.details = Some(json!({"db": db, "runs": tree.json()}));
            Err(fail)
        }
    }
}

fn none_unfinished(command: &str) -> Fail {
    Fail::usage(
        "run.none-unfinished",
        format!("no run is unfinished; name one: spore {command} <runId>"),
    )
}

impl Shown {
    async fn read(engine: &Engine, view: &RunView) -> Result<Self, Fail> {
        let runs = all_runs(engine).await?;
        let now = now_ms();
        Ok(Self {
            arrival: engine.arrival(view).await?,
            package: package(engine, view).await?,
            invocations: engine.list_invocations(&view.key).await?,
            step: engine.async_step(view).await?,
            last_result: last_result(engine, view).await?,
            children: read_tree(engine, runs, Some(&view.key.run)).await?,
            slots: engine.held_slots(now).await?,
            slot_limit: engine.config().max_concurrent_activities,
            now_ms: now,
        })
    }
}

/// The payload of the latest `activity.result` of the outstanding invocation of a run
/// that needs intervention: the error of an attempt whose outcome is unknown. `None`
/// for any other run.
async fn last_result(engine: &Engine, view: &RunView) -> Result<Option<Value>, Fail> {
    let invocation = view
        .snapshot
        .as_deref()
        .and_then(|s| serde_json::from_slice::<State>(s).ok())
        .filter(|s| s.status == RunStatus::NeedsIntervention)
        .and_then(|s| s.invocation);
    let Some(invocation) = invocation else {
        return Ok(None);
    };
    let (mut last, mut after) = (None, 0);
    loop {
        let page = engine.list_events(&view.key, after, 100).await?;
        let Some(end) = page.items.last() else {
            return Ok(last);
        };
        after = end.event.sequence;
        for record in &page.items {
            let event = &record.event;
            if event.kind != event_kind::ACTIVITY_RESULT || record.consumed_revision.is_none() {
                continue;
            }
            let payload: Value = serde_json::from_slice(&event.body).unwrap_or(Value::Null);
            if payload["invocationId"] == invocation.invocation_id.as_str() {
                last = Some(payload);
            }
        }
    }
}

/// Every run of `runs` as a tree, or with `root`, the runs below it, each unfinished
/// one with its node and since when it has stood there (spec/cli.md, `list`).
async fn read_tree(
    engine: &Engine,
    mut runs: Vec<RunView>,
    root: Option<&str>,
) -> Result<Tree, Fail> {
    runs.sort_by(|a, b| (a.created_at_ms, &a.key.run).cmp(&(b.created_at_ms, &b.key.run)));
    let index: HashMap<&str, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.key.run.as_str(), i))
        .collect();
    let mut names = HashMap::new();
    let mut rows = Vec::new();
    for (run, depth) in view::tree(&runs, root) {
        let (node_id, since_ms) = whereabouts(engine, run).await?;
        let step = engine.async_step(run).await?;
        let needs = view::needs(run, step.as_ref());
        let actor = step
            .filter(|_| matches!(needs, Some("external-ready" | "held")))
            .and_then(|s| s.actor);
        if !names.contains_key(&run.package_digest) {
            let name = package(engine, run)
                .await?
                .and_then(|p| p["name"].as_str().map(String::from));
            names.insert(run.package_digest.clone(), name);
        }
        rows.push(Row {
            index: index[run.key.run.as_str()],
            depth,
            node_id,
            since_ms,
            workflow: names[&run.package_digest].clone(),
            needs,
            actor,
        });
    }
    Ok(Tree {
        runs,
        rows,
        flat: false,
    })
}

/// The unfinished runs alone, newest first, not as a tree (spec/cli.md, What a run
/// needs).
async fn unfinished(engine: &Engine) -> Result<Tree, Fail> {
    let mut tree = read_tree(engine, all_runs(engine).await?, None).await?;
    tree.rows.retain(|row| row.needs.is_some());
    let runs = &tree.runs;
    tree.rows.sort_by(|a, b| {
        let (a, b) = (&runs[a.index], &runs[b.index]);
        (b.created_at_ms, &b.key.run).cmp(&(a.created_at_ms, &a.key.run))
    });
    for row in &mut tree.rows {
        row.depth = 0;
    }
    tree.flat = true;
    Ok(tree)
}

/// The node an unfinished run is at and since when it has stood there; `None`s for a
/// run that has finished or not yet taken its first turn.
async fn whereabouts(
    engine: &Engine,
    view: &RunView,
) -> Result<(Option<String>, Option<u64>), Fail> {
    let state: Option<State> = view
        .snapshot
        .as_deref()
        .and_then(|bytes| serde_json::from_slice(bytes).ok());
    let Some(position) = state
        .filter(|s| !s.status.is_terminal() && !view.ended())
        .and_then(|s| s.position)
    else {
        return Ok((None, None));
    };
    let since = engine.arrival(view).await?.since_ms;
    Ok((Some(position.node_id), since))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `spore step show [<runId>]`: the step description, without claiming, probing or
/// recording anything (spec/cli.md, What a run needs).
async fn step_show(engine: &Engine, db: &Path, run: Option<&str>) -> Result<Output, Fail> {
    let view = match run {
        Some(run) => existing(engine, run).await?,
        None => only_unfinished(engine, db, "step show")
            .await?
            .ok_or_else(|| none_unfinished("step show"))?,
    };
    let step = match engine.describe_step(&view).await {
        Ok(Some(step)) => step,
        Ok(None) => {
            return Err(Fail::usage(
                "step.none",
                format!(
                    "run {} is {} and at no activity or await-signal node",
                    view.key.run, view.status
                ),
            ))
        }
        // The step files could not be made; the run is left as it is.
        Err(HostError::Invalid { code, message })
            if [ARTIFACT_UNREADABLE, FILES_UNWRITABLE, FILES_DIR_MISSING]
                .contains(&code.as_str()) =>
        {
            return Err(Fail::internal(&code, message));
        }
        Err(e) => return Err(e.into()),
    };
    let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let from = step
        .from
        .as_ref()
        .map(|a| json!({"nodeId": a.node_id, "outcome": a.outcome}));
    let json = json!({
        "runId": step.key.run, "nodeId": step.node_id, "nodeKind": step.node_kind,
        "activationId": step.activation_id, "actionId": step.action_id,
        "signal": step.signal, "outcomes": step.outcomes,
        "outputSchemas": step.output_schemas,
        "files": path(&step.files), "instructions": path(&step.instructions),
        "from": from,
    });
    let mut text = format!(
        "step       {} ({}, {}) of run {}\n",
        step.node_id, step.node_kind, step.activation_id, step.key.run
    );
    if let Some(arrow) = &step.from {
        text += &format!("from       {} on {}\n", arrow.node_id, arrow.outcome);
    }
    match (&step.action_id, &step.signal) {
        (Some(action), _) => text += &format!("action     {action}\n"),
        (None, Some(signal)) => text += &format!("signal     {signal}\n"),
        (None, None) => {}
    }
    for (i, outcome) in step.outcomes.iter().enumerate() {
        let label = if i == 0 { "outcomes" } else { "" };
        text += &match step.output_schemas.get(outcome) {
            Some(schema) => format!("{label:<10} {outcome}, with data that fits {schema}\n"),
            None => format!("{label:<10} {outcome}\n"),
        };
    }
    if let Some(files) = &step.files {
        text += &format!("files      {}\n", files.display());
    }
    if let Some(instructions) = &step.instructions {
        let content = std::fs::read_to_string(instructions).map_err(|e| {
            Fail::internal(
                "file.unreadable",
                format!("{}: {e}", instructions.display()),
            )
        })?;
        text += &format!("\ninstructions ({}):\n\n{content}", instructions.display());
    }
    done(json, text)
}

/// `spore step …` (spec/cli.md, Steps outside the worker).
async fn step(engine: &Engine, command: &StepCommand) -> Result<Output, Fail> {
    let reported = |report: StepReport| {
        let disposition = disposition(&report.disposition);
        done(
            json!({"runId": report.key.run, "invocationId": report.invocation_id,
                   "attempt": report.attempt, "disposition": disposition}),
            format!(
                "step {} attempt {} {disposition}",
                report.invocation_id, report.attempt
            ),
        )
    };
    match command {
        StepCommand::Claim {
            run_id,
            actor,
            holder,
        } => {
            if let Some(run) = run_id {
                existing(engine, run).await?;
            }
            let holder = holder
                .as_deref()
                .map(|h| parse_json("--holder", h))
                .transpose()?;
            let claim = engine
                .claim_step(run_id.as_deref(), actor.as_deref(), holder)
                .await?;
            match (claim, run_id) {
                (Some(claim), _) => claimed(&claim),
                (None, Some(run)) => {
                    let label = match actor {
                        Some(actor) => format!("for actor {actor}"),
                        None => "without an actor label".to_string(),
                    };
                    Err(Fail::usage(
                        "step.not-ready",
                        format!("run {run} has no ready external step {label}"),
                    ))
                }
                (None, None) => done(json!({"claimed": false}), "nothing to claim".into()),
            }
        }
        StepCommand::Renew { token } => {
            let renewed = engine.renew_step(token).await?;
            done(
                json!({"runId": renewed.key.run,
                       "leaseUntilMs": renewed.lease_until_ms.to_string(),
                       "deadlineMs": renewed.deadline_ms.map(|d| d.to_string())}),
                format!("lease renewed until {} ms", renewed.lease_until_ms),
            )
        }
        StepCommand::Complete {
            token,
            outcome,
            output,
        } => {
            let output = output
                .as_deref()
                .map(|o| parse_json("--output", o))
                .transpose()?
                .unwrap_or(Value::Null);
            reported(engine.complete_step(token, outcome, output).await?)
        }
        StepCommand::Fail {
            token,
            error,
            code,
            retryable,
        } => {
            let code = code.as_deref();
            reported(engine.fail_step(token, error, code, *retryable).await?)
        }
        StepCommand::Abandon { run_id } => {
            let key = existing(engine, run_id).await?.key;
            reported(engine.abandon_step(&key).await?)
        }
        StepCommand::Show { .. } => unreachable!("handled by step_show"),
    }
}

/// A claim as JSON, and as text for a person: the step, how to report it, then its
/// instructions.
fn claimed(claim: &StepClaim) -> Result<Output, Fail> {
    let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    let token = claim.token.to_string();
    let json = json!({
        "claimed": true, "token": token, "runId": claim.key.run,
        "nodeId": claim.node_id, "actionId": claim.action_id,
        "invocationId": claim.invocation_id, "attempt": claim.attempt,
        "input": claim.input, "params": claim.params, "outcomes": claim.outcomes,
        "outputSchemas": claim.output_schemas,
        "deadlineMs": claim.deadline_ms.to_string(),
        "leaseUntilMs": claim.lease_until_ms.to_string(),
        "resultChannel": claim.result_channel,
        "files": path(&claim.files), "instructions": path(&claim.instructions),
        "workDir": path(&claim.work_dir),
    });
    let mut text = format!(
        "claimed step {} of run {} (attempt {})\n",
        claim.node_id, claim.key.run, claim.attempt
    );
    text += &format!("outcomes   {}\n", claim.outcomes.join(", "));
    text += &format!("deadline   {} ms\n", claim.deadline_ms);
    text += &format!("input      {}\n", claim.input);
    if let Some(dir) = &claim.work_dir {
        text += &format!("work dir   {}\n", dir.display());
    }
    text += &format!("token      {token}\n\nreport with:\n");
    text += &format!("  spore step complete {token} --outcome <outcome> [--output <json>]\n");
    text += &format!("  spore step fail {token} --error <message> [--retryable]\n");
    if let Some(instructions) = &claim.instructions {
        let content = std::fs::read_to_string(instructions).map_err(|e| {
            Fail::internal(
                "file.unreadable",
                format!("{}: {e}", instructions.display()),
            )
        })?;
        text += &format!("\ninstructions ({}):\n\n{content}", instructions.display());
    }
    done(json, text)
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

async fn all_runs(engine: &Engine) -> Result<Vec<RunView>, Fail> {
    let mut runs = Vec::new();
    let mut cursor = None;
    loop {
        let page = engine.list_runs(cursor, 100).await?;
        runs.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(runs),
        }
    }
}

async fn list(engine: &Engine, db: &Path, unfinished_only: bool) -> Result<Output, Fail> {
    let tree = if unfinished_only {
        unfinished(engine).await?
    } else {
        read_tree(engine, all_runs(engine).await?, None).await?
    };
    done(
        json!({"db": db, "runs": tree.json()}),
        tree.text(0, now_ms()),
    )
}

async fn events(engine: &Engine, run: &str) -> Result<Output, Fail> {
    let run_view = existing(engine, run).await?;
    let key = run_view.key.clone();
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
    // The replay only adds the node and the outcome; a run it cannot replay still lists.
    let history = engine.history(&run_view).await.unwrap_or_default();
    let happened = |r: &runspore_types::store::EventRecord| history.get(&r.event.sequence);
    let width = records
        .iter()
        .filter_map(|r| happened(r)?.node_id.as_ref())
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(1);
    let text: Vec<String> = records
        .iter()
        .map(|r| view::event_text(r, happened(r), width))
        .collect();
    let json: Vec<Value> = records.iter().map(view::event_json).collect();
    // A cancel is a store record, not an event: it comes after the events.
    let mut text = text.join("\n");
    if let Some(cancel) = &run_view.cancel {
        text += &format!("\n{}", view::cancel_line(cancel));
    }
    let cancel = run_view
        .cancel
        .as_ref()
        .map(|c| json!({"reason": c.reason, "at": c.at_ms.to_string(), "requestId": c.request_id}));
    done(
        json!({"runId": key.run, "events": json, "cancel": cancel}),
        text,
    )
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

/// The workflow a command was given, as the artifact it builds to or is, the folder
/// or artifact file it came from, and for a folder the report of its build (its
/// warnings).
struct Source {
    artifact: Artifact,
    origin: PathBuf,
    report: Option<Report>,
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
                    report: None,
                })
            }
        }
    }

    /// Builds the folder in memory and loads its artifact like a prebuilt one. A folder
    /// with problems is refused with every one of them.
    fn folder(folder: &Path) -> Result<Self, Fail> {
        let checked = runspore_build::check(folder, &machine()).map_err(build_fail)?;
        let Some(built) = checked.built else {
            return Err(Fail::rejected(&checked.report));
        };
        let sealed = built.seal().map_err(build_fail)?;
        Ok(Self {
            artifact: load(&sealed.bytes, folder)?,
            origin: folder.to_path_buf(),
            report: Some(checked.report),
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
    /// workflow, and names neither. The kernel refusing a folder the build accepted is
    /// a check the build missed.
    fn fail(&self, e: HostError) -> Fail {
        let about_input = matches!(e, HostError::InputInvalid(_));
        let about_workflow = matches!(&e, HostError::Invalid { .. } | HostError::Kernel(_))
            && (e.code().starts_with("workflow.") || e.code().starts_with("action."));
        if let (Some(report), true) = (&self.report, about_workflow) {
            let mut report = report.clone();
            report.add(missed_check());
            return Fail::rejected(&report);
        }
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
