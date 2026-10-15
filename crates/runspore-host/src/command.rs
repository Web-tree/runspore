use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use runspore_kernel::mapping;
use runspore_types::canonical;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use runspore_types::store::MAX_HOLDER_BYTES;

use crate::activity::{ActivityContext, ActivityOutput, ActivityRunner, ACTION_ROOTS};
use crate::external::STORE_CHANNEL;
use crate::guard::{Ending, Guarded};
use crate::process;

/// Largest stdout a command may produce, and the most of both streams text mode keeps.
pub const MAX_STDOUT: usize = 64 * 1024;
/// How much of stderr text mode keeps once the streams must be cut.
const TEXT_STDERR: usize = 16 * 1024;
/// How much of each stream a failure's details keep.
const TAIL: usize = 4 * 1024;
/// The first steps a `$get` path of a `result` block may take.
const RESULT_ROOTS: [&str; 5] = ["stdout", "input", "params", "run", "step"];

/// The `command` runner: executes `argv` directly in its own process group, under the
/// guard `ctx.guard`. Unix only.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandRunner;

/// Where a command runs: a path, or a JSON Pointer into the node's input naming one.
#[derive(Debug, Clone)]
enum Cwd {
    Path(String),
    FromInput(String),
}

struct Spec {
    /// Strings, and mappings evaluated before each try.
    argv: Vec<Value>,
    cwd: Option<Cwd>,
    env: Vec<(String, String)>,
    timeout_ms: Option<u64>,
    output: String,
    /// The `result` block's `outcome` and `output` mappings.
    result: Option<(Value, Value)>,
    outcomes: Vec<String>,
    exit_outcomes: Vec<(i32, String)>,
    /// An async command: `argv` is its start command, whose stdout is the holder
    /// handle.
    is_async: bool,
}

/// An async action's probe, watch or stop program (spec/host.md §3, Async commands):
/// the `command` form, with a command's `argv`, `cwd` and `env`.
#[derive(Debug, Clone)]
pub(crate) struct Program {
    argv: Vec<Value>,
    cwd: Option<Cwd>,
    env: Vec<(String, String)>,
    pub timeout_ms: Option<u64>,
}

/// Reads `{"command": {"argv", "cwd", "env", "timeoutMs"}}`; `name` is how a refusal
/// names it. The `component` form is not offered by this host.
pub(crate) fn parse_program(name: &str, value: &Value) -> Result<Program, String> {
    let shape = format!(
        "`{name}` must be {{\"command\": {{\"argv\", \"cwd\", \"env\", \"timeoutMs\"}}}}; \
         this host offers no other form"
    );
    let Some(Value::Object(fields)) = value
        .as_object()
        .filter(|tag| tag.len() == 1)
        .and_then(|tag| tag.get("command"))
    else {
        return Err(shape);
    };
    if let Some(key) = fields
        .keys()
        .find(|k| !["argv", "cwd", "env", "timeoutMs"].contains(&k.as_str()))
    {
        return Err(format!("`{name}.command` has no field `{key}`"));
    }
    let program = Value::Object(fields.clone());
    Ok(Program {
        argv: argv_of(&program).map_err(|e| format!("`{name}.command`: {e}"))?,
        cwd: cwd_of(&program).map_err(|e| format!("`{name}.command`: {e}"))?,
        env: env_of(&program).map_err(|e| format!("`{name}.command`: {e}"))?,
        timeout_ms: crate::external::millis(
            &program,
            "timeoutMs",
            &format!("{name}.command.timeoutMs"),
        )?,
    })
}

fn argv_of(action: &Value) -> Result<Vec<Value>, String> {
    let argv = action
        .get("argv")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or(ARGV_SHAPE)?
        .clone();
    for (index, item) in argv.iter().enumerate() {
        match item {
            Value::String(_) => {}
            Value::Object(_) => mapping::check(item, &ACTION_ROOTS)
                .map_err(|e| format!("`argv` item {index}: {e}"))?,
            _ => return Err(ARGV_SHAPE.into()),
        }
    }
    Ok(argv)
}

fn cwd_of(action: &Value) -> Result<Option<Cwd>, String> {
    Ok(match action.get("cwd") {
        None => None,
        Some(Value::String(s)) => Some(Cwd::Path(s.clone())),
        Some(Value::Object(m)) => match m.get("fromInput") {
            Some(Value::String(pointer)) if m.len() == 1 && is_pointer(pointer) => {
                Some(Cwd::FromInput(pointer.clone()))
            }
            _ => return Err(CWD_SHAPE.into()),
        },
        Some(_) => return Err(CWD_SHAPE.into()),
    })
}

