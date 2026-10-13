//! Every rule of the kernel's validation (spec/kernel.md §2, W01–W15) has a broken
//! folder: the kernel refuses the package the folder would give, by that rule, and
//! `spore validate` reports the build's own code instead, with no word of the
//! kernel's.

mod common;

use std::path::{Path, PathBuf};

use common::Dir;
use runspore_build::Machine;
use runspore_host::{validate_workflow, ActivityRegistry, EngineConfig, NativeRunner};
use runspore_kernel::NativeReducer;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runspore-build/catalogue/examples")
}

/// Each rule, and a catalogue example that breaks it with the code spore gives.
const RULES: [(&str, &str); 15] = [
    ("W01", "settings.wrong-format"),
    ("W02", "dot.bad-name"),
    ("W03", "dot.bad-step-name"),
    ("W04", "action.bad-name"),
    ("W05", "dot.unknown-start"),
    ("W06", "settings.too-large"),
    ("W07", "action.unknown-effect"),
    ("W08", "dot.missing-outcome-arrow"),
    ("W09", "dot.no-arrows"),
    ("W10", "mapping.unknown-operator"),
    ("W11", "node.bad-error-code"),
    ("W12", "node.limit-too-high"),
    ("W13", "node.schema-unknown-outcome"),
    ("W14", "dot.unreachable"),
    ("W15", "dot.no-way-out"),
];

/// The kernel's (and the host's) refusal of a package, as its own words.
fn kernel(package: &serde_json::Value) -> String {
    let bytes = runspore_types::canonical::to_vec(package).unwrap();
    let registry = ActivityRegistry::with_builtins(NativeRunner::new());
    let limits = EngineConfig::default().limits;
    match validate_workflow(&NativeReducer, &registry, limits, &bytes) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn every_kernel_rule_is_the_builds_own_problem() {
    let dir = Dir::new("rules");
    for (rule, code) in RULES {
        let mut folder = examples().join(code);
        if rule == "W01" {
            // 0.1 is a format the kernel reads; one it does not know breaks W01.
            folder = dir.file(code);
            std::fs::create_dir_all(&folder).unwrap();
            for file in ["workflow.dot", "settings.json"] {
                let text = std::fs::read_to_string(examples().join(code).join(file)).unwrap();
                let text = text.replace("runspore.workflow/0.1", "runspore.workflow/9");
                std::fs::write(folder.join(file), text).unwrap();
            }
        }
        let mut checked = runspore_build::check(&folder, &Machine::examples()).unwrap();
        checked.report.folder = code.to_string();
        let draft = checked.draft.as_ref().expect("the folder reads");
        let refused = kernel(draft);
        assert!(
            refused.contains(&format!("{rule}:")),
            "{rule}: the kernel does not refuse the package of {code} by it: {refused:?}"
        );
        let codes = checked.report.codes();
        assert!(codes.contains(&code.to_string()), "{rule}: {codes:?}");
        let text = checked.report.text("spore validate --short x");
        assert!(
            !text.contains(rule) && !text.contains("kernel"),
            "{rule}: the build's text shows the kernel's words:\n{text}"
        );
        let out = dir.run(&["--native-kernel", "validate", folder.to_str().unwrap()]);
        assert_eq!(out.code(), Some(2), "{rule}");
        assert!(
            out.stderr.contains(&format!("  {code}\n")),
            "{rule}: {}",
            out.stderr
        );
        assert!(!out.stderr.contains(&format!("{rule}:")), "{}", out.stderr);
    }
}
