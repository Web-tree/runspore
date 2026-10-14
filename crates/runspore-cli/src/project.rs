//! The project folder of spec/cli.md: which database a command uses, a run's base
//! folder, the project's settings, `spore init`, the ignore file, and workflows named by
//! their graph id.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::cli::{Fail, Output};

/// The project folder, at a repository's root.
pub const FOLDER: &str = ".spore";
/// The database's file name, in the project folder or, without one, the current folder.
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

/// The database a command uses: `given` (`--db`, else `$RUNSPORE_DB`), else the
/// project folder's found by walking up, else `./runspore.db`. Always absolute.
pub fn database(given: Option<&Path>, cwd: &Path) -> PathBuf {
    match (given, find(cwd)) {
        (Some(path), _) => absolute(cwd, path),
        (None, Some(root)) => root.join(FOLDER).join(DATABASE),
        (None, None) => cwd.join(DATABASE),
    }
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

/// The project's settings, from `settings.json` in the database's project folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `worker.slots`: how many steps one worker runs at once.
    pub slots: Option<usize>,
}

/// The settings of the project whose database is `db`: none when `db` is not a project
/// folder's or the folder has no `settings.json`. A file that is not JSON, holds a key
/// spore does not know, or a slot count that is not a whole number from 1 is refused
/// with exit 2.
pub fn settings(db: &Path) -> Result<Settings, Fail> {
    let Some(root) = root_of(db) else {
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
    unknown(worker, "worker.", &["slots"])?;
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
    Ok(Settings { slots })
}

/// A run's base folder: `given` (`--base-dir`), else the folder holding the database's
/// project folder, else the current folder. Always absolute.
pub fn base_dir(given: Option<&Path>, db: &Path, cwd: &Path) -> Result<PathBuf, Fail> {
    match given {
        Some(dir) => {
            let dir = absolute(cwd, dir);
            if !dir.is_dir() {
                let message = format!("--base-dir {}: not a folder", dir.display());
                return Err(Fail::usage("base-dir.invalid", message));
            }
            Ok(dir)
        }
        None => Ok(root_of(db).unwrap_or(cwd).to_path_buf()),
    }
}

/// Before a command writes the database: when it is a project folder's, keep that
/// folder's ignore file current.
pub fn before_write(db: &Path) -> Result<(), Fail> {
    let Some(root) = root_of(db) else {
        return Ok(());
    };
    let folder = root.join(FOLDER);
    if !folder.is_dir() {
        return Ok(());
    }
    keep_ignore_file(&folder)
        .map(|_| ())
        .map_err(|e| Fail::internal("project.ignore-file", format!("{}: {e}", folder.display())))
}

/// Appends the missing ignore lines to `folder/.gitignore`, creating it when absent and
/// keeping every other line. Returns whether the file was new.
fn keep_ignore_file(folder: &Path) -> io::Result<bool> {
    let path = folder.join(IGNORE_FILE);
    let (text, new) = match std::fs::read_to_string(&path) {
        Ok(text) => (text, false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (String::new(), true),
        Err(e) => return Err(e),
    };
    let missing: Vec<&str> = IGNORED
        .into_iter()
        .filter(|line| !text.lines().any(|kept| kept.trim() == *line))
        .collect();
    if missing.is_empty() {
        return Ok(false);
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
        .write_all(lines.as_bytes())?;
    Ok(new)
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
    let ignore = folder.join(IGNORE_FILE);
    let unusable = |path: &Path, message: String| {
        Fail::usage("project.unusable", format!("{}: {message}", path.display()))
    };
    for (path, folder_kind) in [(&folder, true), (&workflows, true), (&ignore, false)] {
        if let Ok(meta) = path.metadata() {
            if meta.is_dir() != folder_kind {
                let kind = if folder_kind { "a folder" } else { "a file" };
                return Err(unusable(path, format!("exists but is not {kind}")));
            }
        }
    }
    let mut created = Vec::new();
    for dir in [&folder, &workflows] {
        if !dir.is_dir() {
            std::fs::create_dir(dir).map_err(|e| unusable(dir, e.to_string()))?;
            created.push(dir.clone());
        }
    }
    if keep_ignore_file(&folder).map_err(|e| unusable(&ignore, e.to_string()))? {
        created.push(ignore);
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
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&workflows)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    let mut names = Vec::new();
    let mut matches = Vec::new();
    let mut broken = None;
    for folder in folders {
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
