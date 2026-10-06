//! The Runspore engine: loads pending events, calls the reducer, commits decisions,
//! runs the activities that committed work authorizes, and reports their results.
//!
//! The engine contains no graph logic. It never decides a branch, a retry, or an
//! outcome route; it only moves durable records between the store and the reducer.

mod activity;
mod command;
mod engine;
mod error;

pub use activity::{
    ActivityContext, ActivityOutput, ActivityRegistry, ActivityRunner, NativeFn, NativeRunner,
};
pub use command::CommandRunner;
pub use engine::{Engine, EngineConfig, StartOutcome, TickReport};
pub use error::HostError;
