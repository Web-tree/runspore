//! Disk pressure (spec/host.md §2, Disk pressure): the free space of the disk a worker's
//! steps write to, and when it is under the floor. Like a kubelet that marks its node
//! unhealthy on disk pressure, a worker on a full disk takes no new step.

use std::fmt;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The floor `EngineConfig::disk_floor` has by default: 1 GiB.
pub const DEFAULT_FLOOR: u64 = 1 << 30;

/// The free space of one disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    /// Bytes a process that is not root may still write.
    pub free: u64,
    /// The folder the disk is mounted on.
    pub mount: PathBuf,
}

/// A disk whose free space is under the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFull {
    pub free: u64,
    pub mount: PathBuf,
    pub floor: u64,
}

impl fmt::Display for DiskFull {
    /// `disk full (312 MiB free on /)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "disk full ({} free on {})",
            bytes_text(self.free),
            self.mount.display()
        )
    }
}

type Measure = dyn Fn(&Path) -> io::Result<Space> + Send + Sync;

/// How the engine reads a disk's free space: the operating system's, or a test's.
#[derive(Clone)]
pub struct Disk(Arc<Measure>);

impl Disk {
    /// `statvfs` of the nearest folder at or above the path that exists.
    pub fn system() -> Self {
        Self(Arc::new(space))
    }

    /// Answers with `measure`, for tests.
    pub fn with(measure: impl Fn(&Path) -> io::Result<Space> + Send + Sync + 'static) -> Self {
        Self(Arc::new(measure))
    }

    /// The disk of `path` when its free space is under `floor`; `None` when it is not,
    /// when `floor` is 0, or when the space cannot be read, which is no reason to stop.
    pub fn full(&self, path: &Path, floor: u64) -> Option<DiskFull> {
        if floor == 0 {
            return None;
        }
        let space = (self.0)(path).ok()?;
        (space.free < floor).then_some(DiskFull {
            free: space.free,
            mount: space.mount,
            floor,
        })
    }
}

impl fmt::Debug for Disk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Disk")
    }
}

/// The free space of the disk holding `path`, or its nearest existing ancestor.
pub fn space(path: &Path) -> io::Result<Space> {
    let existing = path
        .ancestors()
        .find(|dir| dir.exists())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing folder"))?;
    let existing = existing.canonicalize()?;
    let stat = nix::sys::statvfs::statvfs(&existing).map_err(io::Error::from)?;
    #[allow(clippy::unnecessary_cast)]
    let free = (stat.blocks_available() as u64).saturating_mul(stat.fragment_size() as u64);
    Ok(Space {
        free,
        mount: mount_of(&existing)?,
    })
}

/// The topmost ancestor of `path` on the same device: the folder its disk is mounted on.
fn mount_of(path: &Path) -> io::Result<PathBuf> {
    let device = std::fs::metadata(path)?.dev();
    let mut mount = path;
    for dir in path.ancestors().skip(1) {
        match std::fs::metadata(dir) {
            Ok(meta) if meta.dev() == device => mount = dir,
            _ => break,
        }
    }
    Ok(mount.to_path_buf())
}

/// Whether an error is the operating system's "no space left on device".
pub(crate) fn is_full(e: &io::Error) -> bool {
    e.raw_os_error() == Some(nix::errno::Errno::ENOSPC as i32)
}

/// A byte count for a person: `512 B`, `8.0 KiB`, `312 MiB`, `1.5 GiB`.
pub fn bytes_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    match value < 10.0 {
        true => format!("{value:.1} {}", UNITS[unit]),
        false => format!("{value:.0} {}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counts_read_in_binary_units() {
        assert_eq!(bytes_text(0), "0 B");
        assert_eq!(bytes_text(1023), "1023 B");
        assert_eq!(bytes_text(8 * 1024), "8.0 KiB");
        assert_eq!(bytes_text(312 << 20), "312 MiB");
        assert_eq!(bytes_text(3 << 29), "1.5 GiB");
    }

    #[test]
    fn a_disk_is_full_under_its_floor_only() {
        let disk = Disk::with(|_| {
            Ok(Space {
                free: 100,
                mount: "/data".into(),
            })
        });
        let full = disk.full(Path::new("/data/x"), 101).unwrap();
        assert_eq!(full.to_string(), "disk full (100 B free on /data)");
        assert_eq!(disk.full(Path::new("/data/x"), 100), None);
        assert_eq!(disk.full(Path::new("/data/x"), 0), None);
        let unreadable = Disk::with(|_| Err(io::Error::other("no")));
        assert_eq!(unreadable.full(Path::new("/"), u64::MAX), None);
    }

    #[test]
    fn the_system_disk_of_a_missing_folder_is_its_nearest_ancestors() {
        let dir = std::env::temp_dir().join("runspore-disk-missing/a/b");
        let space = space(&dir).unwrap();
        assert!(space.free > 0);
        let canonical = std::env::temp_dir().canonicalize().unwrap();
        assert!(canonical.starts_with(&space.mount), "{space:?}");
    }

    #[test]
    fn no_space_left_is_recognised() {
        assert!(is_full(&io::Error::from_raw_os_error(
            nix::errno::Errno::ENOSPC as i32
        )));
        assert!(!is_full(&io::Error::other("other")));
    }
}
