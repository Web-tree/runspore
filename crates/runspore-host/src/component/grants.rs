//! A step runner's `grants` (spec/host.md §3, `component`): the programs it may start,
//! the sockets it may reach, the folders it may read and those it may write into. The
//! host enforces exactly these and never asks.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

/// The kinds of grant this host offers, sorted.
pub const KINDS: [&str; 4] = ["folders", "programs", "sockets", "writeFolders"];

/// The grant entry that names the invocation's work folder, alone or before `/<p>`.
const WORK_DIR: &str = "$RUNSPORE_WORK_DIR";

/// A grant the action does not have: its kind and the value asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denied {
    pub grant: &'static str,
    pub value: String,
}

impl Denied {
    pub fn message(&self, action_id: &str) -> String {
        let what = match self.grant {
            "writeFolders" => "write folder",
            grant => grant.trim_end_matches('s'),
        };
        format!(
            "action `{action_id}` is not granted {what} `{}`: add it to `grants.{}`",
            self.value, self.grant
        )
    }

    pub fn details(&self) -> Value {
        json!({"grant": self.grant, "value": self.value})
    }
}

/// Why `grants` cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refused {
    /// A kind of grant this host does not offer.
    Unsupported(String),
    /// Any other shape.
    Invalid(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Grants {
    pub programs: Vec<String>,
    pub sockets: Vec<String>,
    pub folders: Vec<String>,
    pub write_folders: Vec<String>,
}

impl Grants {
    /// Reads an action's `grants`; absent is none.
    pub fn parse(grants: Option<&Value>) -> Result<Self, Refused> {
        let mut parsed = Self::default();
        let Some(grants) = grants else {
            return Ok(parsed);
        };
        let Value::Object(fields) = grants else {
            return Err(Refused::Invalid("`grants` must be an object".into()));
        };
        for (kind, list) in fields {
            let into = match kind.as_str() {
                "programs" => &mut parsed.programs,
                "sockets" => &mut parsed.sockets,
                "folders" => &mut parsed.folders,
                "writeFolders" => &mut parsed.write_folders,
                _ => return Err(Refused::Unsupported(kind.clone())),
            };
            let items = list
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .map(|i| i.as_str().filter(|s| !s.is_empty()).map(String::from))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| {
                    Refused::Invalid(format!(
                        "`grants.{kind}` must be a list of non-empty strings"
                    ))
                })?;
            *into = items;
        }
        Ok(parsed)
    }

    /// `argv[0]` of a program to start, or the program of a handle: one of `programs`,
    /// as written.
    pub fn program(&self, program: &str) -> Result<(), Denied> {
        if self.programs.iter().any(|p| p == program) {
            return Ok(());
        }
        Err(Denied {
            grant: "programs",
            value: program.to_string(),
        })
    }

    /// The socket `socket` names, if it is one of `sockets`, compared once both are
    /// resolved against `base` and `home` and their `.` and `..` folded.
    pub fn socket(&self, socket: &str, places: &Places) -> Result<PathBuf, Denied> {
        let wanted = places.resolve(socket);
        if self.sockets.iter().any(|s| places.resolve(s) == wanted) {
            return Ok(wanted);
        }
        Err(Denied {
            grant: "sockets",
            value: socket.to_string(),
        })
    }

    /// The path `path` names, with its symbolic links resolved, if it lies inside one of
    /// `folders`, resolved the same way. A path that does not exist is judged by its
    /// nearest folder that does: inside a grant, the read then fails as any missing file.
    pub fn folder(&self, path: &str, places: &Places) -> Result<PathBuf, Denied> {
        let denied = || Denied {
            grant: "folders",
            value: path.to_string(),
        };
        let wanted = real(&places.resolve(path)).ok_or_else(denied)?;
        if inside(&wanted, &self.folders, places) {
            Ok(wanted)
        } else {
            Err(denied())
        }
    }

    /// The file `path` names, in its folder with symbolic links resolved, if that
    /// folder lies inside one of `writeFolders`, resolved the same way. The file itself
    /// is not followed: a write replaces whatever is there, a link included. A folder
    /// that does not exist is judged as [`Grants::folder`] judges a path; a path that
    /// names no file is refused as outside.
    pub fn write_file(&self, path: &str, places: &Places) -> Result<PathBuf, Denied> {
        let denied = || Denied {
            grant: "writeFolders",
            value: path.to_string(),
        };
        let resolved = places.resolve(path);
        let name = match resolved.components().next_back() {
            Some(Component::Normal(name)) if !path.ends_with('/') => name.to_owned(),
            _ => return Err(denied()),
        };
        let folder = resolved.parent().ok_or_else(denied)?;
        let folder = real(folder).ok_or_else(denied)?;
        if inside(&folder, &self.write_folders, places) {
            Ok(folder.join(name))
        } else {
            Err(denied())
        }
    }
}

