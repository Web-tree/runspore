//! Process records (spec/host.md §3): what an attempt keeps about the process group
//! doing its work, and the check before a lease expiry that stops a leftover group on
//! this machine.

use std::sync::OnceLock;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use runspore_types::store::ProcessRecord;

/// How often a stopping host looks whether the group is gone.
const POLL: Duration = Duration::from_millis(20);
/// How long the host waits for a group to be gone after SIGKILL.
const AFTER_KILL: Duration = Duration::from_secs(1);

/// What the check before a lease expiry found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leftover {
    /// No process of the recorded group runs on this machine any more: expire.
    Gone,
    /// The record cannot be checked from here (another host, EPERM): expire, noting
    /// `process.unchecked`.
    Unchecked,
    /// The group is still there after the stop: leave the attempt for the next pass.
    Remains,
}

struct Machine {
    host: String,
    boot: String,
}

/// This machine's host name and boot identifier, read once.
fn machine() -> Result<&'static Machine, String> {
    static MACHINE: OnceLock<Result<Machine, String>> = OnceLock::new();
    MACHINE
        .get_or_init(|| {
            let host = nix::unistd::gethostname()
                .map_err(|e| format!("host name: {e}"))?
                .to_string_lossy()
                .into_owned();
            Ok(Machine {
                host,
                boot: os::boot_id()?,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The process record of the group `pid` leads, read while it is waiting for the go.
pub fn record(pid: i32) -> Result<ProcessRecord, String> {
    let machine = machine()?;
    let started = os::start_time(pid).ok_or_else(|| format!("start time of process {pid}"))?;
    Ok(ProcessRecord {
        host: machine.host.clone(),
        boot: machine.boot.clone(),
        pgid: pid,
        started: started.to_string(),
    })
}

/// Settles `record` before its attempt's lease is expired: stops the group it names
/// when that group is ours and still on this machine (SIGTERM, `grace`, SIGKILL), and
/// waits until no process of it remains, for at most `grace` plus 1 s.
pub async fn settle(record: &ProcessRecord, grace: Duration) -> Leftover {
    let Ok(machine) = machine() else {
        return Leftover::Unchecked;
    };
    if record.host != machine.host {
        return Leftover::Unchecked;
    }
    if record.boot != machine.boot {
        return Leftover::Gone;
    }
    let Ok(started) = record.started.parse::<u64>() else {
        return Leftover::Unchecked;
    };
    match ours(record.pgid, started) {
        Some(true) => {}
        Some(false) => return Leftover::Gone,
        None => return Leftover::Unchecked,
    }
    let group = Pid::from_raw(record.pgid);
    match killpg(group, Signal::SIGTERM) {
        Err(Errno::ESRCH) => return Leftover::Gone,
        Err(Errno::EPERM) => return Leftover::Unchecked,
        _ => {}
    }
    if gone_within(group, grace).await {
        return Leftover::Gone;
    }
    let _ = killpg(group, Signal::SIGKILL);
    if gone_within(group, AFTER_KILL).await {
        Leftover::Gone
    } else {
        Leftover::Remains
    }
}

/// What a look at a process record from this machine finds, stopping nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// The recorded group runs here.
    Running,
    /// No process of the recorded group runs here any more.
    Gone,
    /// The record is of this machine before it last booted.
    Rebooted,
    /// The record cannot be checked from here: another host, or members that cannot be
    /// read.
    Unchecked,
}

/// Looks whether the group `record` names still runs, as the check before a lease
/// expiry does, its leader checked with its start time and not by the number alone.
pub fn look(record: &ProcessRecord) -> Seen {
    let Ok(machine) = machine() else {
        return Seen::Unchecked;
    };
    if record.host != machine.host {
        return Seen::Unchecked;
    }
    if record.boot != machine.boot {
        return Seen::Rebooted;
    }
    let Ok(started) = record.started.parse::<u64>() else {
        return Seen::Unchecked;
    };
    match ours(record.pgid, started) {
        Some(true) => Seen::Running,
        Some(false) => Seen::Gone,
        None => Seen::Unchecked,
    }
}

/// Whether process `pid`, which started the group `record` names, is known to be gone:
/// the record is of this machine, and the machine has booted since or no process `pid`
/// runs. False when that cannot be told from here.
pub fn starter_gone(record: &ProcessRecord, pid: i32) -> bool {
    starter_runs(record, pid) == Some(false)
}

/// Whether process `pid`, which started the group `record` names, still runs; `None`
/// when that cannot be told from here (another host).
pub fn starter_runs(record: &ProcessRecord, pid: i32) -> Option<bool> {
    let machine = machine().ok()?;
    if record.host != machine.host {
        return None;
    }
    Some(
        record.boot == machine.boot
            && nix::sys::signal::kill(Pid::from_raw(pid), None) != Err(Errno::ESRCH),
    )
}

/// Whether the group `pgid` running now is the one recorded with start time
/// `started`: its leader is alive with that start time, or it has lost its leader and
/// every member started at or after it. `Some(false)` when no process of the group
/// remains; `None` when the members cannot be read.
fn ours(pgid: i32, started: u64) -> Option<bool> {
    let members = os::group_members(pgid)?;
    if members.is_empty() {
        return Some(false);
    }
    Some(match members.iter().find(|(pid, _)| *pid == pgid) {
        Some((_, leader)) => *leader == started,
        None => members.iter().all(|(_, start)| *start >= started),
    })
}

/// Whether no live process of the group remains. A zombie does not count: its parent,
/// or init, reaps it.
fn gone(group: Pid) -> bool {
    killpg(group, None) == Err(Errno::ESRCH)
        || os::group_members(group.as_raw()).is_some_and(|members| members.is_empty())
}

/// Waits up to `limit` for the group to have no live process left.
async fn gone_within(group: Pid, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if gone(group) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(target_os = "linux")]
mod os {
    /// The kernel's random boot ID.
    pub fn boot_id() -> Result<String, String> {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map(|id| id.trim().to_string())
            .map_err(|e| format!("boot ID: {e}"))
    }

    /// `(process group, start time in clock ticks since boot, zombie)` from
    /// `/proc/<pid>/stat`.
    fn stat(pid: &str) -> Option<(i32, u64, bool)> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name may hold spaces and parentheses; the fields follow the last `)`.
        let fields: Vec<&str> = stat
            .get(stat.rfind(')')? + 1..)?
            .split_whitespace()
            .collect();
        let zombie = *fields.first()? == "Z";
        Some((
            fields.get(2)?.parse().ok()?,
            fields.get(19)?.parse().ok()?,
            zombie,
        ))
    }

    pub fn start_time(pid: i32) -> Option<u64> {
        stat(&pid.to_string()).map(|(_, start, _)| start)
    }

    /// The group's processes that are not zombies.
    pub fn group_members(pgid: i32) -> Option<Vec<(i32, u64)>> {
        let entries = std::fs::read_dir("/proc").ok()?;
        Some(
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let pid: i32 = name.parse().ok()?;
                    let (group, start, zombie) = stat(&name)?;
                    (group == pgid && !zombie).then_some((pid, start))
                })
                .collect(),
        )
    }
}