fn env_of(action: &Value) -> Result<Vec<(String, String)>, String> {
    Ok(match action.get("env") {
        None => Vec::new(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect::<Option<_>>()
            .ok_or("`env` values must be strings")?,
        Some(_) => return Err("`env` must be an object of strings".into()),
    })
}

fn parse(action: &Value) -> Result<Spec, String> {
    let argv = argv_of(action)?;
    let cwd = cwd_of(action)?;
    let env = env_of(action)?;
    let is_async = crate::external::async_of(action)?.is_some();
    let timeout_ms = match action.get("timeoutMs") {
        None => None,
        Some(Value::String(s)) => Some(s.parse().map_err(|_| "`timeoutMs` must be a u64")?),
        Some(Value::Number(n)) => Some(n.as_u64().ok_or("`timeoutMs` must be a u64")?),
        Some(_) => return Err("`timeoutMs` must be a u64".into()),
    };
    let output = match action.get("output") {
        None => "text".to_string(),
        Some(Value::String(s)) if ["text", "json", "result", "none"].contains(&s.as_str()) => {
            s.clone()
        }
        Some(_) => {
            return Err("`output` must be \"text\", \"json\", \"result\" or \"none\"".into())
        }
    };
    let outcomes: Vec<&str> = match action.get("outcomes") {
        None => vec!["ok"],
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(Value::as_str).collect())
            .ok_or("`outcomes` must be an array of strings")?,
    };
    let exit_outcomes = match action.get("exitOutcomes") {
        None => Vec::new(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(code, name)| {
                let code: i32 = code
                    .parse()
                    .map_err(|_| format!("`exitOutcomes` key `{code}` is not an exit code"))?;
                match name.as_str() {
                    Some(n) if outcomes.contains(&n) => Ok((code, n.to_string())),
                    _ => Err(format!(
                        "`exitOutcomes` value for `{code}` is not a declared outcome"
                    )),
                }
            })
            .collect::<Result<_, String>>()?,
        Some(_) => return Err("`exitOutcomes` must be an object".into()),
    };
    let result = match action.get("result") {
        None => None,
        Some(Value::Object(m))
            if m.len() == 2 && m.contains_key("outcome") && m.contains_key("output") =>
        {
            if output != "json" {
                return Err("`result` reads JSON stdout, so it needs `output` \"json\"".into());
            }
            for field in ["outcome", "output"] {
                mapping::check(&m[field], &RESULT_ROOTS)
                    .map_err(|e| format!("`result.{field}`: {e}"))?;
            }
            Some((m["outcome"].clone(), m["output"].clone()))
        }
        Some(_) => return Err("`result` must be {\"outcome\", \"output\"}, two mappings".into()),
    };
    let reads_result = output == "result" || result.is_some();
    if reads_result && exit_outcomes.iter().any(|(code, _)| *code == 0) {
        return Err("exit 0 reads the result from stdout, so `exitOutcomes` cannot map 0".into());
    }
    Ok(Spec {
        argv,
        cwd,
        env,
        timeout_ms,
        output,
        result,
        outcomes: outcomes.into_iter().map(str::to_string).collect(),
        exit_outcomes,
        is_async,
    })
}

