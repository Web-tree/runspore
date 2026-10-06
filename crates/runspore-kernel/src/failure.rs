//! Failure constructors and the canonical encoding of everything the kernel emits.

use runspore_types::canonical::{self, CanonError};
use runspore_types::reducer::{Failure, FailureKind};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::code;

fn failure(kind: FailureKind, code: &str, details: impl Into<String>) -> Failure {
    Failure {
        kind,
        code: code.to_string(),
        details: details.into(),
    }
}

pub(crate) fn invalid(code: &str, details: impl Into<String>) -> Failure {
    failure(FailureKind::InvalidInput, code, details)
}

pub(crate) fn incompatible(code: &str, details: impl Into<String>) -> Failure {
    failure(FailureKind::IncompatibleVersion, code, details)
}

pub(crate) fn resource(code: &str, details: impl Into<String>) -> Failure {
    failure(FailureKind::ResourceLimit, code, details)
}

/// A condition the request checks rule out. Reaching one is a kernel defect, and it
/// is still reported as a failure, never as a trap.
pub(crate) fn invariant(details: impl Into<String>) -> Failure {
    failure(
        FailureKind::InvariantViolation,
        code::KERNEL_INVARIANT,
        details,
    )
}

/// Canonical bytes of a typed value the kernel emits.
pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, Failure> {
    canonical::encode(value).map_err(unencodable)
}

/// Canonical bytes of a JSON value the kernel emits.
pub(crate) fn to_vec(value: &Value) -> Result<Vec<u8>, Failure> {
    canonical::to_vec(value).map_err(unencodable)
}

/// Inputs are canonical, so their integers and counters are in range; only nesting
/// can leave the domain, when a value is recorded deeper than it arrived.
fn unencodable(error: CanonError) -> Failure {
    match error {
        CanonError::TooDeep => resource(code::BUDGET_VALUE_DEPTH, error.to_string()),
        other => invariant(format!("emitted value is not canonical: {other}")),
    }
}

/// A JSON object with the given members.
pub(crate) fn object<const N: usize>(members: [(&str, Value); N]) -> Value {
    let mut map = Map::new();
    for (key, value) in members {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}
