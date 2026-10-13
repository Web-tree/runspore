//! Step files and work folders (spec/host.md §3): a step's declared files copied
//! read-only out of the run's artifact, checked before every use, and a writable
//! folder per invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nix::fcntl::{Flock, FlockArg};
use runspore_types::artifact::{self, ArtifactError, EntryKind, Manifest, Owner};
use runspore_types::digest;

const FILE_MODE: u32 = 0o444;
const EXECUTABLE_MODE: u32 = 0o555;
const FOLDER_MODE: u32 = 0o555;
const WORK_MODE: u32 = 0o700;
/// Prefix of the folders a sweep or a repair moves aside before deleting them.
const TRASH: &str = ".trash-";

/// A step's files as a program finds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFiles {
    /// `<files_dir>/steps/<artifact>/<node>/`, absolute.
    pub dir: PathBuf,
    /// The node's instructions file in `dir`, if it has one.
    pub instructions: Option<PathBuf>,
    /// Folder-relative paths of the action's assets.
    pub assets: BTreeSet<String>,
}

impl StepFiles {
    /// The step files' copy of `argv0` when it reads `./<p>` and `<p>` is an asset of
    /// the action; any other `./…` stays relative to `cwd`.
    pub fn program(&self, argv0: &str) -> Option<PathBuf> {
        let path = argv0.strip_prefix("./")?;
        self.assets.contains(path).then(|| self.dir.join(path))
    }
}

/// One file the step files of a node hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Want {
    pub digest: String,
    pub executable: bool,
}

impl Want {
    fn mode(&self) -> u32 {
        if self.executable {
            EXECUTABLE_MODE
        } else {
            FILE_MODE
        }
    }
}

/// The files of `node`, whose action is `action`: its instructions entry and its
/// action's assets, by path. Also returns the instructions path and the asset paths.
pub(crate) fn wanted(
    manifest: &Manifest,
    node: &str,
    action: Option<&str>,
) -> (BTreeMap<String, Want>, Option<String>, BTreeSet<String>) {
    let mut files = BTreeMap::new();
    let mut instructions = None;
    let mut assets = BTreeSet::new();
    for entry in &manifest.entries {
        let owned = match (&entry.kind, &entry.owner) {
            (EntryKind::Instructions, Some(Owner::Node(n))) if n == node => {
                instructions = Some(entry.path.clone());
                true
            }
            (EntryKind::Asset, Some(Owner::Action(a))) if Some(a.as_str()) == action => {
                assets.insert(entry.path.clone());
                true
            }
            _ => false,
        };
        if owned {
            files.insert(
                entry.path.clone(),
                Want {
                    digest: entry.digest.clone(),
                    executable: entry.executable,
                },
            );
        }
    }
    (files, instructions, assets)
}

/// Whether the manifest has any file a step could be given.
pub(crate) fn declares_files(manifest: &Manifest) -> bool {
    manifest
        .entries
        .iter()
        .any(|e| matches!(e.kind, EntryKind::Instructions | EntryKind::Asset))
}

/// Why step files could not be made.
#[derive(Debug)]
pub(crate) enum Unmade {
    /// The artifact or one of its blobs could not be read: `artifact.unreadable`.
    Unreadable(String),
    /// The files folder could not be written: `host.files-unwritable`.
    Unwritable(String),
    /// The artifact declares files and the engine has no files folder:
    /// `host.files-dir-missing`.
    DirMissing,
}

impl From<ArtifactError> for Unmade {
    fn from(e: ArtifactError) -> Self {
        Self::Unreadable(format!("{}: {}", e.code, e.message))
    }
}

impl From<io::Error> for Unmade {
    fn from(e: io::Error) -> Self {
        Self::Unwritable(e.to_string())
    }
}

/// The layout of a files folder.
#[derive(Debug, Clone)]
pub(crate) struct FilesDir {
    root: PathBuf,
}

