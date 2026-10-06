//! Event decoding: `spec/kernel.md` section 5 and request check 4.

use runspore_types::canonical;
use runspore_types::model::{
    code, event_kind, u64_str, ActivityResult, InvocationResolved, RunStarted, SignalReceived,
};
use runspore_types::reducer::{Envelope, Failure};
use serde::de::DeserializeOwned;

use crate::failure;

/// One accepted event with its payload decoded as its kind's type.
pub(crate) enum Event {
    Started(RunStarted),
    Result(ActivityResult),
    Signal(SignalReceived),
    Resolved(InvocationResolved),
}

impl Event {
    /// The kind must be known and the payload must be the canonical encoding of that
    /// kind's type. The envelope's sequence and time must fit the counter domain,
    /// because both are recorded in the snapshot.
    pub(crate) fn decode(envelope: &Envelope) -> Result<Self, Failure> {
        let event = match envelope.kind.as_str() {
            event_kind::RUN_STARTED => Event::Started(payload(envelope)?),
            event_kind::ACTIVITY_RESULT => Event::Result(payload(envelope)?),
            event_kind::SIGNAL_RECEIVED => Event::Signal(payload(envelope)?),
            event_kind::INVOCATION_RESOLVED => Event::Resolved(payload(envelope)?),
            other => {
                return Err(failure::invalid(
                    code::EVENT_UNKNOWN_KIND,
                    format!("unknown event kind {other:?}"),
                ));
            }
        };
        if envelope.sequence > u64_str::MAX || envelope.accepted_at_ms > u64_str::MAX {
            return Err(failure::invalid(
                code::EVENT_INVALID,
                "sequence and acceptedAtMs must not exceed 2^63 - 1",
            ));
        }
        Ok(event)
    }
}

fn payload<T: DeserializeOwned>(envelope: &Envelope) -> Result<T, Failure> {
    canonical::decode(&envelope.payload).map_err(|error| {
        failure::invalid(
            code::EVENT_INVALID,
            format!("{} payload: {error}", envelope.kind),
        )
    })
}
