//! Folders at the edges of what the build reads: the end of a file, one-line graphs,
//! keys given twice, strings over two lines, and fixes that change a container's
//! only member. Each diagnostic resolves, and each fix gives the folder it promises.

use std::path::PathBuf;

use runspore_build::{check, Applicability, Checked, Machine};

const DOT: &str = "digraph \"release\" {\n  dialect = \"runspore/0.2\";\n  start   = \"build\";\n\n  build [label=\"Build\"];\n  done  [kind=\"complete\"];\n\n  build -> done;\n}\n";

const SETTINGS: &str = "{\n  \"format\": \"runspore.workflow/0.2\",\n  \"actions\": {\n    \"build\": {\n      \"kind\": \"command\",\n      \"argv\": [\"make\"],\n      \"effect\": \"idempotent\"\n    }\n  },\n  \"nodes\": {\n    \"build\": { \"action\": \"build\" }\n  }\n}\n";

/// A folder of these two files in a scratch directory, removed on drop.
struct Folder(PathBuf);

impl Folder {
    fn new(dot: &str, settings: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "runspore-edges-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("workflow.dot"), dot).unwrap();
        std::fs::write(dir.join("settings.json"), settings).unwrap();
        Self(dir)
    }

    fn check(&self) -> Checked {
        check(&self.0, &Machine::examples()).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn codes(checked: &Checked) -> Vec<String> {
    checked.report.codes()
}

/// The two files after the first fix of the diagnostic with `code`.
fn fixed(checked: &Checked, code: &str) -> (String, String) {
    let resolved = checked.report.resolved();
    let r = resolved
        .iter()
        .find(|r| r.code == code)
        .unwrap_or_else(|| panic!("no {code}: {}", checked.report.short()));
    checked
        .report
        .with_fix(&r.fixes[0])
        .expect("the fix applies")
}

#[test]
fn a_settings_file_that_ends_early_is_not_json_at_its_end() {
    for settings in [
        "{\n  \"format\": \"runspore.workflow/0.2\",\n",
        "{\n",
        "\n",
        "{\"a\": \"é",
    ] {
        let checked = Folder::new(DOT, settings).check();
        assert_eq!(codes(&checked), ["settings.not-json"], "{settings:?}");
        let r = &checked.report.resolved()[0];
        assert_eq!(r.file, runspore_build::edit::File::Settings);
    }
    // The column counts characters, not bytes.
    let checked = Folder::new(DOT, "{\"é\": 1,, }").check();
    let r = &checked.report.resolved()[0];
    assert_eq!(r.start, (1, 9), "{}", checked.report.short());
}

#[test]
fn a_repeated_key_loses_only_its_first_copy() {
    let settings = SETTINGS.replace(
        "\"effect\": \"idempotent\"",
        "\"effect\": \"idempotent\",\n      \"retry\": {\"maxAttempts\": 2, \"maxAttempts\": 3}",
    );
    let checked = Folder::new(DOT, &settings).check();
    assert_eq!(codes(&checked), ["settings.repeated-key"]);
    let (_, after) = fixed(&checked, "settings.repeated-key");
    assert!(after.contains("\"retry\": {\"maxAttempts\": 3}"), "{after}");
    let applied = checked.report.apply(Applicability::Safe);
    assert_eq!(applied.settings.as_deref(), Some(after.as_str()));

    let top = SETTINGS.replacen(
        "\"format\": \"runspore.workflow/0.2\",",
        "\"format\": \"runspore.workflow/0.1\",\n  \"format\": \"runspore.workflow/0.2\",",
        1,
    );
    let checked = Folder::new(DOT, &top).check();
    let (_, after) = fixed(&checked, "settings.repeated-key");
    assert_eq!(after, SETTINGS);
}

#[test]
fn signal_steps_with_and_without_schemas_differ() {
    let dot = "digraph \"release\" {\n  dialect = \"runspore/0.2\";\n  start   = \"a\";\n  a [kind=\"await-signal\"];\n  b [kind=\"await-signal\"];\n  done [kind=\"complete\"];\n  a -> b [on=\"ok\"];\n  b -> done [on=\"ok\"];\n}\n";
    let settings = "{\n  \"format\": \"runspore.workflow/0.2\",\n  \"nodes\": {\n    \"a\": {\"signal\": \"go\", \"outputSchemas\": {\"ok\": {\"type\": \"object\"}}},\n    \"b\": {\"signal\": \"go\"}\n  }\n}\n";
    let checked = Folder::new(dot, settings).check();
    assert_eq!(codes(&checked), ["node.signal-schemas-differ"]);
}

#[test]
fn a_step_key_alone_under_nodes_becomes_the_step() {
    let settings = SETTINGS.replace(
        "\"build\": { \"action\": \"build\" }",
        "\"action\": \"build\"",
    );
    let checked = Folder::new(DOT, &settings).check();
    let all = codes(&checked);
    assert!(
        all.contains(&"settings.misplaced-step-key".to_string()),
        "{all:?}"
    );
    let (_, after) = fixed(&checked, "settings.misplaced-step-key");
    assert_eq!(
        after,
        SETTINGS.replace("{ \"action\": \"build\" }", "{\"action\": \"build\"}")
    );
}

#[test]
fn fixes_to_a_one_line_graph_stay_inside_it() {
    let one_line =
        "digraph \"release\" { start = \"build\"; build -> done; done [kind=\"complete\"]; }\n";
    let checked = Folder::new(one_line, SETTINGS).check();
    assert_eq!(codes(&checked), ["dot.no-dialect"]);
    let (dot, _) = fixed(&checked, "dot.no-dialect");
    let again = Folder::new(&dot, SETTINGS).check();
    assert!(codes(&again).is_empty(), "{dot}\n{}", again.report.short());

    let no_start = "digraph \"release\" { dialect = \"runspore/0.2\"; build -> done; done [kind=\"complete\"]; }\n";
    let checked = Folder::new(no_start, SETTINGS).check();
    let (dot, _) = fixed(&checked, "dot.no-start");
    let again = Folder::new(&dot, SETTINGS).check();
    assert!(codes(&again).is_empty(), "{dot}\n{}", again.report.short());

    let no_arrow = "digraph \"release\" { dialect = \"runspore/0.2\"; start = \"build\"; build; done [kind=\"complete\"]; }\n";
    let checked = Folder::new(no_arrow, SETTINGS).check();
    let (dot, _) = fixed(&checked, "dot.missing-arrow");
    let again = Folder::new(&dot, SETTINGS).check();
    assert!(codes(&again).is_empty(), "{dot}\n{}", again.report.short());
}

#[test]
fn a_misspelt_step_key_is_not_also_missing() {
    let settings = SETTINGS.replace("{ \"action\": \"build\" }", "{ \"acton\": \"build\" }");
    let checked = Folder::new(DOT, &settings).check();
    assert_eq!(codes(&checked), ["settings.typo-key"]);
}

#[test]
fn a_retry_wait_past_what_the_kernel_holds_is_too_large() {
    let settings = SETTINGS.replace(
        "\"effect\": \"idempotent\"",
        "\"effect\": \"idempotent\",\n      \"retry\": {\"backoffMs\": \"9300000000000000000\"}",
    );
    let checked = Folder::new(DOT, &settings).check();
    assert_eq!(codes(&checked), ["settings.too-large"]);
}

#[test]
fn a_string_over_two_lines_is_replaced_whole() {
    let dot = DOT.replace("\"runspore/0.2\"", "\"runspore/\" +\n  \"0.1\"");
    let checked = Folder::new(&dot, SETTINGS).check();
    assert_eq!(codes(&checked), ["dot.old-dialect"]);
    let (after, _) = fixed(&checked, "dot.old-dialect");
    assert_eq!(after, DOT);
}