/// The hexadecimal part of an artifact digest, or `None` when it is not one: it is a
/// path segment.
fn hex(artifact: &str) -> Option<&str> {
    let hex = artifact.strip_prefix("sha256:")?;
    (!hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

fn unique() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

impl FilesDir {
    pub fn new(root: &Path) -> Self {
        Self {
            root: std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf()),
        }
    }

    fn steps(&self) -> PathBuf {
        self.root.join("steps")
    }

    /// `<files_dir>/steps/<hex>/<node>/`.
    pub fn step_dir(&self, artifact: &str, node: &str) -> Result<PathBuf, Unmade> {
        let hex = hex(artifact)
            .ok_or_else(|| Unmade::Unreadable(format!("{artifact:?} is not an artifact digest")))?;
        Ok(self.steps().join(hex).join(node))
    }

    /// `<files_dir>/work/<run>/<invocation>/`, created with mode 0700 if missing.
    pub fn work_dir(&self, run: &str, invocation: &str) -> io::Result<PathBuf> {
        let dir = self.root.join("work").join(run).join(invocation);
        DirBuilder::new()
            .recursive(true)
            .mode(WORK_MODE)
            .create(&dir)?;
        Ok(dir)
    }

    fn lock_file(&self, hex: &str) -> io::Result<File> {
        fs::create_dir_all(self.steps())?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.steps().join(format!("{hex}.lock")))
    }

    /// The artifact's lock, held shared by a dispatch from its check until its try
    /// ends; `None` while a sweep holds it exclusive. Never waits, so a sweep holding
    /// the lock across an await cannot block the thread it runs on.
    pub fn try_lock_shared(&self, artifact: &str) -> Result<Option<Flock<File>>, Unmade> {
        let hex = hex(artifact)
            .ok_or_else(|| Unmade::Unreadable(format!("{artifact:?} is not an artifact digest")))?;
        match Flock::lock(self.lock_file(hex)?, FlockArg::LockSharedNonblock) {
            Ok(lock) => Ok(Some(lock)),
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(None),
            Err((_, errno)) => Err(Unmade::Unwritable(format!("lock: {errno}"))),
        }
    }

    /// Removes the step files of every artifact for which `unpinned` says no unfinished
    /// run needs them. `unpinned` is called once, with the artifacts whose locks this
    /// call holds exclusive; an artifact whose lock is held elsewhere is skipped.
    pub async fn sweep<F, Fut>(&self, unpinned: F) -> io::Result<usize>
    where
        F: FnOnce(Vec<String>) -> Fut,
        Fut: std::future::Future<Output = Option<BTreeSet<String>>>,
    {
        let steps = self.steps();
        let listing = match fs::read_dir(&steps) {
            Ok(listing) => listing,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut locked = Vec::new();
        for item in listing {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with(TRASH) {
                // Left by a sweep or a repair that died before deleting it.
                let _ = remove_tree(&item.path());
                continue;
            }
            if !item.file_type()?.is_dir() || hex(&format!("sha256:{name}")).is_none() {
                continue;
            }
            let Ok(lock) = Flock::lock(self.lock_file(&name)?, FlockArg::LockExclusiveNonblock)
            else {
                continue;
            };
            locked.push((format!("sha256:{name}"), name, lock));
        }
        if locked.is_empty() {
            return Ok(0);
        }
        let Some(unpinned) = unpinned(locked.iter().map(|(d, _, _)| d.clone()).collect()).await
        else {
            return Ok(0);
        };
        let mut trash = Vec::new();
        for (digest, name, _) in &locked {
            if unpinned.contains(digest) {
                let aside = steps.join(format!("{TRASH}{name}-{}", unique()));
                fs::rename(steps.join(name), &aside)?;
                trash.push(aside);
            }
        }
        drop(locked);
        for aside in &trash {
            remove_tree(aside)?;
        }
        Ok(trash.len())
    }
}

/// Whether `dir` holds exactly `wanted`: regular files with their modes and digests, in
/// folders of mode 0555, and nothing else.
pub(crate) fn intact(dir: &Path, wanted: &BTreeMap<String, Want>) -> bool {
    let mut found = BTreeMap::new();
    if !walk(dir, "", &mut found) || found.len() != wanted.len() {
        return false;
    }
    wanted.iter().all(|(path, want)| {
        found
            .get(path)
            .is_some_and(|(mode, bytes): &(u32, Vec<u8>)| {
                *mode == want.mode() && digest::blob(bytes) == want.digest
            })
    })
}

/// Collects the files under `dir` by relative path; false on anything that is not a
/// regular file or a 0555 folder.
fn walk(dir: &Path, prefix: &str, found: &mut BTreeMap<String, (u32, Vec<u8>)>) -> bool {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return false;
    };
    if !meta.is_dir() || meta.permissions().mode() & 0o7777 != FOLDER_MODE {
        return false;
    }
    let Ok(listing) = fs::read_dir(dir) else {
        return false;
    };
    for item in listing {
        let Ok(item) = item else {
            return false;
        };
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            return false;
        };
        let path = item.path();
        let relative = format!("{prefix}{name}");
        let Ok(meta) = fs::symlink_metadata(&path) else {
            return false;
        };
        if meta.is_dir() {
            if !walk(&path, &format!("{relative}/"), found) {
                return false;
            }
        } else if meta.is_file() {
            let Ok(bytes) = fs::read(&path) else {
                return false;
            };
            found.insert(relative, (meta.permissions().mode() & 0o7777, bytes));
        } else {
            return false;
        }
    }
    true
}

