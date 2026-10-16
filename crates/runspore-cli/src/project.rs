//! The project folder of spec/cli.md: which database a command uses, a run's base
//! folder, the project's settings, `spore init`, the ignore file, and workflows named by
//! their graph id.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::cli::{Fail, Output};

/// The project folder, at a repository's root.
pub const FOLDER: &str = ".spore";
/// The database's file name: in the repository's folder under `$SPORE_HOME`, or, at
/// the old place, in the project folder.
pub const DATABASE: &str = "runspore.db";
const WORKFLOWS: &str = "workflows";
/// The project's settings file, in the project folder.
const SETTINGS: &str = "settings.json";
const IGNORE_FILE: &str = ".gitignore";
/// The lines the ignore file must hold.
const IGNORED: [&str; 3] = ["runspore.db", "runspore.db-*", "runspore.db.files/"];
/// What a 0.1 database leaves at a repository's root.
const LEGACY: [&str; 4] = [
    "runspore.db",
    "runspore.db-wal",
    "runspore.db-shm",
    "runspore.db.files",
];

/// The nearest folder at or above `from` that holds a `.spore/` directory.
pub fn find(from: &Path) -> Option<&Path> {
    from.ancestors().find(|dir| dir.join(FOLDER).is_dir())
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    let joined = cwd.join(path);
    std::path::absolute(&joined).unwrap_or(joined)
}

/// The database a command uses, and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    /// The database file. Always absolute.
    pub path: PathBuf,
    pub place: Place,
}

/// Where a command found its database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `--db` or `$RUNSPORE_DB`.
    Given,
    /// The old place: an existing `<dir>/.spore/runspore.db` found by walking up.
    /// `shared` is the database the repository would share without it, when that can
    /// be worked out.
    Old { shared: Option<PathBuf> },
    /// The repository's database under `$SPORE_HOME`, shared by its worktrees and
    /// clones.
    Shared,
}

/// The database a command uses: `given` (`--db`, else `$RUNSPORE_DB`), else an existing
/// `.spore/runspore.db` of the project folder found by walking up, else the database
/// every worktree and clone of the repository shares under `$SPORE_HOME`.
pub fn database(given: Option<&Path>, cwd: &Path) -> Result<Database, Fail> {
    if let Some(path) = given {
        return Ok(Database {
            path: absolute(cwd, path),
            place: Place::Given,
        });
    }
    let shared = shared(cwd);
    if let Some(old) = find(cwd)
        .map(|root| root.join(FOLDER).join(DATABASE))
        .filter(|db| db.exists())
    {
        return Ok(Database {
            path: old,
            place: Place::Old {
                shared: shared.ok(),
            },
        });
    }
    Ok(Database {
        path: shared?,
        place: Place::Shared,
    })
}

/// `$SPORE_HOME/db/<repository key>/runspore.db`, `$SPORE_HOME` being `~/.spore` when
/// unset.
fn shared(cwd: &Path) -> Result<PathBuf, Fail> {
    let home = match std::env::var_os("SPORE_HOME").filter(|home| !home.is_empty()) {
        Some(home) => absolute(cwd, Path::new(&home)),
        None => std::env::home_dir()
            .filter(|home| !home.as_os_str().is_empty())
            .ok_or_else(|| {
                Fail::usage(
                    "project.no-home",
                    "no home folder to keep the database in: set SPORE_HOME, or name the database with --db",
                )
            })?
            .join(FOLDER),
    };
    Ok(home
        .join("db")
        .join(repository_key(find(cwd).unwrap_or(cwd)))
        .join(DATABASE))
}

/// The key of the repository `dir` belongs to, as a relative path: its `origin`
/// remote as `host/owner/name`, else the path of its git common dir, which every
/// worktree of one clone shares, else, outside git, `dir`'s own path.
fn repository_key(dir: &Path) -> PathBuf {
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|out| out.status.success())?;
        let text = String::from_utf8(out.stdout).ok()?;
        Some(text.trim_end().to_string()).filter(|text| !text.is_empty())
    };
    let Some(common) = git(&["rev-parse", "--git-common-dir"]) else {
        return path_key(&real(dir));
    };
    git(&["config", "--get", "remote.origin.url"])
        .and_then(|url| origin_key(&url, dir))
        .unwrap_or_else(|| path_key(&real(&dir.join(common))))
}

