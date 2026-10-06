use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use runspore_types::canonical;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::activity::{ActivityContext, ActivityOutput, ActivityRunner};

/// Largest stdout a command may produce.
pub const MAX_STDOUT: usize = 64 * 1024;
/// How much of stderr is kept for a failure's details.
const STDERR_TAIL: usize = 4 * 1024;

/// The `command` runner: executes `argv` directly in its own process group.
/// Unix only.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandRunner;

struct Spec {
    argv: Vec<String>,
    cwd: Option<String>,
    env: Vec<(String, String)>,
    timeout_ms: Option<u64>,
    output: String,
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
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("`cwd` must be a string".into()),
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
        Some(Value::String(s)) if ["text", "json", "none"].contains(&s.as_str()) => s.clone(),
        Some(_) => return Err("`output` must be \"text\", \"json\" or \"none\"".into()),
    };
    let outcomes: Vec<&str> = match action.get("outcomes") {
        None => Vec::new(),
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
    Ok(Spec {
        argv,
        cwd,
        env,
        timeout_ms,
        output,
        exit_outcomes,
    })
}

/// Reads `reader` to EOF, keeping at most `limit` bytes from the start (`head`) or the end.
async fn drain(mut reader: impl AsyncRead + Unpin, limit: usize, head: bool) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut overflow = false;
    let mut buf = [0u8; 8192];
    while let Ok(n) = reader.read(&mut buf).await {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > limit {
            overflow = true;
            if head {
                kept.truncate(limit);
            } else {
                kept.drain(..kept.len() - limit);
            }
        }
    }
    (kept, overflow)
}

fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
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

enum Ending {
    Exited(std::process::ExitStatus),
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
        let stdin_bytes = canonical::to_vec(input).unwrap_or_default();
        let mut command = Command::new(&spec.argv[0]);
        command
            .args(&spec.argv[1..])
            .current_dir(ctx.base_dir.join(spec.cwd.as_deref().unwrap_or(".")))
            .envs(spec.env.iter().cloned())
            .env("RUNSPORE_RUN_ID", &ctx.key.run)
            .env("RUNSPORE_NODE_ID", &ctx.node_id)
            .env("RUNSPORE_INVOCATION_ID", &ctx.invocation_id)
            .env("RUNSPORE_ATTEMPT", ctx.attempt.to_string())
            .env("RUNSPORE_EFFECT_KEY", &ctx.effect_key)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => return failure("command.spawn", e.to_string(), Value::Null, true),
        };
        let pid = child.id();
        let stdin = child.stdin.take();
        let feed = tokio::spawn(async move {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(&stdin_bytes).await;
            }
        });
        let stdout = tokio::spawn(drain(child.stdout.take().expect("piped"), MAX_STDOUT, true));
        let stderr = tokio::spawn(drain(
            child.stderr.take().expect("piped"),
            STDERR_TAIL,
            false,
        ));
        let timeout = async {
            match spec.timeout_ms {
                Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        let ending = tokio::select! {
            status = child.wait() => match status {
                Ok(status) => Ending::Exited(status),
                Err(_) => Ending::Stopped,
            },
            () = timeout => Ending::TimedOut,
            () = ctx.stopped() => Ending::Stopped,
        };
        kill_group(pid);
        let _ = child.wait().await;
        feed.abort();
        let (stdout, overflow) = stdout.await.unwrap_or_default();
        let (stderr, _) = stderr.await.unwrap_or_default();
        let status = match ending {
            Ending::TimedOut => return unknown("command.timeout", "the command timed out"),
            Ending::Stopped => return unknown("command.stopped", "the engine stopped the command"),
            Ending::Exited(status) => status,
        };
        let Some(code) = status.code() else {
            let signal = status.signal().unwrap_or_default();
            return unknown("command.signaled", &format!("killed by signal {signal}"));
        };
        let outcome = match spec.exit_outcomes.iter().find(|(c, _)| *c == code) {
            Some((_, name)) => name.clone(),
            None if code == 0 => "ok".to_string(),
            None => {
                let details = json!({
                    "exitCode": code,
                    "stderrTail": String::from_utf8_lossy(&stderr),
                });
                return failure("command.exit", format!("exit code {code}"), details, true);
            }
        };
        if overflow {
            let message = format!("stdout exceeds {MAX_STDOUT} bytes");
            return failure("command.output-too-large", message, Value::Null, false);
        }
        let output = match spec.output.as_str() {
            "none" => Value::Null,
            "json" => match canonical::parse(&stdout) {
                Ok(value) => value,
                Err(e) => {
                    let message = format!("stdout is not canonical-domain JSON: {e}");
                    return failure("command.malformed-output", message, Value::Null, false);
                }
            },
            _ => {
                let mut text = Map::new();
                text.insert("exitCode".into(), json!(code));
                text.insert("stdout".into(), json!(String::from_utf8_lossy(&stdout)));
                Value::Object(text)
            }
        };
        ActivityOutput::Success { outcome, output }
    }
}