/// Makes `dir` again from `wanted`, reading each file's bytes with `read`, unless the
/// folder already holds them. A broken folder is moved aside and removed; the new one
/// is built beside it and renamed into place.
pub(crate) fn make(
    dir: &Path,
    wanted: &BTreeMap<String, Want>,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, ArtifactError>,
) -> Result<(), Unmade> {
    let parent = dir.parent().expect("a step folder has a parent");
    let node = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    let node = node.unwrap_or_default();
    fs::create_dir_all(parent)?;
    let mut blobs = BTreeMap::new();
    for (path, want) in wanted {
        artifact::check_path(path)?;
        let bytes = read(&want.digest)?;
        if digest::blob(&bytes) != want.digest {
            return Err(Unmade::Unreadable(format!(
                "blob {} does not match its digest",
                want.digest
            )));
        }
        blobs.insert(path.clone(), bytes);
    }
    for _ in 0..3 {
        if intact(dir, wanted) {
            return Ok(());
        }
        if fs::symlink_metadata(dir).is_ok() {
            let aside = parent.join(format!("{TRASH}{node}-{}", unique()));
            match fs::rename(dir, &aside) {
                Ok(()) => remove_tree(&aside)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        let fresh = parent.join(format!(".make-{node}-{}", unique()));
        if let Err(e) = write_tree(&fresh, wanted, &blobs) {
            let _ = remove_tree(&fresh);
            return Err(e);
        }
        match fs::rename(&fresh, dir) {
            Ok(()) => return Ok(()),
            // Another dispatch put its folder in place first; check that one.
            Err(_) if fs::symlink_metadata(dir).is_ok() => remove_tree(&fresh)?,
            Err(e) => {
                let _ = remove_tree(&fresh);
                return Err(e.into());
            }
        }
    }
    if intact(dir, wanted) {
        Ok(())
    } else {
        Err(Unmade::Unwritable(format!(
            "{} keeps changing while it is made",
            dir.display()
        )))
    }
}

/// Writes the files into the new folder `root`, then makes every folder read-only.
fn write_tree(
    root: &Path,
    wanted: &BTreeMap<String, Want>,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> Result<(), Unmade> {
    DirBuilder::new().mode(0o700).create(root)?;
    let mut folders = vec![root.to_path_buf()];
    for (path, want) in wanted {
        let file = root.join(path);
        // `check_path` rules out `..` and absolute paths; nothing under a new folder
        // is a link, so the file stays inside `root`.
        if !file.starts_with(root) {
            return Err(Unmade::Unreadable(format!(
                "entry path {path:?} leaves the folder"
            )));
        }
        let mut folder = root.to_path_buf();
        for segment in path.split('/').take(path.split('/').count() - 1) {
            folder.push(segment);
            match fs::symlink_metadata(&folder) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    return Err(Unmade::Unreadable(format!(
                        "entry path {path:?} runs through a file"
                    )))
                }
                Err(_) => {
                    DirBuilder::new().mode(0o700).create(&folder)?;
                    folders.push(folder.clone());
                }
            }
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .map_err(|e| match e.kind() {
                io::ErrorKind::AlreadyExists | io::ErrorKind::IsADirectory => {
                    Unmade::Unreadable(format!("entry path {path:?} clashes with another"))
                }
                _ => e.into(),
            })?;
        out.write_all(&blobs[path])?;
        out.sync_all()?;
        fs::set_permissions(&file, fs::Permissions::from_mode(want.mode()))?;
    }
    // Deepest first, so each folder is still writable while its children change.
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for folder in folders {
        fs::set_permissions(&folder, fs::Permissions::from_mode(FOLDER_MODE))?;
    }
    Ok(())
}

/// Removes a tree whose folders may be read-only. Links are removed, never followed.
pub(crate) fn remove_tree(root: &Path) -> io::Result<()> {
    fn writable(dir: &Path) -> io::Result<()> {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        for item in fs::read_dir(dir)? {
            let item = item?;
            if item.file_type()?.is_dir() {
                writable(&item.path())?;
            }
        }
        Ok(())
    }
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
        Ok(meta) if !meta.is_dir() => return fs::remove_file(root),
        Ok(_) => writable(root)?,
    }
    match fs::remove_dir_all(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("runspore-files-{}-{name}", std::process::id()));
        let _ = remove_tree(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files(items: &[(&str, &[u8], bool)]) -> (BTreeMap<String, Want>, BTreeMap<String, Vec<u8>>) {
        let mut wanted = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        for (path, bytes, executable) in items {
            let digest = digest::blob(bytes);
            wanted.insert(
                path.to_string(),
                Want {
                    digest: digest.clone(),
                    executable: *executable,
                },
            );
            blobs.insert(digest, bytes.to_vec());
        }
        (wanted, blobs)
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn makes_read_only_files_and_repairs_tampering() {
        let root = temp("make");
        let dir = root.join("a").join("node");
        let (wanted, blobs) = files(&[
            ("i/n.md", b"read me", false),
            ("run.sh", b"#!/bin/sh\n", true),
        ]);
        let read = |d: &str| Ok(blobs[d].clone());
        make(&dir, &wanted, read).unwrap();
        assert!(intact(&dir, &wanted));
        assert_eq!(mode(&dir), 0o555);
        assert_eq!(mode(&dir.join("i")), 0o555);
        assert_eq!(mode(&dir.join("i/n.md")), 0o444);
        assert_eq!(mode(&dir.join("run.sh")), 0o555);

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("extra"), b"x").unwrap();
        assert!(!intact(&dir, &wanted));
        make(&dir, &wanted, read).unwrap();
        assert!(intact(&dir, &wanted) && !dir.join("extra").exists());

        fs::set_permissions(dir.join("i/n.md"), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!intact(&dir, &wanted));
        make(&dir, &wanted, read).unwrap();
        assert_eq!(fs::read(dir.join("i/n.md")).unwrap(), b"read me");
        remove_tree(&root).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn a_missing_or_wrong_blob_is_unreadable() {
        let root = temp("unreadable");
        let dir = root.join("a").join("node");
        let (wanted, _) = files(&[("n.md", b"read me", false)]);
        let missing = make(&dir, &wanted, |d| {
            Err(ArtifactError::invalid(format!("no blob {d}")))
        });
        assert!(matches!(missing, Err(Unmade::Unreadable(_))));
        let wrong = make(&dir, &wanted, |_| Ok(b"other".to_vec()));
        assert!(matches!(wrong, Err(Unmade::Unreadable(_))));
        assert!(!dir.exists());
        remove_tree(&root).unwrap();
    }

    #[test]
    fn a_bundled_program_is_an_asset_named_with_dot_slash() {
        let files = StepFiles {
            dir: PathBuf::from("/s"),
            instructions: None,
            assets: ["tools/run.sh".to_string()].into(),
        };
        assert_eq!(
            files.program("./tools/run.sh"),
            Some(PathBuf::from("/s/tools/run.sh"))
        );
        assert_eq!(files.program("tools/run.sh"), None);
        assert_eq!(files.program("./test.sh"), None);
    }
}
