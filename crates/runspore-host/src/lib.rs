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
mod external;
mod files;
pub mod guard;
pub mod process;

pub use activity::{
    ActivityContext, ActivityOutput, ActivityRegistry, ActivityRunner, NativeFn, NativeRunner,
    RecordProcess, ACTION_ROOTS,
};
pub use arrival::{Arrival, Arrow, End};
pub use command::CommandRunner;
pub use engine::{
    validate_workflow, Engine, EngineConfig, ExternalStep, RunIds, StartOutcome, StepClaim,
    StepRenewed, StepReport, StepState, TickReport, ARTIFACT_UNREADABLE, FILES_DIR_MISSING,
    FILES_UNWRITABLE, RESULT_CHANNEL,
};
pub use error::HostError;
pub use external::{StepToken, DEFAULT_TIMEOUT_MS as EXTERNAL_DEFAULT_TIMEOUT_MS};
pub use files::StepFiles;
