//! What an unfinished run needs (spec/cli.md, What a run needs): the one answer that
//! `status`, `list` and `continue` show and that `follow` and `run_until_parked` stop on.

use serde::Serialize;

use runspore_types::model::RunStatus;
use runspore_types::store::RunView;

use crate::{AsyncStep, CutOff, StepState};

/// What an unfinished run needs, by the names of spec/cli.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Need {
    /// The run is quarantined, whatever else holds.
    Quarantined,
    /// A worker has something to do.
    Working,
    /// It would be working, but its try is cut off.
    Driver,
    /// It waits at an await-signal node.
    Signal,
    /// It waits for a signal while a run it started has not ended.
    Child,
    /// It needs intervention.
    Stuck,
    /// It is at an external step nobody holds.
    ExternalReady,
    /// It is at an async step that is held.
    Held,
    /// It is at a step this host cannot act on the way the run recorded.
    CapabilityMissing,
}

impl Need {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quarantined => "quarantined",
            Self::Working => "working",
            Self::Driver => "driver",
            Self::Signal => "signal",
            Self::Child => "child",
            Self::Stuck => "stuck",
            Self::ExternalReady => "external-ready",
            Self::Held => "held",
            Self::CapabilityMissing => "capability-missing",
        }
    }
}

/// What a run needs, with the facts beside its view it was decided from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needs {
    pub need: Option<Need>,
    /// The async step the run is at, or the step this host falls short of.
    pub step: Option<AsyncStep>,
    /// The run's claimed try when nothing works it any more.
    pub cut_off: Option<CutOff>,
}

/// What decides a run's need beside its view.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts<'a> {
    /// The async step the run is at, or the step this host falls short of.
    pub step: Option<&'a AsyncStep>,
    /// Whether the run's claimed try is cut off.
    pub cut_off: bool,
    /// Whether a run it started has not ended.
    pub child_unfinished: bool,
}

/// What the run needs; `None` once it has completed, failed or been cancelled.
pub fn need(view: &RunView, facts: &Facts) -> Option<Need> {
    if view.quarantine.is_some() {
        return Some(Need::Quarantined);
    }
    if view.ended() {
        return None;
    }
    let need = match RunStatus::parse(&view.status) {
        _ if view.pending() => Need::Working,
        Some(RunStatus::Waiting) if facts.child_unfinished => Need::Child,
        Some(RunStatus::Waiting) => Need::Signal,
        Some(RunStatus::NeedsIntervention) => Need::Stuck,
        _ => match facts.step.map(|s| s.state) {
            Some(StepState::Ready) => Need::ExternalReady,
            Some(StepState::Held) => Need::Held,
            Some(StepState::CapabilityMissing) => Need::CapabilityMissing,
            None => Need::Working,
        },
    };
    // A try held outside the worker is never cut off: its probe and its lease say.
    Some(match need {
        Need::Working if facts.cut_off && facts.step.is_none() => Need::Driver,
        need => need,
    })
}
