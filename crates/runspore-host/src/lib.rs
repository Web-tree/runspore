//! The Runspore engine: loads pending events, calls the reducer, commits decisions,
//! runs the activities that committed work authorizes, and reports their results.
//!
//! The engine contains no graph logic. It never decides a branch, a retry, or an
//! outcome route; it only moves durable records between the store and the reducer.

mod activity;
mod arrival;
mod command;
mod engine;
mod error;
mod files;
pub mod guard;
pub mod process;

pub use activity::{
    ActivityContext, ActivityOutput, ActivityRegistry, ActivityRunner, NativeFn, NativeRunner,
    RecordProcess,
};
pub use arrival::{Arrival, Arrow, End};
pub use command::CommandRunner;
pub use engine::{
    validate_workflow, Engine, EngineConfig, RunIds, StartOutcome, TickReport, ARTIFACT_UNREADABLE,
    FILES_DIR_MISSING, FILES_UNWRITABLE,
};
pub use error::HostError;
pub use files::StepFiles;