/// Sets up a program the host starts for a step (spec/host.md §3): its folder, the
/// action's `env`, the step's variables, piped standard streams, and after `env` the
/// variables a workflow cannot override.
fn prepare(command: &mut Command, ctx: &ActivityContext, cwd: &Path, env: &[(String, String)]) {
    command
        .current_dir(cwd)
        .envs(env.iter().cloned())
        .env("RUNSPORE_RUN_ID", &ctx.key.run)
        .env("RUNSPORE_NODE_ID", &ctx.node_id)
        .env("RUNSPORE_INVOCATION_ID", &ctx.invocation_id)
        .env("RUNSPORE_ATTEMPT", ctx.attempt.to_string())
        .env("RUNSPORE_VISIT", ctx.visit.to_string())
        .env("RUNSPORE_EFFECT_KEY", &ctx.effect_key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // After `env`, so a workflow cannot point its steps at another database.
    if let Some(db) = &ctx.db {
        command.env("RUNSPORE_DB", db);
    }
    // After `env` too; one the step lacks is removed, so a value the worker inherited
    // never reaches it.
    let files = ctx.files.as_ref();
    let instructions = files.and_then(|f| f.instructions.as_ref());
    for (name, value) in [
        ("RUNSPORE_STEP_FILES", files.map(|f| &f.dir)),
        ("RUNSPORE_INSTRUCTIONS", instructions),
        ("RUNSPORE_WORK_DIR", ctx.work_dir.as_ref()),
    ] {
        match value {
            Some(path) => command.env(name, path),
            None => command.env_remove(name),
        };
    }
    for name in ["RUNSPORE_TOKEN", "RUNSPORE_RESULT_CHANNEL"] {
        command.env_remove(name);
    }
}

/// A probe's or watch's stdout and how it ended; `None` when it did not run, was
/// stopped, timed out, or ended by a signal.
pub(crate) struct Ran {
    pub code: i32,
    pub stdout: Vec<u8>,
}

/// Runs an async action's probe or watch for the try `ctx` names: under the guard,
/// without a process record, with `stdin` and the step's variables. Each line of stdout
/// is offered to `line`; the lines it takes are left out of `Ran::stdout`.
pub(crate) async fn run_program(
    ctx: &ActivityContext,
    program: &Program,
    input: &Value,
    stdin: Vec<u8>,
    timeout: Duration,
    line: impl Fn(&[u8]) -> bool + Send + 'static,
) -> Option<Ran> {
    let cwd = working_folder(program.cwd.as_ref(), &ctx.base_dir, input).ok()?;
    let mut argv = words(&program.argv, &ctx.scope(input)).ok()?;
    if let Some(found) = ctx.files.as_ref().and_then(|f| f.program(&argv[0])) {
        argv[0] = found.to_string_lossy().into_owned();
    }
    let setup = |command: &mut Command| prepare(command, ctx, &cwd, &program.env);
    let mut guarded = Guarded::spawn(&ctx.guard, ctx.stop_grace, &argv, setup)
        .await
        .ok()?;
    guarded.go().await.ok()?;
    let (input_pipe, stdout, stderr) = guarded.stdio();
    let feed = tokio::spawn(async move {
        if let Some(mut pipe) = input_pipe {
            let _ = pipe.write_all(&stdin).await;
        }
    });
    let stdout = tokio::spawn(lines(stdout.expect("piped"), line));
    let stderr = tokio::spawn(drain(stderr.expect("piped"), TAIL, false));
    let ending = tokio::select! {
        ending = guarded.wait() => Some(ending),
        () = tokio::time::sleep(timeout) => None,
        () = ctx.stopped() => None,
    };
    if ending.is_none() {
        guarded.stop().await;
    }
    feed.abort();
    let stdout = stdout.await.unwrap_or_default();
    let _ = stderr.await;
    match ending? {
        Ending::Exited(code) => Some(Ran { code, stdout }),
        Ending::Signaled(_) | Ending::Lost => None,
    }
}

/// Reads `reader` to EOF line by line, keeping at most `MAX_STDOUT` bytes of the lines
/// `take` does not take.
async fn lines(reader: impl AsyncRead + Unpin, take: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let mut reader = tokio::io::BufReader::new(reader);
    let mut kept = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match tokio::io::AsyncBufReadExt::read_until(&mut reader, b'\n', &mut line).await {
            Ok(0) | Err(_) => return kept,
            Ok(_) => {}
        }
        if !take(&line) && kept.len() + line.len() <= MAX_STDOUT {
            kept.extend_from_slice(&line);
        }
    }
}

const ARGV_SHAPE: &str = "`argv` must be a non-empty array of strings and mappings";
const CWD_SHAPE: &str = "`cwd` must be a string or {\"fromInput\": \"<JSON Pointer>\"}";

