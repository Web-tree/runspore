//! An async action's probe, watch and stop (spec/host.md §3, Async commands): the
//! action's own check of a holder handle, its own wait on it, and its own way to end
//! the work behind it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use runspore_types::canonical;
use serde_json::{json, Value};

use crate::activity::{ActivityContext, ActivityOutput};
use crate::command::{self, Program};

/// How long a probe may run when its program gives no `timeoutMs`.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a stop may run when its program gives no `timeoutMs`.
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// What a probe says of a holder. `Dead` and `Stopped` end the attempt, with its
/// candidate answer when it holds one (spec/host.md §2, Candidate answers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Alive,
    Dead,
    Unknown,
    /// The holder still runs, but its work has stopped.
    Stopped,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Dead => "dead",
            Self::Unknown => "unknown",
            Self::Stopped => "stopped",
        }
    }

    /// Whether the answer ends the attempt it was given for.
    pub fn ends(self) -> bool {
        matches!(self, Self::Dead | Self::Stopped)
    }
}

/// How a watch ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Watched {
    /// The job ended and the watch delivered its result.
    Result(ActivityOutput),
    /// The host lost sight of the job; the attempt stays held.
    Lost,
}

/// Runs the probe on `handle`: exit 0 with exactly `{"holder": "alive" | "dead" |
/// "unknown" | "stopped"}` is that answer; anything else reads as unknown.
pub(crate) async fn probe(
    ctx: &ActivityContext,
    program: &Program,
    input: &Value,
    handle: &[u8],
) -> Answer {
    let timeout = program
        .timeout_ms
        .map_or(PROBE_TIMEOUT, Duration::from_millis);
    let ran = command::run_program(ctx, program, input, handle.to_vec(), timeout, |_| false).await;
    let Some(ran) = ran.filter(|ran| ran.code == 0) else {
        return Answer::Unknown;
    };
    let said = canonical::parse(&ran.stdout).ok();
    let answer = said
        .as_ref()
        .and_then(Value::as_object)
        .filter(|fields| fields.len() == 1)
        .and_then(|fields| fields.get("holder")?.as_str());
    match answer {
        Some("alive") => Answer::Alive,
        Some("dead") => Answer::Dead,
        Some("stopped") => Answer::Stopped,
        _ => Answer::Unknown,
    }
}

/// Runs the stop on `handle`. Returns its exit code, `None` when it did not exit by
/// itself (a timeout or a signal) or could not be started, and its stdout.
pub(crate) async fn stop(
    ctx: &ActivityContext,
    program: &Program,
    input: &Value,
    handle: &[u8],
) -> (Option<i32>, Vec<u8>) {
    let timeout = program
        .timeout_ms
        .map_or(STOP_TIMEOUT, Duration::from_millis);
    match command::run_program(ctx, program, input, handle.to_vec(), timeout, |_| false).await {
        Some(ran) => (Some(ran.code), ran.stdout),
        None => (None, Vec::new()),
    }
}

/// What a `{"waits": "<text>"}` line says its holder waits for, or `None` for any other
/// line.
fn waits(line: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let fields = value.as_object()?;
    match (fields.len(), fields.get("waits")) {
        (1, Some(Value::String(text))) => Some(text.clone()),
        _ => None,
    }
}

/// Runs the watch on `handle` for at most `timeout` (its program's own `timeoutMs`
/// first). What each `waits` line says goes to `told`. On exit 0 the rest of stdout is one
/// result in result mode's shapes; an outcome the action does not declare is
/// `command.undeclared-outcome`. Any other ending is `Lost`.
pub(crate) async fn watch(
    ctx: &ActivityContext,
    program: &Program,
    input: &Value,
    handle: &[u8],
    timeout: Duration,
    outcomes: &[String],
    told: Arc<Mutex<Option<String>>>,
) -> Watched {
    let timeout = program.timeout_ms.map_or(timeout, Duration::from_millis);
    let take = move |line: &[u8]| match waits(line) {
        Some(text) => {
            *told.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
            true
        }
        None => false,
    };
    let ran = command::run_program(ctx, program, input, handle.to_vec(), timeout, take).await;
    let Some(ran) = ran.filter(|ran| ran.code == 0) else {
        return Watched::Lost;
    };
    match command::result_output(&ran.stdout) {
        Ok(ActivityOutput::Success { outcome, .. }) if !outcomes.contains(&outcome) => {
            let message = format!("outcome `{outcome}` is not declared by the action");
            Watched::Result(ActivityOutput::Failure {
                error: ActivityOutput::error(
                    "command.undeclared-outcome",
                    message,
                    json!({ "outcome": outcome }),
                ),
                retryable: false,
            })
        }
        Ok(result) => Watched::Result(result),
        Err(_) => Watched::Lost,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waits_line_is_told_apart() {
        assert_eq!(
            waits(br#"{"waits": "answer in pane 3"}"#),
            Some("answer in pane 3".into())
        );
        assert_eq!(waits(b"{\"waits\":\"\"}\n"), Some(String::new()));
        for other in [
            &br#"{"needs": "person", "message": "answer in pane 3"}"#[..],
            br#"{"waits": 7}"#,
            br#"{"waits": "a review", "extra": 1}"#,
            br#"{"outcome": "ok", "output": null}"#,
            b"not json",
        ] {
            assert_eq!(waits(other), None);
        }
    }
}
