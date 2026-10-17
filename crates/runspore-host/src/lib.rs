//! The Runspore engine: loads pending events, calls the reducer, commits decisions,
//! runs the activities that committed work authorizes, and reports their results.
//!
//! The engine contains no graph logic. It never decides a branch, a retry, or an
//! outcome route; it only moves durable records between the store and the reducer.

mod activity;
mod arrival;
mod command;
mod component;
pub mod disk;
mod engine;
mod error;
mod external;
mod files;
pub mod guard;
mod holder;
pub mod process;
pub mod progress;

pub use activity::{
    ActivityContext, ActivityOutput, ActivityRegistry, ActivityRunner, NativeFn, NativeRunner,
    RecordProcess, ACTION_ROOTS,
};
pub use arrival::{Arrival, Arrow, End};
pub use command::CommandRunner;
pub use component::{ComponentRunner, GRANT_KINDS};
pub use engine::{
    cut_off, validate_artifact, validate_workflow, AsyncStep, Cancelled, CutOff, CutOffReason,
    Engine, EngineConfig, HeldSlot, Observer, RunIds, StartOutcome, StepClaim, StepDescription,
    StepRenewed, StepReport, StepState, Stopped, TickReport, ARTIFACT_UNREADABLE,
    CAPABILITY_MISSING, DISK_FULL, FILES_DIR_MISSING, FILES_UNWRITABLE,
};
pub use error::{HostError, Unrouted};
pub use external::{StepToken, DEFAULT_TIMEOUT_MS as EXTERNAL_DEFAULT_TIMEOUT_MS};
pub use files::StepFiles;
pub use holder::Answer;
