use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use runspore_types::canonical;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::activity::{ActivityContext, ActivityOutput, ActivityRunner};
use crate::guard::{Ending, Guarded};
use crate::process;

/// Largest stdout a command may produce, and the most of both streams text mode keeps.
pub const MAX_STDOUT: usize = 64 * 1024;
/// How much of stderr text mode keeps once the streams must be cut.
const TEXT_STDERR: usize = 16 * 1024;
/// How much of stderr is kept for a failure's details.
const STDERR_TAIL: usize = 4 * 1024;

/// The `command` runner: executes `argv` directly in its own process group, under the
/// guard `ctx.guard`. Unix only.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandRunner;

/// Where a command runs: a path, or a JSON Pointer into the node's input naming one.
enum Cwd {
    Path(String),
    FromInput(String),
}

struct Spec {
    argv: Vec<String>,
    cwd: Option<Cwd>,
    env: Vec<(String, String)>,
    timeout_ms: Option<u64>,
    output: String,
    outcomes: Vec<String>,
    exit_outcomes: Vec<(i32, String)>,
}

fn parse(action: &Value) -> Result<Spec, String> {
    let argv: Vec<String> = action
        .get("argv")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or("`argv` must be a non-empty array of strings")?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect::<Option<_>>()
        .ok_or("`argv` must be a non-empty array of strings")?;
    let cwd = match action.get("cwd") {
        None => None,
        Some(Value::String(s)) => Some(Cwd::Path(s.clone())),
        Some(Value::Object(m)) => match m.get("fromInput") {
            Some(Value::String(pointer)) if m.len() == 1 && is_pointer(pointer) => {
                Some(Cwd::FromInput(pointer.clone()))
            }
            _ => return Err(CWD_SHAPE.into()),
        },
        Some(_) => return Err(CWD_SHAPE.into()),
    };
    let env = match action.get("env") {
        None => Vec::new(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect::<Option<_>>()
            .ok_or("`env` values must be strings")?,
        Some(_) => return Err("`env` must be an object of strings".into()),
    };
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
    if output == "result" && exit_outcomes.iter().any(|(code, _)| *code == 0) {
        return Err("in result mode exit 0 reads stdout, so `exitOutcomes` cannot map 0".into());
    }
    Ok(Spec {
        argv,
        cwd,
        env,
        timeout_ms,
        output,
        outcomes: outcomes.into_iter().map(str::to_string).collect(),
        exit_outcomes,
    })
}

const CWD_SHAPE: &str = "`cwd` must be a string or {\"fromInput\": \"<JSON Pointer>\"}";

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

/// Reads `reader` to EOF, keeping at most `limit` bytes from the start (`head`) or the
/// end. Returns them with the stream's full length.
async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize, head: bool) -> (Vec<u8>, usize) {
    let mut kept = Vec::new();
    let mut total = 0;
    let mut buf = [0u8; 8192];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        total += n;
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > limit {
            if head {
                kept.truncate(limit);
            } else {
                kept.drain(..kept.len() - limit);
            }
        }
    }
    (kept, total)
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

/// Reads a result-mode stdout: `{"outcome", "output"}` or `{"error", "retryable"}`.
fn result_output(stdout: &[u8]) -> Result<ActivityOutput, String> {
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

fn failure(code: &str, message: String, details: Value, retryable: bool) -> ActivityOutput {
    ActivityOutput::Failure {
        error: ActivityOutput::error(code, message, details),
        retryable,
    }
}

fn unknown(code: &str, message: &str) -> ActivityOutput {
    ActivityOutput::Unknown {
        error: ActivityOutput::error(code, message, Value::Null),
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
        let stdin_bytes = canonical::to_vec(input).unwrap_or_default();
        let setup = |command: &mut Command| {
            command
                .current_dir(cwd)
                .envs(spec.env.iter().cloned())
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
            // After `env` too; one the step lacks is removed, so a value the worker
            // inherited never reaches it.
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
        };
        let mut argv = spec.argv.clone();
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
            if text { MAX_STDOUT } else { STDERR_TAIL },
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
        let (stdout, stdout_total) = stdout.await.unwrap_or_default();
        let (stderr, stderr_total) = stderr.await.unwrap_or_default();
        let code = match ending {
            Stop::TimedOut => return unknown("command.timeout", "the command timed out"),
            Stop::Stopped => return unknown("command.stopped", "the engine stopped the command"),
            Stop::Ended(Ending::Exited(code)) => code,
            Stop::Ended(Ending::Signaled(signal)) => {
                return unknown("command.signaled", &format!("killed by signal {signal}"));
            }
            Stop::Ended(Ending::Lost) => {
                return unknown("command.signaled", "the guard ended without reporting");
            }
        };
        let mapped = spec.exit_outcomes.iter().find(|(c, _)| *c == code);
        let outcome = match mapped {
            Some((_, name)) => name.clone(),
            None if code == 0 => "ok".to_string(),
            None => {
                let details = json!({
                    "exitCode": code,
                    "stderrTail": String::from_utf8_lossy(tail(&stderr, STDERR_TAIL)),
                });
                return failure("command.exit", format!("exit code {code}"), details, true);
            }
        };
        if !text && stdout_total > MAX_STDOUT {
            let message = format!("stdout exceeds {MAX_STDOUT} bytes");
            return failure("command.output-too-large", message, Value::Null, false);
        }
        let result = match spec.output.as_str() {
            "none" => ActivityOutput::Success {
                outcome,
                output: Value::Null,
            },
            "json" => match canonical::parse(&stdout) {
                Ok(output) => ActivityOutput::Success { outcome, output },
                Err(e) => {
                    let message = format!("stdout is not canonical-domain JSON: {e}");
                    return failure("command.malformed-output", message, Value::Null, false);
                }
            },
            "result" if mapped.is_some() => ActivityOutput::Success {
                outcome,
                output: Value::Null,
            },
            "result" => match result_output(&stdout) {
                Ok(result) => result,
                Err(e) => {
                    let message = format!("stdout is not a result: {e}");
                    return failure("command.malformed-output", message, Value::Null, false);
                }
            },
            _ => ActivityOutput::Success {
                outcome,
                output: text_output(code, (&stdout, stdout_total), (&stderr, stderr_total)),
            },
        };
        match result {
            ActivityOutput::Success { outcome, .. } if !spec.outcomes.contains(&outcome) => {
                let message = format!("outcome `{outcome}` is not declared by the action");
                let details = json!({ "outcome": outcome });
                failure("command.undeclared-outcome", message, details, false)
            }
            result => result,
        }
    }
}
