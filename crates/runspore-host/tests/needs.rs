//! What an unfinished run needs, one row of spec/cli.md's table (What a run needs) per
//! test, decided from the run's view and the facts beside it.

use std::collections::BTreeMap;

use runspore_host::needs::{need, Facts, Need};
use runspore_host::{AsyncStep, StepState};
use runspore_types::store::{Quarantine, RunKey, RunView};

/// A run that has applied every event it accepted, with `status`.
fn run(status: &str) -> RunView {
    RunView {
        key: RunKey::new("default", "run_1"),
        start_key: "key".into(),
        package_digest: "sha256:0".into(),
        artifact_digest: None,
        status: status.into(),
        revision: 3,
        applied_sequence: 2,
        next_sequence: 3,
        snapshot: None,
        snapshot_digest: None,
        quarantine: None,
        created_at_ms: 0,
        updated_at_ms: 0,
        base_dir: None,
        parent: None,
        result_channels: BTreeMap::new(),
        cancel: None,
    }
}

/// `run`, with an event accepted and not yet applied.
fn pending(status: &str) -> RunView {
    RunView {
        next_sequence: 4,
        ..run(status)
    }
}

fn step(state: StepState) -> AsyncStep {
    AsyncStep {
        kind: "external".into(),
        actor: Some("reviewer".into()),
        state,
        attempt: 1,
        invocation_id: "inv".into(),
        node_id: "review".into(),
        action_id: "review".into(),
        holder: None,
        deadline_ms: None,
        lease_until_ms: None,
        missing: None,
        waits: None,
    }
}

fn alone() -> Facts<'static> {
    Facts::default()
}

#[test]
fn a_finished_run_needs_nothing() {
    for status in ["completed", "failed", "cancelled"] {
        assert_eq!(need(&run(status), &alone()), None, "{status}");
    }
}

#[test]
fn a_quarantined_run_needs_its_release_whatever_else_holds() {
    let mut view = pending("waiting");
    view.quarantine = Some(Quarantine {
        code: "host.files-unwritable".into(),
        details: "No space left on device".into(),
        at_ms: 1,
    });
    let held = step(StepState::Held);
    let facts = Facts {
        step: Some(&held),
        cut_off: true,
        child_unfinished: true,
    };
    assert_eq!(need(&view, &facts), Some(Need::Quarantined));
}

#[test]
fn a_run_with_an_event_not_yet_applied_is_working() {
    assert_eq!(need(&pending("waiting"), &alone()), Some(Need::Working));
    assert_eq!(
        need(&pending("needs-intervention"), &alone()),
        Some(Need::Working)
    );
    assert_eq!(need(&pending("created"), &alone()), Some(Need::Working));
}

#[test]
fn a_run_at_a_step_a_worker_runs_is_working() {
    assert_eq!(need(&run("running"), &alone()), Some(Need::Working));
}

#[test]
fn a_working_run_whose_try_is_cut_off_needs_a_driver() {
    let facts = Facts {
        cut_off: true,
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::Driver));
    assert_eq!(need(&pending("running"), &facts), Some(Need::Driver));
}

#[test]
fn a_run_at_an_await_signal_node_needs_a_signal() {
    assert_eq!(need(&run("waiting"), &alone()), Some(Need::Signal));
}

#[test]
fn a_run_that_waits_while_a_run_it_started_is_unfinished_waits_on_that_child() {
    let facts = Facts {
        child_unfinished: true,
        ..alone()
    };
    assert_eq!(need(&run("waiting"), &facts), Some(Need::Child));
}

#[test]
fn a_run_that_needs_intervention_is_stuck() {
    assert_eq!(
        need(&run("needs-intervention"), &alone()),
        Some(Need::Stuck)
    );
}

#[test]
fn a_run_at_an_external_step_nobody_holds_is_external_ready() {
    let ready = step(StepState::Ready);
    let facts = Facts {
        step: Some(&ready),
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::ExternalReady));
}

#[test]
fn a_run_at_a_held_async_step_is_held_whatever_its_holder_reports() {
    let mut held = step(StepState::Held);
    let facts = Facts {
        step: Some(&held),
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::Held));
    held.waits = Some("approve the release?".into());
    let facts = Facts {
        step: Some(&held),
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::Held));
}

#[test]
fn a_run_at_a_step_this_spore_cannot_act_on_is_capability_missing() {
    let missing = step(StepState::CapabilityMissing);
    let facts = Facts {
        step: Some(&missing),
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::CapabilityMissing));
}

#[test]
fn a_held_or_external_step_is_never_cut_off() {
    let held = step(StepState::Held);
    let facts = Facts {
        step: Some(&held),
        cut_off: true,
        ..alone()
    };
    assert_eq!(need(&run("running"), &facts), Some(Need::Held));
}

#[test]
fn each_need_reads_as_the_spec_names_it() {
    let names: Vec<&str> = [
        Need::Quarantined,
        Need::Working,
        Need::Driver,
        Need::Signal,
        Need::Child,
        Need::Stuck,
        Need::ExternalReady,
        Need::Held,
        Need::CapabilityMissing,
    ]
    .iter()
    .map(|n| n.as_str())
    .collect();
    assert_eq!(
        names,
        [
            "quarantined",
            "working",
            "driver",
            "signal",
            "child",
            "stuck",
            "external-ready",
            "held",
            "capability-missing"
        ]
    );
    assert_eq!(
        serde_json::to_value(Need::ExternalReady).unwrap(),
        "external-ready"
    );
}