/// The words of `argv` for this try: each mapping evaluated against `scope`. One that
/// does not give a string is `command.argv-invalid`; one that does not evaluate keeps
/// the kernel's mapping code. Neither is retryable: another try evaluates the same.
fn words(argv: &[Value], scope: &Map<String, Value>) -> Result<Vec<String>, ActivityOutput> {
    let mut operations = 0;
    let mut words = Vec::with_capacity(argv.len());
    for (index, item) in argv.iter().enumerate() {
        if let Value::String(word) = item {
            words.push(word.clone());
            continue;
        }
        match mapping::evaluate(item, scope, &mut operations) {
            Ok(Value::String(word)) => words.push(word),
            Ok(value) => {
                let message =
                    format!("`argv` item {index} evaluates to a value that is not a string");
                let details = json!({ "index": index, "value": value });
                return Err(failure("command.argv-invalid", message, details, false));
            }
            Err(error) => {
                let message = format!("`argv` item {index} does not evaluate");
                let details = error.details(&[json!("argv"), json!(index)]);
                return Err(failure(error.code(), message, details, false));
            }
        }
    }
    Ok(words)
}

/// Whether `pointer` is a JSON Pointer (RFC 6901): empty, or `/` and its steps, each
/// `~` followed by `0` or `1`.
fn is_pointer(pointer: &str) -> bool {
    (pointer.is_empty() || pointer.starts_with('/'))
        && pointer
            .split('~')
            .skip(1)
            .all(|rest| rest.starts_with(['0', '1']))
}

/// The folder the command runs in: `cwd` against the run's base folder. A `cwd` from
/// input must name an existing folder, else `command.cwd-invalid`.
fn working_folder(
    cwd: Option<&Cwd>,
    base: &Path,
    input: &Value,
) -> Result<PathBuf, ActivityOutput> {
    let pointer = match cwd {
        None => return Ok(base.to_path_buf()),
        Some(Cwd::Path(path)) => return Ok(base.join(path)),
        Some(Cwd::FromInput(pointer)) => pointer,
    };
    let invalid = |message: String, cwd: Option<&str>| {
        let mut details = json!({ "pointer": pointer });
        if let Some(cwd) = cwd {
            details["cwd"] = json!(cwd);
        }
        failure("command.cwd-invalid", message, details, false)
    };
    match input.pointer(pointer) {
        None => Err(invalid(
            format!("the input has nothing at `{pointer}` for `cwd`"),
            None,
        )),
        Some(Value::String(path)) => {
            let folder = base.join(path);
            if folder.is_dir() {
                Ok(folder)
            } else {
                let message = format!("`cwd` {} is not an existing folder", folder.display());
                Err(invalid(message, Some(path)))
            }
        }
        Some(_) => Err(invalid(
            format!("the input at `{pointer}` for `cwd` is not a string"),
            None,
        )),
    }
}

/// A command's stdout and stderr as [`drain`] read them.
type Streams<'a> = (&'a Drained, &'a Drained);

/// What [`drain`] read of a stream.
#[derive(Default)]
struct Drained {
    kept: Vec<u8>,
    /// The stream's last `TAIL` bytes, and the three before them so [`tail`] can find a
    /// character boundary.
    end: Vec<u8>,
    /// The stream's full length.
    total: usize,
}

impl Drained {
    /// The end of the stream for a failure's details.
    fn tail(&self) -> String {
        String::from_utf8_lossy(tail(&self.end, TAIL)).into_owned()
    }
}

/// Reads `reader` to EOF, keeping at most `limit` bytes from the start (`head`) or the
/// end, and the stream's end apart.
async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize, head: bool) -> Drained {
    let mut drained = Drained::default();
    let Drained { kept, end, total } = &mut drained;
    let mut buf = [0u8; 8192];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        *total += n;
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > limit {
            if head {
                kept.truncate(limit);
            } else {
                kept.drain(..kept.len() - limit);
            }
        }
        end.extend_from_slice(&buf[..n]);
        if end.len() > TAIL + 3 {
            end.drain(..end.len() - TAIL - 3);
        }
    }
    drained
}

/// A failure's `details` with the end of each stream added.
fn with_tails(mut details: Map<String, Value>, (stdout, stderr): Streams) -> Value {
    details.insert("stdoutTail".into(), Value::String(stdout.tail()));
    details.insert("stderrTail".into(), Value::String(stderr.tail()));
    Value::Object(details)
}

/// The last at most `limit` bytes of `bytes`, starting on a UTF-8 character boundary.
fn tail(bytes: &[u8], limit: usize) -> &[u8] {
    let cut = bytes.len().saturating_sub(limit);
    let continuation = |i: &usize| {
        bytes
            .get(*i)
            .is_some_and(|b| b & 0b1100_0000 == 0b1000_0000)
    };
    // A character has at most three continuation bytes; past that the bytes are not UTF-8.
    let start = if cut == 0 {
        0
    } else {
        (cut..cut + 3).find(|i| !continuation(i)).unwrap_or(cut)
    };
    &bytes[start.min(bytes.len())..]
}

