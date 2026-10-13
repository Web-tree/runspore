//! Workflow artifacts (`spec/artifact.md`): the manifest, its entry-path rules, and a
//! loaded artifact as the host keeps it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canonical;
use crate::digest::Digest;

/// Manifest format of this version.
pub const FORMAT: &str = "runspore.artifact/0.1";
/// The one DOT dialect an artifact of this version is built from.
pub const DIALECT: &str = "runspore/0.2";

/// The canonical JSON a component's `manifest()` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub dialect: String,
    pub name: String,
    /// The package digest; `blob(main)` returns the package bytes.
    pub main: Digest,
    /// Sorted by `path`, then owner. A file two owners name is an entry of each.
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub digest: Digest,
    pub kind: EntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Source,
    Instructions,
    Asset,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Instructions => "instructions",
            Self::Asset => "asset",
        }
    }
}

/// The node an `instructions` entry belongs to, or the action an `asset` belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    Node(String),
    Action(String),
}

/// A rejected artifact (`spec/artifact.md` §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactError {
    pub code: &'static str,
    pub message: String,
}

impl ArtifactError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("artifact.invalid", message)
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ArtifactError {}

/// Entry-path rules (`spec/artifact.md` §3) for one path; uniqueness is the
/// manifest's to check.
pub fn check_path(path: &str) -> Result<(), ArtifactError> {
    let why = if path.is_empty() {
        Some("is empty")
    } else if path.starts_with('/') {
        Some("is absolute")
    } else if path.contains('\\') {
        Some("contains a backslash")
    } else if path.contains('\0') {
        Some("contains NUL")
    } else if path.split('/').any(str::is_empty) {
        Some("has an empty segment")
    } else if path.split('/').any(|s| s == "." || s == "..") {
        Some("has a `.` or `..` segment")
    } else {
        None
    };
    match why {
        Some(why) => Err(ArtifactError::new(
            "artifact.invalid-path",
            format!("entry path {path:?} {why}"),
        )),
        None => Ok(()),
    }
}

impl Manifest {
    /// Canonical JSON bytes, as `manifest()` returns them.
    pub fn encode(&self) -> Vec<u8> {
        canonical::encode(self).expect("a manifest is always in the canonical domain")
    }

    /// Reads `manifest()` bytes: canonical JSON of the manifest shape, a format and
    /// dialect this version reads, and valid entry paths, each once per owner and
    /// naming one file, with owners where their kind needs one.
    pub fn decode(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let manifest: Self = canonical::decode(bytes)
            .map_err(|e| ArtifactError::invalid(format!("manifest: {e}")))?;
        if manifest.format != FORMAT {
            return Err(ArtifactError::new(
                "artifact.unsupported",
                format!(
                    "manifest format {:?}; this version reads {FORMAT}",
                    manifest.format
                ),
            ));
        }
        if manifest.dialect != DIALECT {
            return Err(ArtifactError::new(
                "artifact.unsupported",
                format!(
                    "dialect {:?}; this version reads {DIALECT}",
                    manifest.dialect
                ),
            ));
        }
        let mut owned_paths = BTreeSet::new();
        let mut digests = BTreeMap::new();
        for entry in &manifest.entries {
            check_path(&entry.path)?;
            if !owned_paths.insert((entry.path.as_str(), &entry.owner)) {
                return Err(ArtifactError::new(
                    "artifact.invalid-path",
                    format!("entry path {:?} appears twice for one owner", entry.path),
                ));
            }
            if *digests.entry(entry.path.as_str()).or_insert(&entry.digest) != &entry.digest {
                return Err(ArtifactError::new(
                    "artifact.invalid-path",
                    format!("entry path {:?} names two different files", entry.path),
                ));
            }
            let owned = matches!(
                (entry.kind, &entry.owner),
                (EntryKind::Source, None)
                    | (EntryKind::Instructions, Some(Owner::Node(_)))
                    | (EntryKind::Asset, Some(Owner::Action(_)))
            );
            if !owned {
                return Err(ArtifactError::invalid(format!(
                    "entry {:?}: a `{}` entry has the wrong owner",
                    entry.path,
                    entry.kind.as_str()
                )));
            }
        }
        Ok(manifest)
    }

