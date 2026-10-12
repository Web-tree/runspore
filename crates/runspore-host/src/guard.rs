//! The guard: the process the host puts in front of every process it starts for an
//! attempt (spec/host.md §3).
//!
//! The guard is a hidden mode of the host's own executable. The host starts it with
//! two pipes: a control pipe the host writes to, and a report pipe it reads from. The
//! guard moves into a session and group of its own, then starts a holding process in a
//! new group, which waits for the go before it runs the program. The guard reports the
//! holding process's ID, lets the program run once the host says go, and reports how
//! the program ended. When the host's end of the control pipe closes without a stop,
//! the host is gone and the guard stops the program's group.
//!
//! Lines on the report pipe: `pid <n>`, `running`, `error <message>`, `exit <code>`,
//! `signal <n>`. Lines on the control pipe: `go`, `stop`.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::sync::Mutex;
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::pipe;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::signal::unix::{signal, SignalKind};

/// First argument that starts the guard mode.
pub const GUARD_MODE: &str = "__guard";
/// First argument that starts the holding process the guard puts the program in.
const HOLD_MODE: &str = "__guard-hold";

/// How often a stopping guard looks whether the group is gone.
const POLL: Duration = Duration::from_millis(20);

/// Runs the guard's hidden modes when this process was started as one of them, and
/// returns its exit code; otherwise returns `None`. A binary that embeds the engine
/// calls this first thing in `main`, before it starts any thread.
pub fn main_if_requested() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next()?;
    let rest: Vec<OsString> = args.collect();
    if mode == GUARD_MODE {
        Some(guard_main(&rest))
    } else if mode == HOLD_MODE {
        Some(hold_main(&rest))
    } else {
        None
    }
}

/// How a guarded program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It exited with this code.
    Exited(i32),
    /// A signal ended it.
    Signaled(i32),
    /// The guard ended without reporting the program's ending.
    Lost,
}

/// Serializes the windows in which the guard's ends of its pipes are inheritable, so a
/// guard started from another thread never inherits them.
static SPAWN: Mutex<()> = Mutex::new(());

/// The host's handle on one guarded program. Dropping it closes the control pipe: the
/// guard then stops the program's group as it does when the host dies.
pub struct Guarded {
    guard: Child,
    control: Option<pipe::Sender>,
    reports: Lines<BufReader<pipe::Receiver>>,
    pid: i32,
}

impl Guarded {
    /// Starts `argv` under the guard `guard` (an executable with the guard mode).
    /// `setup` sets the working directory, environment and standard streams, which the
    /// program inherits through the guard. Returns once the program's process exists
    /// and waits for [`Guarded::go`]; the program has not run yet.
    pub async fn spawn(
        guard: &Path,
        stop_grace: Duration,
        argv: &[String],
        setup: impl FnOnce(&mut Command),
    ) -> Result<Self, String> {
        let (guard, control_write, report_read) = {
            let _window = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
            let (control_read, control_write) = nix::unistd::pipe().map_err(text)?;
            let (report_read, report_write) = nix::unistd::pipe().map_err(text)?;
            cloexec(&control_write)?;
            cloexec(&report_read)?;
            let mut command = Command::new(guard);
            command
                .arg(GUARD_MODE)
                .arg(control_read.as_raw_fd().to_string())
                .arg(report_write.as_raw_fd().to_string())
                .arg(stop_grace.as_millis().to_string())
                .args(argv);
            setup(&mut command);
            (command.spawn().map_err(text)?, control_write, report_read)
        };
        let control = pipe::Sender::from_owned_fd(control_write).map_err(text)?;
        let reports = pipe::Receiver::from_owned_fd(report_read).map_err(text)?;
        let mut guarded = Self {
            guard,
            control: Some(control),
            reports: BufReader::new(reports).lines(),
            pid: 0,
        };
        let line = guarded.report().await;
        guarded.pid = match line.as_deref().and_then(|l| l.strip_prefix("pid ")) {
            Some(pid) => pid.parse().map_err(text)?,
            None => return Err(refusal(line)),
        };
        Ok(guarded)
    }

    /// The program's process ID, which is also its process group ID.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// The guard's standard streams, which the program inherits.
    pub fn stdio(&mut self) -> (Option<ChildStdin>, Option<ChildStdout>, Option<ChildStderr>) {
        (
            self.guard.stdin.take(),
            self.guard.stdout.take(),
            self.guard.stderr.take(),
        )
    }