/// Text-mode output: both streams whole when together they fit in `MAX_STDOUT`, else
/// each cut to its end, stderr to at most `TEXT_STDERR`. The slices hold at least the
/// ends to keep; `*_total` are the streams' full lengths.
fn text_output(
    code: i32,
    (stdout, stdout_total): (&[u8], usize),
    (stderr, stderr_total): (&[u8], usize),
) -> Value {
    let (stdout_kept, stderr_kept) = if stdout_total + stderr_total <= MAX_STDOUT {
        (stdout, stderr)
    } else {
        let stderr_kept = tail(stderr, TEXT_STDERR);
        (tail(stdout, MAX_STDOUT - stderr_kept.len()), stderr_kept)
    };
    json!({
        "exitCode": code,
        "stdout": String::from_utf8_lossy(stdout_kept),
        "stderr": String::from_utf8_lossy(stderr_kept),
        "truncated": stdout_kept.len() < stdout_total || stderr_kept.len() < stderr_total,
    })
}

/// An async start's stdout on exit 0: its holder handle, canonical-domain JSON of at
/// most 4 KiB.
fn holder_handle(
    stdout: &[u8],
    total: usize,
    fail: impl Fn(&str, String, Map<String, Value>, bool) -> ActivityOutput,
) -> ActivityOutput {
    if total > MAX_HOLDER_BYTES {
        let message = format!("the holder handle is {total} bytes; at most {MAX_HOLDER_BYTES}");
        return fail("command.output-too-large", message, Map::new(), false);
    }
    match canonical::parse(stdout) {
        Ok(holder) => ActivityOutput::Held { holder },
        Err(e) => {
            let message = format!("stdout is not a holder handle, canonical-domain JSON: {e}");
            fail("command.malformed-output", message, Map::new(), false)
        }
    }
}

/// Reads a result-mode stdout: `{"outcome", "output"}` or `{"error", "retryable"}`.
pub(crate) fn result_output(stdout: &[u8]) -> Result<ActivityOutput, String> {
    let value = canonical::parse(stdout).map_err(|e| e.to_string())?;
    let shape = "expected {\"outcome\", \"output\"} or {\"error\", \"retryable\"}";
    let Value::Object(fields) = value else {
        return Err(shape.into());
    };
    let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    keys.sort_unstable();
    match keys.as_slice() {
        ["outcome", "output"] => {
            let outcome = fields["outcome"]
                .as_str()
                .ok_or("`outcome` must be a string")?;
            Ok(ActivityOutput::Success {
                outcome: outcome.to_string(),
                output: fields["output"].clone(),
            })
        }
        ["error", "retryable"] => {
            let retryable = fields["retryable"]
                .as_bool()
                .ok_or("`retryable` must be a boolean")?;
            let error = fields["error"]
                .as_object()
                .ok_or("`error` must be an object")?;
            let field = |key: &str| error.get(key).and_then(Value::as_str);
            let (Some(code), Some(message)) = (field("code"), field("message")) else {
                return Err("`error` needs string `code` and `message`".into());
            };
            if error
                .keys()
                .any(|k| !["code", "message", "details"].contains(&k.as_str()))
            {
                return Err("`error` holds only `code`, `message` and `details`".into());
            }
            let details = error.get("details").cloned().unwrap_or(Value::Null);
            Ok(ActivityOutput::Failure {
                error: ActivityOutput::error(code, message, details),
                retryable,
            })
        }
        _ => Err(shape.into()),
    }
}