/// `path` with its symbolic links resolved; for a path that does not exist, its nearest
/// folder that does, resolved, joined with the rest. `.` and `..` are already folded,
/// and what does not exist holds no link to follow.
fn real(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        match std::fs::canonicalize(at) {
            Ok(found) => return Some(rest.iter().rev().fold(found, |p, part| p.join(part))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                rest.push(at.file_name()?.to_owned());
                at = at.parent()?;
            }
            Err(_) => return None,
        }
    }
}

/// Whether `wanted`, its links resolved, lies inside one of `granted`, each resolved
/// the same way.
fn inside(wanted: &Path, granted: &[String], places: &Places) -> bool {
    granted.iter().any(|folder| {
        places
            .grant(folder)
            .and_then(|f| std::fs::canonicalize(f).ok())
            .is_some_and(|f| wanted.starts_with(f))
    })
}

/// Where relative and `~/` paths of one call point: the run's base folder and the
/// worker's home folder; and the invocation's work folder, which a folder grant names
/// as `$RUNSPORE_WORK_DIR`.
#[derive(Debug, Clone)]
pub(crate) struct Places {
    pub base: PathBuf,
    pub home: Option<PathBuf>,
    pub work: Option<PathBuf>,
}

impl Places {
    /// The folder a grant entry names: `$RUNSPORE_WORK_DIR[/<p>]` under the work
    /// folder, none without one; anything else as [`Places::resolve`] reads a path.
    pub fn grant(&self, entry: &str) -> Option<PathBuf> {
        let Some(rest) = entry.strip_prefix(WORK_DIR) else {
            return Some(self.resolve(entry));
        };
        let rest = match rest.strip_prefix('/') {
            Some(rest) => rest,
            None if rest.is_empty() => "",
            // `$RUNSPORE_WORK_DIRX` is a relative path like any other.
            None => return Some(self.resolve(entry)),
        };
        let work = self.work.as_ref()?;
        Some(self.resolve(&work.join(rest).to_string_lossy()))
    }

