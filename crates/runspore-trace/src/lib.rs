//! Golden kernel traces (`spec/kernel.md` section 9).
//!
//! A trace is one workflow plus an ordered list of events, each with the exact
//! decision or failure a conforming reducer must return for it. Every reducer
//! vehicle (native, Wasmtime, a transpiled component in Node or Bun) replays
//! the same traces and must agree byte for byte.
//!
//! - [`format`](mod@format): the `runspore.trace/0.1` document, [`load`] and [`save`].
//! - [`run`](mod@run): replays a trace against a
//!   [`Reducer`](runspore_types::reducer::Reducer) and reports the first
//!   [`Mismatch`]; with `RUNSPORE_BLESS=1`, [`run_file`] and [`run_dir`] rewrite
//!   the expectations instead.
//! - [`compile`](mod@compile): the same trace with every byte string already
//!   canonical, for runners that do not implement canonical JSON.
//! - [`generate`](mod@generate): a seeded generator that builds a random valid
//!   workflow and drives a reducer through it, recording what the reducer did.
//!
//! Requests are a pure function of the trace: the runner adds nothing that
//! depends on the host, the clock, or the reducer under test.

pub mod compile;
mod diff;
mod error;
pub mod format;
pub mod generate;
mod pretty;
pub mod run;
#[cfg(test)]
mod testing;

pub use compile::{compile, CompiledTrace};
pub use error::Error;
pub use format::{load, save, Event, Expect, Payload, Step, Trace};
pub use generate::{cross_check, generate};
pub use run::{
    bless, bless_file, bless_requested, run, run_compiled, run_dir, run_file, DirError, Mismatch,
};