/// `path` with its links resolved, else as it is.
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// An absolute path as a key: its parts below the root.
fn path_key(path: &Path) -> PathBuf {
    path.components()
        .filter_map(|part| match part {
            std::path::Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect()
}

/// An `origin` URL as `host/owner/name`: `git@github.com:Web-tree/runspore.git` and
/// `https://github.com/Web-tree/runspore` are both `github.com/Web-tree/runspore`. The
/// user, password, port and a trailing `.git` are left out, and the host is lower
/// case. A local path, plain or `file://`, is that path, taken from `dir` when
/// relative. None when nothing is left.
fn origin_key(url: &str, dir: &Path) -> Option<PathBuf> {
    let host_of = |authority: &str| {
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let host = match host.strip_prefix('[') {
            Some(v6) => v6.split_once(']').map_or(v6, |(address, _)| address),
            None => host.split_once(':').map_or(host, |(name, _)| name),
        };
        host.to_ascii_lowercase()
    };
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        (host_of(authority), path.to_string())
    } else if let Some((authority, path)) = url
        .split_once(':')
        .filter(|(authority, _)| !authority.is_empty() && !authority.contains('/'))
    {
        (host_of(authority), path.to_string())
    } else {
        let local = real(&dir.join(url));
        (String::new(), local.to_string_lossy().into_owned())
    };
    let mut parts: Vec<&str> = std::iter::once(host.as_str())
        .chain(path.split('/'))
        .filter(|part| !matches!(*part, "" | "." | ".."))
        .collect();
    let last = parts.pop()?;
    let last = last.strip_suffix(".git").unwrap_or(last);
    if !last.is_empty() {
        parts.push(last);
    }
    (!parts.is_empty()).then(|| parts.into_iter().collect())
}

/// The folder of step files and work folders: `<database path>.files`.
pub fn files_dir(db: &Path) -> PathBuf {
    let mut path = db.as_os_str().to_owned();
    path.push(".files");
    PathBuf::from(path)
}

/// `<dir>` when `db` is `<dir>/.spore/runspore.db`, however that path was given.
pub fn root_of(db: &Path) -> Option<&Path> {
    let folder = db.parent()?;
    let named = db.file_name()? == DATABASE && folder.file_name()? == FOLDER;
    named.then(|| folder.parent()).flatten()
}

/// The project's settings, from `settings.json` in the project folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `worker.slots`: how many steps one worker runs at once.
    pub slots: Option<usize>,
    /// `worker.diskFloorMb`: the free MiB a worker's disk must keep for it to take a
    /// step; 0 never pauses.
    pub disk_floor_mb: Option<u64>,
}

/// The settings of the project folder found by walking up from `cwd`: none without one
/// or when it has no `settings.json`. A file that is not JSON, holds a key spore does
/// not know, a slot count that is not a whole number from 1, or a disk floor that is
/// not a whole number from 0 is refused with exit 2.
pub fn settings(cwd: &Path) -> Result<Settings, Fail> {
    let Some(root) = find(cwd) else {
        return Ok(Settings::default());
    };
    let path = root.join(FOLDER).join(SETTINGS);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => {
            let message = format!("{}: {e}", path.display());
            return Err(Fail::internal("project.settings-unreadable", message));
        }
    };
    let invalid = |message: String| {
        Fail::usage(
            "project.settings-invalid",
            format!("{}: {message}", path.display()),
        )
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    let unknown = |object: &Map<String, Value>, at: &str, known: &[&str]| match object
        .keys()
        .find(|k| !known.contains(&k.as_str()))
    {
        Some(key) => Err(invalid(format!(
            "{at}{key} is not a setting; known: {}",
            known.join(", ")
        ))),
        None => Ok(()),
    };
    let Value::Object(top) = &value else {
        return Err(invalid("expected an object".into()));
    };
    unknown(top, "", &["worker"])?;
    let Some(worker) = top.get("worker") else {
        return Ok(Settings::default());
    };
    let Value::Object(worker) = worker else {
        return Err(invalid("worker: expected an object".into()));
    };
    unknown(worker, "worker.", &["slots", "diskFloorMb"])?;
    let slots = match worker.get("slots") {
        None => None,
        Some(n) => Some(
            n.as_u64()
                .filter(|n| *n >= 1)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| {
                    invalid(format!(
                        "worker.slots: expected a whole number from 1, got {n}"
                    ))
                })?,
        ),
    };
    let disk_floor_mb = match worker.get("diskFloorMb") {
        None => None,
        Some(n) => Some(n.as_u64().ok_or_else(|| {
            invalid(format!(
                "worker.diskFloorMb: expected a whole number from 0, got {n}"
            ))
        })?),
    };
    Ok(Settings {
        slots,
        disk_floor_mb,
    })
}

