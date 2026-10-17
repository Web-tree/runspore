//! A step runner's `grants` (spec/host.md §3, `component`): the programs it may start,
//! the sockets it may reach and the folders it may read. The host enforces exactly
//! these and never asks.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

/// The kinds of grant this host offers, sorted.
pub const KINDS: [&str; 3] = ["folders", "programs", "sockets"];

/// A grant the action does not have: its kind and the value asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Denied {
    pub grant: &'static str,
    pub value: String,
}

impl Denied {
    pub fn message(&self, action_id: &str) -> String {
        let what = self.grant.trim_end_matches('s');
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
    /// `folders`, resolved the same way. A path that does not exist is refused as
    /// outside: nothing can be read there.
    pub fn folder(&self, path: &str, places: &Places) -> Result<PathBuf, Denied> {
        let denied = || Denied {
            grant: "folders",
            value: path.to_string(),
        };
        let wanted = std::fs::canonicalize(places.resolve(path)).map_err(|_| denied())?;
        let inside = self.folders.iter().any(|folder| {
            std::fs::canonicalize(places.resolve(folder)).is_ok_and(|f| wanted.starts_with(f))
        });
        if inside {
            Ok(wanted)
        } else {
            Err(denied())
        }
    }
}

/// Where relative and `~/` paths of one call point: the run's base folder and the
/// worker's home folder.
#[derive(Debug, Clone)]
pub(crate) struct Places {
    pub base: PathBuf,
    pub home: Option<PathBuf>,
}

impl Places {
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
            folders: Vec::new(),
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
        assert!(grants.folder("granted/missing", &p).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