#[cfg(target_os = "macos")]
mod os {
    use libproc::bsd_info::BSDInfo;
    use libproc::proc_pid::pidinfo;
    use libproc::processes::{pids_by_type, ProcFilter};
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    use sysctl::Sysctl;

    /// When the machine booted, `kern.boottime`, as `<seconds>.<microseconds>`.
    pub fn boot_id() -> Result<String, String> {
        let boot = sysctl::Ctl::new("kern.boottime")
            .and_then(|ctl| ctl.value_as::<nix::libc::timeval>())
            .map_err(|e| format!("boot time: {e}"))?;
        Ok(format!("{}.{:06}", boot.tv_sec, boot.tv_usec))
    }

    /// `pbi_status` of a zombie (`SZOMB`).
    const ZOMBIE: u32 = 5;

    /// `(process group, start time in microseconds since the epoch, zombie)`.
    fn info(pid: i32) -> Option<(i32, u64, bool)> {
        let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
        let start = info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec;
        let zombie = info.pbi_status == ZOMBIE;
        Some((i32::try_from(info.pbi_pgid).ok()?, start, zombie))
    }

    pub fn start_time(pid: i32) -> Option<u64> {
        info(pid).map(|(_, start, _)| start)
    }

    /// The group's processes that are not zombies; `None` when a member that still
    /// exists cannot be read.
    pub fn group_members(pgid: i32) -> Option<Vec<(i32, u64)>> {
        let pgrpid = u32::try_from(pgid).ok()?;
        let mut members = Vec::new();
        for pid in pids_by_type(ProcFilter::ByProgramGroup { pgrpid }).ok()? {
            let pid = i32::try_from(pid).ok()?;
            match info(pid) {
                Some((group, start, false)) if group == pgid => members.push((pid, start)),
                Some(_) => {}
                None if kill(Pid::from_raw(pid), None) == Err(Errno::ESRCH) => {}
                None => return None,
            }
        }
        Some(members)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod os {
    pub fn boot_id() -> Result<String, String> {
        Err("process records need Linux or macOS".into())
    }

    pub fn start_time(_pid: i32) -> Option<u64> {
        None
    }

    pub fn group_members(_pgid: i32) -> Option<Vec<(i32, u64)>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;

    use super::*;

    fn here(pgid: i32, started: u64) -> ProcessRecord {
        let machine = machine().unwrap();
        ProcessRecord {
            host: machine.host.clone(),
            boot: machine.boot.clone(),
            pgid,
            started: started.to_string(),
        }
    }

    #[test]
    fn a_record_names_this_machine_and_the_process_start() {
        let pid = std::process::id() as i32;
        let record = record(pid).unwrap();
        assert_eq!(record.pgid, pid);
        assert_eq!(record.started, os::start_time(pid).unwrap().to_string());
        assert!(!record.host.is_empty() && !record.boot.is_empty());
        assert_eq!(record.boot, machine().unwrap().boot);
    }

    #[tokio::test]
    async fn another_host_is_unchecked_and_another_boot_is_gone() {
        let mut elsewhere = here(1, 0);
        elsewhere.host = format!("{}-elsewhere", elsewhere.host);
        assert_eq!(
            settle(&elsewhere, Duration::ZERO).await,
            Leftover::Unchecked
        );
        let mut rebooted = here(1, 0);
        rebooted.boot = "another-boot".into();
        assert_eq!(settle(&rebooted, Duration::ZERO).await, Leftover::Gone);
    }

    #[tokio::test]
    async fn only_the_recorded_group_is_stopped() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let started = os::start_time(pid).unwrap();
        // Reaped as soon as it ends, as a leftover's parent, init, would.
        let reaper = std::thread::spawn(move || child.wait());
        // Recorded later than the leader started: the leader is someone else's.
        let stranger = here(pid, started + 1);
        assert_eq!(settle(&stranger, Duration::ZERO).await, Leftover::Gone);
        assert_eq!(
            killpg(Pid::from_raw(pid), None),
            Ok(()),
            "a stranger was stopped"
        );
        let ours = here(pid, started);
        assert_eq!(settle(&ours, Duration::from_secs(5)).await, Leftover::Gone);
        assert_eq!(killpg(Pid::from_raw(pid), None), Err(Errno::ESRCH));
        let _ = reaper.join();
    }

    #[test]
    fn a_look_tells_a_running_group_from_a_gone_one() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let record = here(pid, os::start_time(pid).unwrap());
        assert_eq!(look(&record), Seen::Running);
        assert_eq!(
            look(&here(pid, os::start_time(pid).unwrap() + 1)),
            Seen::Gone,
            "a leader that started later is someone else's"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(look(&record), Seen::Gone);
        let mut rebooted = record.clone();
        rebooted.boot = "another-boot".into();
        assert_eq!(look(&rebooted), Seen::Rebooted);
        let mut elsewhere = record;
        elsewhere.host = format!("{}-elsewhere", elsewhere.host);
        assert_eq!(look(&elsewhere), Seen::Unchecked);
    }

    /// Linux only: a container whose first process does not reap leaves zombies for
    /// good. On macOS `launchd` reaps a dead leftover at once.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_zombie_does_not_keep_a_group_alive() {
        let mut child = std::process::Command::new("true")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        // Not reaped yet: a zombie, which `kill` still finds.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(killpg(Pid::from_raw(pid), None), Ok(()));
        assert!(gone(Pid::from_raw(pid)));
        assert_eq!(ours(pid, 0), Some(false));
        let _ = child.wait();
    }
}