/// A run's base folder: `given` (`--base-dir`), else the folder holding the project
/// folder found by walking up from `cwd`, else `cwd`. Always absolute.
pub fn base_dir(given: Option<&Path>, cwd: &Path) -> Result<PathBuf, Fail> {
    match given {
        Some(dir) => {
            let dir = absolute(cwd, dir);
            if !dir.is_dir() {
                let message = format!("--base-dir {}: not a folder", dir.display());
                return Err(Fail::usage("base-dir.invalid", message));
            }
            Ok(dir)
        }
        None => Ok(find(cwd).unwrap_or(cwd).to_path_buf()),
    }
}

/// Before a command writes the database: create the shared database's folder, and,
/// for a database at `<dir>/.spore/runspore.db`, keep that folder's ignore file current.
pub fn before_write(db: &Database) -> Result<(), Fail> {
    if let (Place::Shared, Some(folder)) = (&db.place, db.path.parent()) {
        std::fs::create_dir_all(folder).map_err(|e| {
            Fail::internal(
                "project.database-folder",
                format!("{}: {e}", folder.display()),
            )
        })?;
    }
    let Some(root) = root_of(&db.path) else {
        return Ok(());
    };
    let folder = root.join(FOLDER);
    if !folder.is_dir() {
        return Ok(());
    }
    keep_ignore_file(&folder)
        .map_err(|e| Fail::internal("project.ignore-file", format!("{}: {e}", folder.display())))
}