/// The result a `result` block maps from a json-mode stdout on exit 0: its `outcome`
/// mapping must give a declared outcome, its `output` mapping any value. A failure's
/// details carry the end of each stream.
fn mapped_result(
    (outcome, output): &(Value, Value),
    parsed: Value,
    streams: Streams,
    ctx: &ActivityContext,
    input: &Value,
    outcomes: &[String],
) -> ActivityOutput {
    let mut roots = ctx.scope(input);
    roots.insert("stdout".into(), parsed);
    let fail = |code: &str, message: String, details: Map<String, Value>| {
        failure(code, message, with_tails(details, streams), false)
    };
    let invalid = |field: &str, message: String, details: Value| {
        let mut all = Map::from_iter([("field".to_string(), json!(field))]);
        if let Value::Object(details) = details {
            all.extend(details);
        }
        fail("command.result-invalid", message, all)
    };
    let mut operations = 0;
    let mut evaluate = |field: &str, mapping: &Value| {
        mapping::evaluate(mapping, &roots, &mut operations).map_err(|e| {
            let message = format!("`result.{field}` did not evaluate: {}", e.code());
            invalid(field, message, e.details(&[]))
        })
    };
    let outcome = match evaluate("outcome", outcome) {
        Ok(Value::String(outcome)) => outcome,
        Ok(value) => {
            let message = "`result.outcome` is not a string".to_string();
            return invalid("outcome", message, json!({ "value": value }));
        }
        Err(failed) => return failed,
    };
    if !outcomes.contains(&outcome) {
        let message = format!("outcome `{outcome}` is not declared by the action");
        let details = Map::from_iter([("outcome".to_string(), json!(outcome))]);
        return fail("command.undeclared-outcome", message, details);
    }
    match evaluate("output", output) {
        Ok(output) => match canonical::to_vec(&output) {
            Ok(_) => ActivityOutput::Success { outcome, output },
            Err(e) => {
                let message = format!("`result.output` is not canonical-domain JSON: {e}");
                invalid("output", message, json!({ "reason": e.code() }))
            }
        },
        Err(failed) => failed,
    }
}

fn failure(code: &str, message: String, details: Value, retryable: bool) -> ActivityOutput {
    ActivityOutput::Failure {
        error: ActivityOutput::error(code, message, details),
        retryable,
    }
}

fn unknown(code: &str, message: &str, streams: Streams) -> ActivityOutput {
    ActivityOutput::Unknown {
        error: ActivityOutput::error(code, message, with_tails(Map::new(), streams)),
    }
}

/// Why the runner stopped waiting for the command.
enum Stop {
    Ended(Ending),
    TimedOut,
    Stopped,
}

#[async_trait]
impl ActivityRunner for CommandRunner {
    fn validate(&self, action_id: &str, action: &Value) -> Result<(), String> {
        parse(action)
            .map(|_| ())
            .map_err(|e| format!("action `{action_id}`: {e}"))
    }

