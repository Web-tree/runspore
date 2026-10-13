//! The `external` action kind (spec/host.md §2, Steps outside the worker): its fields,
//! and the token that names a claimed attempt.

use std::fmt;
use std::str::FromStr;

use runspore_types::digest::ids;
use runspore_types::store::{AttemptRef, RunKey};
use serde_json::Value;

/// The kind of an action an actor outside the worker does.
pub const KIND: &str = "external";
/// `async.timeoutMs` when the action gives none: one hour.
pub const DEFAULT_TIMEOUT_MS: u64 = 3_600_000;

/// The fields of an `external` action the host acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct External {
    pub actor: Option<String>,
    pub timeout_ms: u64,
    pub lease_ms: Option<u64>,
}

impl External {
    /// The lease a claim takes and a renewal extends by: `leaseMs`, or the whole
    /// deadline when the holder is not expected to renew.
    pub fn lease_ms(&self) -> u64 {
        self.lease_ms.unwrap_or(self.timeout_ms)
    }
}

pub(crate) fn is_external(action: &Value) -> bool {
    action.get("kind").and_then(Value::as_str) == Some(KIND)
}

/// `[a-z0-9][a-z0-9.-]{0,63}`: an actor label, or the code of a reported failure.
pub(crate) fn is_label(text: &str) -> bool {
    let bytes = text.as_bytes();
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    !bytes.is_empty()
        && bytes.len() <= 64
        && allowed(&bytes[0])
        && bytes[1..].iter().all(|b| allowed(b) || *b == b'.' || *b == b'-')
}

/// A positive count of milliseconds written as a canonical decimal string.
fn millis(async_spec: &Value, field: &str) -> Result<Option<u64>, String> {
    let Some(value) = async_spec.get(field) else {
        return Ok(None);
    };
    let text = value.as_str().unwrap_or_default();
    let canonical = !text.is_empty() && !text.starts_with('0') && text.len() <= 19;
    match text.parse::<u64>() {
        Ok(ms) if canonical && ms <= i64::MAX as u64 && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(Some(ms))
        }
        _ => Err(format!(
            "`async.{field}` must be a positive decimal string of milliseconds"
        )),
    }
}

/// Reads an `external` action. Fields it does not use are left alone, as the other
/// kinds leave theirs.
pub(crate) fn parse(action: &Value) -> Result<External, String> {
    let actor = match action.get("actor") {
        None => None,
        Some(Value::String(actor)) if is_label(actor) => Some(actor.clone()),
        Some(_) => return Err("`actor` must match [a-z0-9][a-z0-9.-]{0,63}".into()),
    };
    let null = Value::Null;
    let async_spec = match action.get("async") {
        None => &null,
        Some(spec @ Value::Object(_)) => spec,
        Some(_) => return Err("`async` must be an object".into()),
    };
    let timeout_ms = millis(async_spec, "timeoutMs")?.unwrap_or(DEFAULT_TIMEOUT_MS);
    let lease_ms = millis(async_spec, "leaseMs")?;
    if lease_ms.is_some_and(|lease| lease >= timeout_ms) {
        return Err(format!(
            "`async.leaseMs` must be smaller than `async.timeoutMs` ({timeout_ms})"
        ));
    }
    Ok(External {
        actor,
        timeout_ms,
        lease_ms,
    })
}

/// The credential of a claimed attempt: its run, invocation, attempt number, owner and
/// fence, as one string. Only its bearer can renew, complete or fail the attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepToken {
    pub run: String,
    pub invocation_id: String,
    pub attempt: u32,
    pub owner: String,
    pub fence: u64,
}

/// Separates the parts of a token; the run ID, which comes first, may hold it.
const SEPARATOR: char = '~';

impl StepToken {
    pub(crate) fn of(attempt: &AttemptRef, number: u32) -> Self {
        Self {
            run: attempt.key.run.clone(),
            invocation_id: attempt.invocation_id.clone(),
            attempt: number,
            owner: attempt.owner.clone(),
            fence: attempt.fence,
        }
    }

    /// The attempt the token names, in `tenant`.
    pub(crate) fn attempt_ref(&self, tenant: &str) -> AttemptRef {
        AttemptRef {
            key: RunKey::new(tenant, &self.run),
            invocation_id: self.invocation_id.clone(),
            attempt_id: ids::attempt(&self.invocation_id, self.attempt),
            owner: self.owner.clone(),
            fence: self.fence,
        }
    }
}

impl fmt::Display for StepToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = SEPARATOR;
        write!(
            f,
            "{}{s}{}{s}{}{s}{}{s}{}",
            self.run, self.invocation_id, self.attempt, self.owner, self.fence
        )
    }
}

impl FromStr for StepToken {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let mut parts = text.rsplitn(5, SEPARATOR);
        let fence = parts.next().ok_or(())?.parse().map_err(|_| ())?;
        let owner = parts.next().ok_or(())?.to_string();
        let attempt = parts.next().ok_or(())?.parse().map_err(|_| ())?;
        let invocation_id = parts.next().ok_or(())?.to_string();
        let run = parts.next().ok_or(())?.to_string();
        if [&run, &invocation_id, &owner].iter().any(|p| p.is_empty()) {
            return Err(());
        }
        Ok(Self {
            run,
            invocation_id,
            attempt,
            owner,
            fence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_token_reads_back_as_written() {
        let token = StepToken {
            run: "run~odd".into(),
            invocation_id: "inv_1".into(),
            attempt: 2,
            owner: "actor-ab".into(),
            fence: 7,
        };
        assert_eq!(token.to_string().parse::<StepToken>(), Ok(token));
        for bad in ["", "a~b", "run~inv~x~owner~1", "run~inv~1~owner~", "~inv~1~o~1"] {
            assert!(bad.parse::<StepToken>().is_err(), "{bad}");
        }
    }

    #[test]
    fn the_fields_are_checked() {
        let defaults = parse(&json!({"kind": "external"})).unwrap();
        assert_eq!((defaults.timeout_ms, defaults.lease_ms), (DEFAULT_TIMEOUT_MS, None));
        let set = parse(&json!({"kind": "external", "actor": "reviewer",
                                "async": {"timeoutMs": "60000", "leaseMs": "5000"}}))
        .unwrap();
        assert_eq!(
            (set.actor.as_deref(), set.timeout_ms, set.lease_ms()),
            (Some("reviewer"), 60_000, 5_000)
        );
        for bad in [
            json!({"actor": "Reviewer"}),
            json!({"async": []}),
            json!({"async": {"timeoutMs": 60000}}),
            json!({"async": {"timeoutMs": "0"}}),
            json!({"async": {"timeoutMs": "060"}}),
            json!({"async": {"leaseMs": "3600000"}}),
            json!({"async": {"timeoutMs": "10", "leaseMs": "10"}}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }
}
