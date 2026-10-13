//! The catalogue's quality checks, and its example folders as an Elm-style catalogue
//! of broken workflows: each gives exactly its codes and the text checked in beside
//! it, its first fix gives the fixed folder, and each safe fix leaves nothing new.
//!
//! `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites the
//! expected texts, the fixed folders of codes whose fix spore makes,
//! `spec/settings.schema.json` and `docs/codes/`. Review the diff: a changed message
//! shows up there.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use runspore_build::catalogue::{self, catalogue, internal};
use runspore_build::{check, Applicability, Checked, Machine};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("catalogue")
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Checks `folder` as `spore validate <name>` would, on the examples' machine.
fn checked(folder: &Path, name: &str) -> Checked {
    let mut checked = check(folder, &Machine::examples()).expect("the folder reads");
    checked.report.folder = name.to_string();
    checked
}

fn codes(checked: &Checked) -> BTreeSet<String> {
    checked.report.codes().into_iter().collect()
}

/// Every file of a folder, by its path inside it.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(dir.join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = rel.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// A copy of the example folder with `overlay` written over it, in a scratch folder.
struct Copy(PathBuf);

impl Copy {
    fn new(example: &Path, name: &str, overlay: &[(String, Vec<u8>)]) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "runspore-catalogue-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let dir = base.join(name);
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        for (path, bytes) in files(example).into_iter().chain(overlay.iter().cloned()) {
            let target = dir.join(&path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        // Symbolic links of the example come along as links.
        for (path, target) in links(example) {
            let link = dir.join(&path);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(target, link).unwrap();
        }
        Self(base)
    }

    fn folder(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn links(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if entry.file_type().unwrap().is_symlink() {
            out.push((
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_link(entry.path()).unwrap(),
            ));
        }
    }
    out
}

fn compare(path: &Path, actual: &str, failures: &mut Vec<String>, bless: bool) {
    if bless {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, actual).unwrap();
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(expected) if expected == actual => {}
        Ok(expected) => failures.push(format!(
            "{} differs:\n--- expected\n{expected}\n--- actual\n{actual}",
            path.display()
        )),
        Err(_) => failures.push(format!("{} is missing", path.display())),
    }
}

#[test]
fn every_entry_passes_the_quality_checks() {
    let mut found = Vec::new();
    for entry in &catalogue().entries {
        found.extend(catalogue::quality(entry));
        for code in &entry.also {
            if catalogue().get(code).is_none() {
                found.push(format!("{}: `also` names unknown code {code}", entry.code));
            }
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_quality_checks_catch_a_bad_entry() {
    let bad = catalogue::Catalogue::parse(
        r#"
[[code]]
code = "x.bad"
severity = "problem"
title = "Bad {thing}"
message = "Something is wrong."
fix = "do {nothing}."
args = { unused = "computed" }
example = "x"
"#,
    )
    .unwrap();
    let found = catalogue::quality(&bad.entries[0]);
    for wanted in [
        "the title has placeholders",
        "uses {nothing}, which is not declared",
        "names nothing from the author's files",
        "{unused} is declared and never used",
    ] {
        assert!(
            found.iter().any(|f| f.contains(wanted)),
            "{wanted}: {found:?}"
        );
    }
}

#[test]
fn every_example_gives_its_codes_its_text_and_its_fixed_folder() {
    let failures = examples(false);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Checks (or with `bless`, rewrites) every example's text and fixed folder.
fn examples(bless: bool) -> Vec<String> {
    let mut failures = Vec::new();
    let mut some_also = false;
    for entry in &catalogue().entries {
        let Some(name) = &entry.example else {
            assert!(internal(&entry.code), "{} has no example", entry.code);
            continue;
        };
        some_also |= !entry.also.is_empty();
        let example = root().join("examples").join(name);
        assert!(
            example.join("workflow.dot").is_file(),
            "{}: no example folder {}",
            entry.code,
            example.display()
        );
        let wrong = checked(&example, name);
        let mut wanted: BTreeSet<String> = entry.also.iter().cloned().collect();
        wanted.insert(entry.code.clone());
        if codes(&wrong) != wanted {
            failures.push(format!(
                "{}: the example gives {:?}, the catalogue says {:?}",
                entry.code,
                codes(&wrong),
                wanted
            ));
        }
        let command = format!("spore validate --short {name}");
        compare(
            &root().join("examples").join(format!("{name}.txt")),
            &wrong.report.text(&command),
            &mut failures,
            bless,
        );

        // The first fix applied to the example gives the fixed folder.
        let resolved = wrong.report.resolved();
        let fixed_dir = root().join("fixed").join(&entry.code);
        let mine = resolved.iter().find(|r| r.code == entry.code);
        if let Some(fix) = mine.and_then(|r| r.fixes.first()) {
            let (dot, settings) = wrong.report.with_fix(fix).expect("the first fix applies");
            let originals = files(&example);
            let original = |p: &str| {
                originals
                    .iter()
                    .find(|(path, _)| path == p)
                    .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_default()
            };
            let changed: Vec<(String, String)> =
                [("workflow.dot", dot), ("settings.json", settings)]
                    .into_iter()
                    .filter(|(p, text)| *text != original(p))
                    .map(|(p, t)| (p.to_string(), t))
                    .collect();
            if bless {
                let _ = std::fs::remove_dir_all(&fixed_dir);
                for (path, text) in &changed {
                    compare(&fixed_dir.join(path), text, &mut failures, true);
                }
            } else {
                let stored: Vec<(String, String)> = files(&fixed_dir)
                    .into_iter()
                    .map(|(p, b)| (p, String::from_utf8_lossy(&b).into_owned()))
                    .collect();
                if stored != changed {
                    failures.push(format!(
                        "{}: the first fix does not give {}",
                        entry.code,
                        fixed_dir.display()
                    ));
                }
            }
        }
        let overlay = files(&fixed_dir);
        if overlay.is_empty() {
            failures.push(format!(
                "{}: no fixed folder {}",
                entry.code,
                fixed_dir.display()
            ));
            continue;
        }
        let copy = Copy::new(&example, name, &overlay);
        let fixed = checked(&copy.folder(name), name);
        if codes(&fixed).contains(&entry.code) {
            failures.push(format!(
                "{}: the fixed folder still gives it: {:?}",
                entry.code,
                codes(&fixed)
            ));
        }
    }
    if !some_also {
        failures.push("no example gives more than one code".to_string());
    }
    failures
}

#[test]
fn every_safe_fix_leaves_nothing_new() {
    let mut failures = Vec::new();
    let names: BTreeSet<&String> = catalogue()
        .entries
        .iter()
        .filter_map(|e| e.example.as_ref())
        .collect();
    for name in names {
        let example = root().join("examples").join(name);
        let wrong = checked(&example, name);
        let before = wrong.report.codes();
        for r in wrong.report.resolved() {
            for fix in r
                .fixes
                .iter()
                .filter(|f| f.applicability == Applicability::Safe)
            {
                let (dot, settings) = wrong.report.with_fix(fix).expect("a safe fix applies");
                let overlay = vec![
                    ("workflow.dot".to_string(), dot.into_bytes()),
                    ("settings.json".to_string(), settings.into_bytes()),
                ];
                let copy = Copy::new(&example, name, &overlay);
                let after = checked(&copy.folder(name), name).report.codes();
                let mut left = before.clone();
                if let Some(i) = left.iter().position(|c| *c == r.code) {
                    left.remove(i);
                }
                let (mut a, mut l) = (after.clone(), left.clone());
                a.sort();
                l.sort();
                if a != l {
                    failures.push(format!(
                        "{name}: the safe fix of {} leaves {after:?}, not {left:?}",
                        r.code
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn no_example_makes_a_broken_message() {
    for entry in &catalogue().entries {
        let Some(name) = &entry.example else { continue };
        let wrong = checked(&root().join("examples").join(name), name);
        assert!(
            !wrong
                .report
                .codes()
                .iter()
                .any(|c| c == "spore.broken-message"),
            "{name}: {}",
            wrong.report.short()
        );
    }
}

#[test]
fn every_code_has_a_page_and_the_docs_are_current() {
    let failures = docs(false);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn docs(bless: bool) -> Vec<String> {
    let mut failures = Vec::new();
    let docs = repo().join("docs/codes");
    for entry in &catalogue().entries {
        let explained = runspore_build::explain(&entry.code).expect("every code explains");
        assert!(explained.contains(&entry.title), "{}", entry.code);
        if !internal(&entry.code) {
            assert!(
                explained.contains(&format!("  {}\n", entry.code)),
                "{}: the page shows no block of it:\n{explained}",
                entry.code
            );
        }
        let page = runspore_build::page(&entry.code).unwrap();
        compare(
            &docs.join(format!("{}.md", entry.code)),
            &page,
            &mut failures,
            bless,
        );
    }
    compare(
        &docs.join("README.md"),
        &runspore_build::index(),
        &mut failures,
        bless,
    );
    failures
}

#[test]
fn the_published_schema_is_the_generated_one() {
    let failures = schema(false);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn schema(bless: bool) -> Vec<String> {
    let mut failures = Vec::new();
    compare(
        &repo().join("spec/settings.schema.json"),
        &runspore_build::settings::schema_text(),
        &mut failures,
        bless,
    );
    failures
}

/// The validation guide shows the example as it is and the report spore really
/// gives, and its fixed folder is valid.
#[test]
fn the_validation_guide_shows_real_output() {
    let guide = std::fs::read_to_string(repo().join("docs/validation.md")).unwrap();
    let example = root().join("examples/nightly-backup");
    for (what, text) in [
        (
            "workflow.dot",
            std::fs::read_to_string(example.join("workflow.dot")).unwrap(),
        ),
        (
            "settings.json",
            std::fs::read_to_string(example.join("settings.json")).unwrap(),
        ),
        (
            "the report",
            std::fs::read_to_string(root().join("examples/nightly-backup.txt")).unwrap(),
        ),
    ] {
        assert!(
            guide.contains(&text),
            "the guide does not show {what} as it is"
        );
    }
    // The second ```dot and ```json blocks are the fixed folder.
    let block = |fence: &str| -> String {
        let start = guide.rmatch_indices(fence).next().unwrap().0 + fence.len();
        let end = start + guide[start..].find("```").unwrap();
        guide[start..end].to_string()
    };
    let overlay = vec![
        ("workflow.dot".to_string(), block("```dot\n").into_bytes()),
        ("settings.json".to_string(), block("```json\n").into_bytes()),
    ];
    let copy = Copy::new(&example, "nightly-backup", &overlay);
    let fixed = checked(&copy.folder("nightly-backup"), "nightly-backup");
    assert!(
        fixed.report.diagnostics.is_empty(),
        "{}",
        fixed.report.short()
    );
    assert!(fixed.built.is_some());
}

/// One mistake gives one message: what follows from a broken part is not reported.
#[test]
fn follow_on_problems_are_not_reported() {
    // (example, a code that would only repeat its mistake)
    let follow_ons = [
        // A missing arrow leaves "done" unreached; that is the same mistake.
        ("nightly-backup", "dot.unreachable"),
        // A key in the wrong place under "nodes" is not also a step of the wrong shape.
        ("nightly-backup", "settings.wrong-type"),
        // "efect" is there, misspelt: "effect" is not also missing.
        ("settings.typo-key", "action.no-effect"),
        // The settings of "build" are there under a misspelt name.
        ("settings.step-typo", "node.missing-field"),
        // An arrow with a broken outcome is not checked against the action.
        ("dot.bad-outcome", "dot.missing-arrow"),
        ("dot.bad-outcome", "dot.undeclared-outcome"),
        // A step whose kind is unknown is not checked as an activity.
        ("dot.unknown-kind", "node.missing-field"),
        // A value the package cannot hold is not also the wrong type, or too large.
        ("settings.not-integer", "settings.wrong-type"),
        ("settings.number-too-large", "settings.too-large"),
        // A broken outcome list is not checked against the arrows.
        ("action.bad-outcome", "dot.undeclared-outcome"),
        ("action.bad-outcome", "dot.missing-outcome-arrow"),
    ];
    for (name, follow_on) in follow_ons {
        let codes = checked(&root().join("examples").join(name), name)
            .report
            .codes();
        assert!(
            !codes.iter().any(|c| c == follow_on),
            "{name} also reports {follow_on}: {codes:?}"
        );
    }
}

/// Rewrites everything the tests above compare against.
#[test]
#[ignore = "rewrites the checked-in expectations; run with `-- --ignored bless`"]
fn bless() {
    let mut failures = schema(true);
    failures.extend(examples(true));
    failures.extend(docs(true));
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