    async fn run(&self, ctx: &ActivityContext, action: &Value, input: &Value) -> ActivityOutput {
        let spec = match parse(action) {
            Ok(spec) => spec,
            Err(e) => return failure("command.invalid", e, Value::Null, false),
        };
        let cwd = match working_folder(spec.cwd.as_ref(), &ctx.base_dir, input) {
            Ok(cwd) => cwd,
            Err(invalid) => return invalid,
        };
        let mut argv = match words(&spec.argv, &ctx.scope(input)) {
            Ok(argv) => argv,
            Err(invalid) => return invalid,
        };
        let stdin_bytes = canonical::to_vec(input).unwrap_or_default();
        let setup = |command: &mut Command| {
            prepare(command, ctx, &cwd, &spec.env);
            // An async start's work reports with the attempt's token.
            if let (true, Some(token)) = (spec.is_async, &ctx.token) {
                command
                    .env("RUNSPORE_TOKEN", token)
                    .env("RUNSPORE_RESULT_CHANNEL", STORE_CHANNEL);
            }
        };
        if let Some(program) = ctx.files.as_ref().and_then(|f| f.program(&argv[0])) {
            argv[0] = program.to_string_lossy().into_owned();
        }
        let mut guarded = match Guarded::spawn(&ctx.guard, ctx.stop_grace, &argv, setup).await {
            Ok(guarded) => guarded,
            Err(e) => return failure("command.spawn", e, Value::Null, true),
        };
        // Dropping `guarded` on these failures makes the guard kill the waiting process.
        let recorded = match process::record(guarded.pid()) {
            Ok(record) => ctx.record_process(Some(record)).await,
            Err(e) => Err(e),
        };
        if let Err(e) = recorded {
            let message = format!("the process record was not stored: {e}");
            return failure("command.spawn", message, Value::Null, true);
        }
        if let Err(e) = guarded.go().await {
            let _ = ctx.record_process(None).await;
            return failure("command.spawn", e, Value::Null, true);
        }
        let (stdin, stdout, stderr) = guarded.stdio();
        let feed = tokio::spawn(async move {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(&stdin_bytes).await;
            }
        });
        // Text mode keeps the streams' ends; the other modes need stdout from its start.
        let text = spec.output == "text";
        let stdout = tokio::spawn(drain(stdout.expect("piped"), MAX_STDOUT, !text));
        let stderr = tokio::spawn(drain(
            stderr.expect("piped"),
            if text { MAX_STDOUT } else { TAIL },
            false,
        ));
        let timeout = async {
            match spec.timeout_ms {
                Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        let ending = tokio::select! {
            ending = guarded.wait() => Stop::Ended(ending),
            () = timeout => Stop::TimedOut,
            () = ctx.stopped() => Stop::Stopped,
        };
        if !matches!(ending, Stop::Ended(_)) {
            guarded.stop().await;
        }
        // The guard has stopped the group unless it ended without reporting.
        if !matches!(ending, Stop::Ended(Ending::Lost)) {
            let _ = ctx.record_process(None).await;
        }
        feed.abort();
        let (stdout, stderr) = (
            stdout.await.unwrap_or_default(),
            stderr.await.unwrap_or_default(),
        );
        let streams = (&stdout, &stderr);
        // Every failure of a program that ran shows how it ended on both streams; one the
        // program names in result mode is its own.
        let fail = |code: &str, message: String, details: Map<String, Value>, retryable| {
            failure(code, message, with_tails(details, streams), retryable)
        };
        let code = match ending {
            Stop::TimedOut => return unknown("command.timeout", "the command timed out", streams),
            Stop::Stopped => {
                return unknown("command.stopped", "the engine stopped the command", streams);
            }
            Stop::Ended(Ending::Exited(code)) => code,
            Stop::Ended(Ending::Signaled(signal)) => {
                let message = format!("killed by signal {signal}");
                return unknown("command.signaled", &message, streams);
            }
            Stop::Ended(Ending::Lost) => {
                let message = "the guard ended without reporting";
                return unknown("command.signaled", message, streams);
            }
        };
        if spec.is_async && code == 0 {
            return holder_handle(&stdout.kept, stdout.total, fail);
        }
        let mapped = spec.exit_outcomes.iter().find(|(c, _)| *c == code);
        let outcome = match mapped {
            Some((_, name)) => name.clone(),
            None if code == 0 => "ok".to_string(),
            None => {
                let details = Map::from_iter([("exitCode".to_string(), json!(code))]);
                return fail("command.exit", format!("exit code {code}"), details, true);
            }
        };
        if !text && stdout.total > MAX_STDOUT {
            let message = format!("stdout exceeds {MAX_STDOUT} bytes");
            return fail("command.output-too-large", message, Map::new(), false);
        }
        let malformed =
            |message: String| fail("command.malformed-output", message, Map::new(), false);
        let result = match spec.output.as_str() {
            "none" => ActivityOutput::Success {
                outcome,
                output: Value::Null,
            },
            // A result block reads plain JSON: only what it maps must be in the domain.
            "json" => match &spec.result {
                Some(result) if mapped.is_none() => match serde_json::from_slice(&stdout.kept) {
                    Ok(parsed) => {
                        mapped_result(result, parsed, streams, ctx, input, &spec.outcomes)
                    }
                    Err(e) => return malformed(format!("stdout is not JSON: {e}")),
                },
                _ => match canonical::parse(&stdout.kept) {
                    Ok(parsed) => ActivityOutput::Success {
                        outcome,
                        output: parsed,
                    },
                    Err(e) => {
                        return malformed(format!("stdout is not canonical-domain JSON: {e}"))
                    }
                },
            },
            "result" if mapped.is_some() => ActivityOutput::Success {
                outcome,
                output: Value::Null,
            },
            "result" => match result_output(&stdout.kept) {
                Ok(result) => result,
                Err(e) => return malformed(format!("stdout is not a result: {e}")),
            },
            _ => ActivityOutput::Success {
                outcome,
                output: text_output(
                    code,
                    (&stdout.kept, stdout.total),
                    (&stderr.kept, stderr.total),
                ),
            },
        };
        match result {
            ActivityOutput::Success { outcome, .. } if !spec.outcomes.contains(&outcome) => {
                let message = format!("outcome `{outcome}` is not declared by the action");
                let details = Map::from_iter([("outcome".to_string(), json!(outcome))]);
                fail("command.undeclared-outcome", message, details, false)
            }
            result => result,
        }
    }
}
