//! An async action's probe and watch (spec/host.md §3, Async commands): the action's
//! own check of a holder handle, and its own wait on it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use runspore_types::canonical;
use serde_json::{json, Value};

use crate::activity::{ActivityContext, ActivityOutput};
use crate::command::{self, Program};

/// How long a probe may run when its program gives no `timeoutMs`.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a probe says of a holder. Only `Dead` frees the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Alive,
    Dead,
    Unknown,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Dead => "dead",
            Self::Unknown => "unknown",
        }
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
/// "unknown"}` is that answer; anything else reads as unknown.
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
        _ => Answer::Unknown,
    }
}

/// The message of a `{"needs": "person", "message": "<text>"}` line, or `None` for any
/// other line. A line without a message gives an empty one.
fn needs(line: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let fields = value.as_object()?;
    let only = fields.keys().all(|k| k == "needs" || k == "message");
    if !only || fields.get("needs") != Some(&json!("person")) {
        return None;
    }
    match fields.get("message") {
        None => Some(String::new()),
        Some(Value::String(message)) => Some(message.clone()),
        Some(_) => None,
    }
}

/// Runs the watch on `handle` for at most `timeout` (its program's own `timeoutMs`
/// first). Each `needs` line goes to `waits`. On exit 0 the rest of stdout is one
/// result in result mode's shapes; an outcome the action does not declare is
/// `command.undeclared-outcome`. Any other ending is `Lost`.
pub(crate) async fn watch(
    ctx: &ActivityContext,
    program: &Program,
    input: &Value,
    handle: &[u8],
    timeout: Duration,
    outcomes: &[String],
    waits: Arc<Mutex<Option<String>>>,
) -> Watched {
    let timeout = program.timeout_ms.map_or(timeout, Duration::from_millis);
    let take = move |line: &[u8]| match needs(line) {
        Some(message) => {
            *waits.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
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
    fn a_needs_line_is_told_apart() {
        assert_eq!(
            needs(br#"{"needs": "person", "message": "answer in pane 3"}"#),
            Some("answer in pane 3".into())
        );
        assert_eq!(needs(b"{\"needs\":\"person\"}\n"), Some(String::new()));
        for other in [
            &br#"{"needs": "machine"}"#[..],
            br#"{"needs": "person", "message": 7}"#,
            br#"{"needs": "person", "extra": 1}"#,
            br#"{"outcome": "ok", "output": null}"#,
            b"not json",
        ] {
            assert_eq!(needs(other), None);
        }
    }
}