    /// `~/<p>` under the home folder, an absolute path as it is, anything else under
    /// the base folder; `.` and `..` folded without reading the disk.
    pub fn resolve(&self, path: &str) -> PathBuf {
        let joined = match (path.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ if Path::new(path).is_absolute() => PathBuf::from(path),
            _ => self.base.join(path),
        };
        let mut folded = PathBuf::new();
        for part in joined.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    folded.pop();
                }
                other => folded.push(other),
            }
        }
        folded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(base: &Path) -> Places {
        Places {
            base: base.to_path_buf(),
            home: Some(PathBuf::from("/home/someone")),
            work: None,
        }
    }

    #[test]
    fn grants_read_by_kind() {
        let grants = Grants::parse(Some(
            &json!({"programs": ["herdr"], "sockets": ["~/s.sock"]}),
        ))
        .unwrap();
        assert_eq!(grants.programs, ["herdr"]);
        assert!(grants.folders.is_empty());
        assert_eq!(Grants::parse(None).unwrap(), Grants::default());
        assert_eq!(
            Grants::parse(Some(&json!({"network": ["*"]}))),
            Err(Refused::Unsupported("network".into()))
        );
        for bad in [
            json!([]),
            json!({"programs": "herdr"}),
            json!({"programs": [""]}),
        ] {
            assert!(matches!(
                Grants::parse(Some(&bad)),
                Err(Refused::Invalid(_))
            ));
        }
    }

    #[test]
    fn programs_and_sockets_are_compared_as_granted() {
        let base = Path::new("/work/run");
        let grants = Grants {
            programs: vec!["herdr".into(), "./bin/job".into()],
            sockets: vec!["~/.herdr/api.sock".into(), "run.sock".into()],
            ..Grants::default()
        };
        assert!(grants.program("herdr").is_ok());
        assert!(grants.program("./bin/job").is_ok());
        let denied = grants.program("/usr/bin/herdr").unwrap_err();
        assert_eq!(denied.grant, "programs");
        assert!(denied
            .message("review")
            .contains("program `/usr/bin/herdr`"));
        let p = places(base);
        assert!(grants.socket("/home/someone/.herdr/api.sock", &p).is_ok());
        assert!(grants.socket("./x/../run.sock", &p).is_ok());
        assert!(grants.socket("/tmp/other.sock", &p).is_err());
    }

    #[test]
    fn a_folder_grant_covers_what_is_below_it_and_no_link_out() {
        let root = std::env::temp_dir().join(format!("runspore-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("granted/inner")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        std::fs::write(root.join("granted/inner/a.txt"), "a").unwrap();
        std::fs::write(root.join("other/b.txt"), "b").unwrap();
        std::os::unix::fs::symlink(root.join("other"), root.join("granted/out")).unwrap();
        let grants = Grants {
            folders: vec!["granted".into()],
            ..Grants::default()
        };
        let p = places(&root);
        assert!(grants.folder("granted/inner/a.txt", &p).is_ok());
        assert!(grants.folder("granted", &p).is_ok());
        assert!(grants.folder("other/b.txt", &p).is_err());
        assert!(grants.folder("granted/out/b.txt", &p).is_err());
        assert!(grants.folder("granted/../other/b.txt", &p).is_err());
        // A missing path inside the grant is the read's own error, not the grant's.
        assert!(grants.folder("granted/missing", &p).is_ok());
        assert!(grants.folder("granted/missing/deeper", &p).is_ok());
        assert!(
            grants.folder("granted/out/missing", &p).is_err(),
            "a link out"
        );
        assert!(grants.folder("other/missing", &p).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_write_grant_covers_files_of_folders_below_it_and_no_link_out() {
        let root =
            std::env::temp_dir().join(format!("runspore-write-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("out/inner")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        std::os::unix::fs::symlink(root.join("other"), root.join("out/away")).unwrap();
        let grants = Grants {
            write_folders: vec!["out".into()],
            folders: vec!["other".into()],
            ..Grants::default()
        };
        let p = places(&root);
        let real = std::fs::canonicalize(&root).unwrap();
        assert_eq!(
            grants.write_file("out/a.txt", &p).unwrap(),
            real.join("out/a.txt")
        );
        assert!(grants.write_file("out/inner/b", &p).is_ok());
        let denied = grants.write_file("other/c", &p).unwrap_err();
        assert_eq!(denied.grant, "writeFolders");
        assert!(denied
            .message("review")
            .contains("not granted write folder `other/c`: add it to `grants.writeFolders`"));
        for outside in [
            "out/away/c",
            "out/away/missing/c",
            "out/../other/c",
            "other/missing/c",
            "out",
            "out/",
            "out/inner/..",
        ] {
            assert!(grants.write_file(outside, &p).is_err(), "{outside}");
        }
        assert!(
            grants.write_file("out/missing/c", &p).is_ok(),
            "a missing folder inside the grant fails the write itself"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_work_folder_grant_names_the_invocations_work_folder() {
        let root = std::env::temp_dir().join(format!("runspore-work-grant-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work/sub")).unwrap();
        std::fs::create_dir_all(root.join("base")).unwrap();
        let grants = Grants {
            write_folders: vec!["$RUNSPORE_WORK_DIR".into()],
            folders: vec!["$RUNSPORE_WORK_DIR/sub".into()],
            ..Grants::default()
        };
        let mut p = places(&root.join("base"));
        let token = root.join("work/token");
        assert!(grants.write_file(&token.to_string_lossy(), &p).is_err());
        p.work = Some(root.join("work"));
        assert!(grants.write_file(&token.to_string_lossy(), &p).is_ok());
        assert!(grants
            .folder(&root.join("work/sub").to_string_lossy(), &p)
            .is_ok());
        assert!(grants
            .folder(&root.join("work").to_string_lossy(), &p)
            .is_err());
        assert_eq!(
            p.grant("$RUNSPORE_WORK_DIRX"),
            Some(root.join("base/$RUNSPORE_WORK_DIRX"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
