//! Everything that can stop a trace from being loaded, replayed, or recorded.

use std::fmt;
use std::path::PathBuf;

use runspore_types::canonical::CanonError;

use crate::run::Mismatch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A trace file or directory could not be read or written.
    Io { path: PathBuf, message: String },
    /// The text is not a trace document: bad JSON, an unknown or missing field.
    Syntax {
        path: Option<PathBuf>,
        message: String,
    },
    /// `format` names another document type or version.
    Format {
        trace: String,
        found: String,
        expected: &'static str,
    },
    /// A plain-JSON value of the trace is outside the canonical domain, so no
    /// request can be built from it. `location` is a path such as
    /// `steps[2].event.payload`.
    Fixture {
        trace: String,
        location: String,
        source: CanonError,
    },
    /// A step has no `expect` block. Record one with `RUNSPORE_BLESS=1`.
    Unblessed {
        trace: String,
        step: usize,
        event_id: String,
    },
    /// The reducer returned bytes that are not canonical JSON, so they cannot be
    /// written into an `expect` block.
    Output {
        trace: String,
        step: usize,
        event_id: String,
        location: String,
        source: CanonError,
    },
    /// The reducer answered a step differently from its `expect` block.
    Mismatch(Box<Mismatch>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Error::Syntax { path, message } => {
                if let Some(path) = path {
                    write!(f, "{}: ", path.display())?;
                }
                write!(f, "not a trace document: {message}")
            }
            Error::Format {
                trace,
                found,
                expected,
            } => write!(
                f,
                "trace `{trace}`: format is `{found}`, expected `{expected}`"
            ),
            Error::Fixture {
                trace,
                location,
                source,
            } => write!(f, "trace `{trace}`: {location} is not usable: {source}"),
            Error::Unblessed {
                trace,
                step,
                event_id,
            } => write!(
                f,
                "trace `{trace}`, step {step} (event `{event_id}`) has no `expect`; \
                 record it with RUNSPORE_BLESS=1"
            ),
            Error::Output {
                trace,
                step,
                event_id,
                location,
                source,
            } => write!(
                f,
                "trace `{trace}`, step {step} (event `{event_id}`): the reducer returned a \
                 {location} that cannot be recorded: {source}"
            ),
            Error::Mismatch(mismatch) => mismatch.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<Mismatch> for Error {
    fn from(mismatch: Mismatch) -> Self {
        Error::Mismatch(Box::new(mismatch))
    }
}