    /// The first entry at `path`; every entry at one path holds the same file.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }
}

/// A loaded workflow artifact: the file, its manifest, and the package it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// `digest::artifact` of `bytes`.
    pub digest: Digest,
    pub bytes: Vec<u8>,
    pub manifest: Manifest,
    /// Canonical package bytes; `digest::package` of them is `manifest.main`.
    pub package: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(entries: Vec<Entry>) -> Manifest {
        Manifest {
            format: FORMAT.into(),
            dialect: DIALECT.into(),
            name: "w".into(),
            main: "sha256:00".into(),
            entries,
        }
    }

    fn source(path: &str) -> Entry {
        Entry {
            path: path.into(),
            digest: "sha256:00".into(),
            kind: EntryKind::Source,
            owner: None,
            executable: false,
        }
    }

    #[test]
    fn round_trips_through_canonical_bytes() {
        let mut m = manifest(vec![source("settings.json"), source("workflow.dot")]);
        m.entries.push(Entry {
            path: "x/run.sh".into(),
            digest: "sha256:01".into(),
            kind: EntryKind::Asset,
            owner: Some(Owner::Action("a".into())),
            executable: true,
        });
        let bytes = m.encode();
        assert!(String::from_utf8_lossy(&bytes).contains(r#""owner":{"action":"a"}"#));
        assert_eq!(Manifest::decode(&bytes), Ok(m));
    }

    #[test]
    fn rejects_unsafe_and_repeated_paths() {
        for bad in [
            "",
            "/etc/passwd",
            "a\\b",
            "a//b",
            "./a",
            "a/../b",
            "..",
            "a/",
        ] {
            let bytes = manifest(vec![source(bad)]).encode();
            assert_eq!(
                Manifest::decode(&bytes).unwrap_err().code,
                "artifact.invalid-path",
                "{bad:?}"
            );
        }
        let bytes = manifest(vec![source("a"), source("a")]).encode();
        assert_eq!(
            Manifest::decode(&bytes).unwrap_err().code,
            "artifact.invalid-path"
        );
    }

    #[test]
    fn one_file_may_have_several_owners_but_one_digest() {
        let owned = |node: &str, digest: &str| Entry {
            path: "shared.md".into(),
            digest: digest.into(),
            kind: EntryKind::Instructions,
            owner: Some(Owner::Node(node.into())),
            executable: false,
        };
        let shared = manifest(vec![owned("a", "sha256:00"), owned("b", "sha256:00")]);
        assert!(Manifest::decode(&shared.encode()).is_ok());
        let split = manifest(vec![owned("a", "sha256:00"), owned("b", "sha256:01")]);
        assert_eq!(
            Manifest::decode(&split.encode()).unwrap_err().code,
            "artifact.invalid-path"
        );
    }

    #[test]
    fn rejects_other_formats_and_dialects() {
        let mut m = manifest(vec![]);
        m.dialect = "runspore/0.1".into();
        assert_eq!(
            Manifest::decode(&m.encode()).unwrap_err().code,
            "artifact.unsupported"
        );
        let mut m = manifest(vec![]);
        m.format = "runspore.artifact/9".into();
        assert_eq!(
            Manifest::decode(&m.encode()).unwrap_err().code,
            "artifact.unsupported"
        );
        assert_eq!(
            Manifest::decode(b"{}").unwrap_err().code,
            "artifact.invalid"
        );
    }

    #[test]
    fn owners_match_kinds() {
        let mut entry = source("i.md");
        entry.kind = EntryKind::Instructions;
        let bytes = manifest(vec![entry.clone()]).encode();
        assert_eq!(
            Manifest::decode(&bytes).unwrap_err().code,
            "artifact.invalid"
        );
        entry.owner = Some(Owner::Node("n".into()));
        assert!(Manifest::decode(&manifest(vec![entry]).encode()).is_ok());
    }
}