    /// Lets the program run. Fails when it could not be started.
    pub async fn go(&mut self) -> Result<(), String> {
        self.send("go").await;
        match self.report().await {
            Some(line) if line == "running" => Ok(()),
            line => Err(refusal(line)),
        }
    }

    /// Waits until the program has ended and its group has been stopped. Cancel-safe.
    pub async fn wait(&mut self) -> Ending {
        let ending = match self.report().await {
            Some(line) => parse_ending(&line),
            None => Ending::Lost,
        };
        let _ = self.guard.wait().await;
        ending
    }

    /// Stops the program's group (SIGTERM, the grace, SIGKILL) and waits until it is
    /// done.
    pub async fn stop(&mut self) -> Ending {
        self.send("stop").await;
        self.wait().await
    }

    async fn send(&mut self, line: &str) {
        if let Some(control) = &mut self.control {
            let _ = control.write_all(format!("{line}\n").as_bytes()).await;
        }
    }

    async fn report(&mut self) -> Option<String> {
        self.reports.next_line().await.ok().flatten()
    }
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What a report that is not the expected one says about the failure.
fn refusal(line: Option<String>) -> String {
    match line {
        Some(line) => line.strip_prefix("error ").unwrap_or(&line).to_string(),
        None => "the guard ended without starting the command".to_string(),
    }
}

fn parse_ending(line: &str) -> Ending {
    let number = |prefix| line.strip_prefix(prefix).and_then(|n: &str| n.parse().ok());
    match (number("exit "), number("signal ")) {
        (Some(code), _) => Ending::Exited(code),
        (None, Some(signal)) => Ending::Signaled(signal),
        (None, None) => Ending::Lost,
    }
}

/// Keeps `fd` from being inherited. The other end of each pipe stays inheritable for
/// the one child it is made for.
fn cloexec(fd: &OwnedFd) -> Result<(), String> {
    fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .map(drop)
        .map_err(text)
}

/// Takes over an inherited pipe end by its number: opens it afresh through `/dev/fd`
/// (close-on-exec) and closes the inherited number, so no child inherits it.
fn adopt(number: &OsString, write: bool) -> Option<File> {
    let number: i32 = number.to_str()?.parse().ok()?;
    let file = OpenOptions::new()
        .read(!write)
        .write(write)
        .open(format!("/dev/fd/{number}"))
        .ok()?;
    let _ = nix::unistd::close(number);
    Some(file)
}

fn guard_main(args: &[OsString]) -> ExitCode {
    let [control, report, grace, program, rest @ ..] = args else {
        return ExitCode::from(2);
    };
    let (Some(control), Some(mut report)) = (adopt(control, false), adopt(report, true)) else {
        return ExitCode::from(2);
    };
    let grace = Duration::from_millis(grace.to_str().and_then(|g| g.parse().ok()).unwrap_or(0));
    // Leave the host's session and group: a signal to either never reaches the guard.
    let _ = nix::unistd::setsid();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = writeln!(report, "error {e}");
            return ExitCode::from(1);
        }
    };
    runtime.block_on(guard(control, report, grace, program, rest))
}

async fn guard(
    control: File,
    mut report: File,
    grace: Duration,
    program: &OsString,
    rest: &[OsString],
) -> ExitCode {
    let mut say = move |line: String| {
        let _ = writeln!(report, "{line}");
    };
    let mut control = match pipe::Receiver::from_file(control) {
        Ok(control) => BufReader::new(control).lines(),
        Err(e) => {
            say(format!("error {e}"));
            return ExitCode::from(1);
        }
    };
    let held = match Hold::spawn(program, rest) {
        Ok(held) => held,
        Err(e) => {
            say(format!("error {e}"));
            return ExitCode::from(1);
        }
    };
    let Hold {
        mut child,
        go,
        status,
    } = held;
    let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) else {
        say("error the command's process has no ID".into());
        return ExitCode::from(1);
    };
    let group = Pid::from_raw(pid);
    say(format!("pid {pid}"));

    // The program runs only once the host says go. Anything else means the host is
    // gone or gave up: the waiting process is killed before it runs anything.
    if control.next_line().await.ok().flatten().as_deref() != Some("go") {
        let _ = killpg(group, Signal::SIGKILL);
        let _ = child.wait().await;
        return ExitCode::SUCCESS;
    }
    if let Err(e) = release(go, status) {
        let _ = child.wait().await;
        say(format!("error {e}"));
        return ExitCode::SUCCESS;
    }
    say("running".into());

    let status = tokio::select! {
        status = child.wait() => {
            // A normal ending: whatever the program left in its group goes at once.
            let _ = killpg(group, Signal::SIGKILL);
            status
        }
        // A stop from the host, or its end of the pipe closed: the host is gone.
        _ = control.next_line() => stop_group(group, grace, &mut child).await,
    };
    say(match status {
        Ok(status) => ending_line(status),
        Err(e) => format!("error {e}"),
    });
    ExitCode::SUCCESS
}

