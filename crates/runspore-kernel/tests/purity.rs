//! The kernel stays pure and portable: its sources never mention a construct that
//! could make output depend on the host, and its manifest pulls in nothing new.

use std::fs;
use std::path::{Path, PathBuf};

/// Matched as plain substrings, comments included, so a later edit cannot
/// reintroduce one of them unnoticed.
const FORBIDDEN: &[&str] = &[
    "HashMap",
    "HashSet",
    "f32",
    "f64",
    "std::time",
    "std::env",
    "std::fs",
    "std::thread",
    "static mut",
    "thread_local",
    "unwrap(",
    "expect(",
    "unsafe",
    "Instant",
    "SystemTime",
    "panic!",
    "unreachable!",
    "todo!",
    "unimplemented!",
];

/// Matched as whole identifiers, so `operand` is not mistaken for the crate.
const FORBIDDEN_WORDS: &[&str] = &["rand"];

const ALLOWED_DEPENDENCIES: &[&str] = &["runspore-types", "serde", "serde_json"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src is readable") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(start, _)| {
        let bytes = line.as_bytes();
        let end = start + word.len();
        let before = start == 0 || !is_identifier(bytes[start - 1]);
        let after = end == bytes.len() || !is_identifier(bytes[end]);
        before && after
    })
}

#[test]
fn sources_use_no_forbidden_construct() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no sources under {}", root.display());

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("source is UTF-8");
        for (number, line) in text.lines().enumerate() {
            let plain = FORBIDDEN.iter().filter(|item| line.contains(**item));
            let words = FORBIDDEN_WORDS
                .iter()
                .filter(|word| contains_word(line, word));
            for item in plain.chain(words) {
                violations.push(format!("{}:{}: {item}", file.display(), number + 1));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn the_word_matcher_respects_identifier_boundaries() {
    assert!(contains_word("use rand::Rng;", "rand"));
    assert!(contains_word("rand", "rand"));
    assert!(contains_word("let x = rand ();", "rand"));
    assert!(!contains_word("the left operand", "rand"));
    assert!(!contains_word("random_value", "rand"));
    assert!(!contains_word("my_rand", "rand"));
}

#[test]
fn the_manifest_depends_only_on_the_contract_crates() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = fs::read_to_string(manifest).expect("manifest is readable");
    let mut section = String::new();
    let mut dependencies = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_string();
            assert!(
                !section.starts_with("[target") && section != "[build-dependencies]",
                "unexpected dependency section {section}"
            );
        } else if section == "[dependencies]" && !line.is_empty() && !line.starts_with('#') {
            let name = line
                .split(['.', '=', ' '])
                .next()
                .expect("split yields a first item");
            dependencies.push(name.to_string());
        }
    }
    dependencies.sort();
    assert_eq!(dependencies, ALLOWED_DEPENDENCIES);
}