/// Appends the missing ignore lines to `folder/.gitignore`, creating it when absent and
/// keeping every other line.
fn keep_ignore_file(folder: &Path) -> io::Result<()> {
    let path = folder.join(IGNORE_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let missing: Vec<&str> = IGNORED
        .into_iter()
        .filter(|line| !text.lines().any(|kept| kept.trim() == *line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut lines = String::new();
    if !text.is_empty() && !text.ends_with('\n') {
        lines.push('\n');
    }
    for line in missing {
        lines.push_str(line);
        lines.push('\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(lines.as_bytes())
}

/// `spore init`: the project folder at the repository root, or a refusal that creates
/// nothing.
pub fn init(cwd: &Path) -> Result<Output, Fail> {
    let root = cwd
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(cwd);
    if let Some(other) = cwd
        .ancestors()
        .find(|dir| *dir != root && dir.join(FOLDER).is_dir())
    {
        return Err(Fail::usage(
            "project.nested",
            format!(
                "{} already exists; the project folder belongs at the repository root {}",
                other.join(FOLDER).display(),
                root.display()
            ),
        ));
    }
    if let Some(legacy) = LEGACY
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.symlink_metadata().is_ok())
    {
        return Err(Fail::usage(
            "project.legacy-database",
            format!(
                "{} belongs to spore 0.1: finish its runs with the 0.1 binary or move it aside",
                legacy.display()
            ),
        ));
    }
    let folder = root.join(FOLDER);
    let workflows = folder.join(WORKFLOWS);
    let unusable = |path: &Path, message: String| {
        Fail::usage("project.unusable", format!("{}: {message}", path.display()))
    };
    for path in [&folder, &workflows] {
        if path.metadata().is_ok_and(|meta| !meta.is_dir()) {
            return Err(unusable(path, "exists but is not a folder".into()));
        }
    }
    let mut created = Vec::new();
    for dir in [&folder, &workflows] {
        if !dir.is_dir() {
            std::fs::create_dir(dir).map_err(|e| unusable(dir, e.to_string()))?;
            created.push(dir.clone());
        }
    }
    let text = if created.is_empty() {
        format!("project folder {} is ready", folder.display())
    } else {
        let lines: Vec<String> = created
            .iter()
            .map(|p| format!("created {}", p.display()))
            .collect();
        lines.join("\n")
    };
    Ok(Output {
        json: json!({"root": root, "created": created}),
        text,
        raw: None,
        stderr: String::new(),
        exit: 0,
    })
}

/// A workflow argument as given or, for a bare name (not an existing path, no `/`),
/// the folder under the project folder's `workflows/` whose graph id is that name.
pub fn workflow(arg: &Path, cwd: &Path) -> Result<PathBuf, Fail> {
    let Some(name) = arg.to_str() else {
        return Ok(arg.to_path_buf());
    };
    if name.is_empty() || name.contains('/') || cwd.join(arg).symlink_metadata().is_ok() {
        return Ok(arg.to_path_buf());
    }
    let Some(root) = find(cwd) else {
        return Err(Fail::usage(
            "workflow.not-found",
            format!(
                "{name} is neither a path nor a workflow: no project folder ({FOLDER}/) at or above {}",
                cwd.display()
            ),
        ));
    };
    let workflows = root.join(FOLDER).join(WORKFLOWS);
    let mut names = Vec::new();
    let mut matches = Vec::new();
    let mut broken = None;
    for folder in folders(&workflows) {
        match runspore_build::name(&folder) {
            Ok(found) if found == name => matches.push(folder),
            Ok(found) => names.push(found),
            Err(e) if folder.file_name().is_some_and(|f| f == name) => broken = Some(e),
            Err(_) => {}
        }
    }
    match (matches.len(), broken) {
        (1, _) => Ok(matches.remove(0)),
        (0, Some(e)) => Err(Fail::usage(e.code, e.to_string())),
        (0, None) => {
            names.sort();
            let known = if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            };
            Err(Fail::usage(
                "workflow.not-found",
                format!(
                    "no workflow named {name} in {}; workflows: {known}",
                    workflows.display()
                ),
            ))
        }
        _ => {
            let found: Vec<String> = matches.iter().map(|p| p.display().to_string()).collect();
            Err(Fail::usage(
                "workflow.ambiguous",
                format!(
                    "{} workflows are named {name}: {}",
                    found.len(),
                    found.join(", ")
                ),
            ))
        }
    }
}

/// The folders in `workflows`, sorted.
fn folders(workflows: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(workflows)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    folders
}

/// The workflows of the project folder found by walking up from `cwd`, by name, each
/// with its folder; a folder whose name cannot be read is left out.
pub fn workflows(cwd: &Path) -> Vec<(String, PathBuf)> {
    let Some(root) = find(cwd) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = folders(&root.join(FOLDER).join(WORKFLOWS))
        .into_iter()
        .filter_map(|folder| Some((runspore_build::name(&folder).ok()?, folder)))
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_host_owner_name_however_it_is_written() {
        let dir = Path::new("/work/repo");
        for url in [
            "git@github.com:Web-tree/runspore.git",
            "https://github.com/Web-tree/runspore",
            "https://github.com/Web-tree/runspore.git/",
            "https://user:token@GitHub.com/Web-tree/runspore",
            "ssh://git@github.com:22/Web-tree/runspore.git",
            "github.com:Web-tree/runspore",
        ] {
            assert_eq!(
                origin_key(url, dir),
                Some(PathBuf::from("github.com/Web-tree/runspore")),
                "{url}"
            );
        }
        let local = Some(PathBuf::from("srv/git/runspore"));
        assert_eq!(origin_key("/srv/git/runspore.git", dir), local);
        assert_eq!(origin_key("file:///srv/git/runspore.git", dir), local);
        assert_eq!(
            origin_key("https://example.com/../../etc", dir),
            Some(PathBuf::from("example.com/etc"))
        );
        assert_eq!(origin_key("https://", dir), None);
    }

    #[test]
    fn a_path_key_is_the_path_below_the_root() {
        assert_eq!(
            path_key(Path::new("/home/me/repo/.git")),
            PathBuf::from("home/me/repo/.git")
        );
    }
}