fn ending_line(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "error unknown ending".into(),
    }
}

/// SIGTERM to the group, up to `grace` for it to be gone, then SIGKILL. Returns the
/// leader's exit status.
async fn stop_group(
    group: Pid,
    grace: Duration,
    leader: &mut Child,
) -> std::io::Result<ExitStatus> {
    let _ = killpg(group, Signal::SIGTERM);
    let deadline = tokio::time::Instant::now() + grace;
    let mut status = None;
    loop {
        if status.is_none() {
            tokio::select! {
                s = leader.wait() => status = Some(s),
                () = tokio::time::sleep(POLL) => {}
            }
        } else if killpg(group, None) == Err(Errno::ESRCH) {
            break;
        } else {
            tokio::time::sleep(POLL).await;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = killpg(group, Signal::SIGKILL);
            break;
        }
    }
    match status {
        Some(status) => status,
        None => leader.wait().await,
    }
}

/// The holding process and the guard's ends of its two pipes: `go` releases it,
/// `status` carries an error if the program could not be run and closes when it runs.
struct Hold {
    child: Child,
    go: OwnedFd,
    status: OwnedFd,
}

impl Hold {
    fn spawn(program: &OsString, rest: &[OsString]) -> Result<Self, String> {
        let (go_read, go) = nix::unistd::pipe().map_err(text)?;
        let (status, status_write) = nix::unistd::pipe().map_err(text)?;
        cloexec(&go)?;
        cloexec(&status)?;
        let exe = std::env::current_exe().map_err(text)?;
        let child = Command::new(exe)
            .arg(HOLD_MODE)
            .arg(go_read.as_raw_fd().to_string())
            .arg(status_write.as_raw_fd().to_string())
            .arg(program)
            .args(rest)
            .process_group(0)
            .spawn()
            .map_err(text)?;
        Ok(Self { child, go, status })
    }
}

/// Lets the holding process run its program, and returns the error it reports if the
/// program could not be run.
fn release(go: OwnedFd, status: OwnedFd) -> Result<(), String> {
    let mut go = File::from(go);
    let _ = go.write_all(b"g");
    drop(go);
    let mut error = String::new();
    let _ = File::from(status).read_to_string(&mut error);
    if error.is_empty() {
        Ok(())
    } else {
        Err(error)
    }
}

fn hold_main(args: &[OsString]) -> ExitCode {
    let [go, status, program, rest @ ..] = args else {
        return ExitCode::from(2);
    };
    let (Some(mut go), Some(mut status)) = (adopt(go, false), adopt(status, true)) else {
        return ExitCode::from(2);
    };
    let mut byte = [0u8; 1];
    if !matches!(go.read(&mut byte), Ok(1)) {
        return ExitCode::from(1);
    }
    drop(go);
    reset_signals();
    // `exec` empties the signal mask and returns only on failure.
    let error = std::process::Command::new(program).args(rest).exec();
    let _ = write!(status, "{error}");
    ExitCode::from(127)
}

/// Gives the program default dispositions for the signals a parent commonly ignores.
/// A handled signal reverts to its default action at `exec`; an ignored one would stay
/// ignored, so each gets a handler first.
fn reset_signals() {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let _context = runtime.enter();
    for kind in [
        SignalKind::hangup(),
        SignalKind::interrupt(),
        SignalKind::quit(),
        SignalKind::terminate(),
    ] {
        let _ = signal(kind);
    }
}

/// This process's own executable: the default guard of an engine.
pub fn own_executable() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}
