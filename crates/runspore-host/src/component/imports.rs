//! The host interface a step runner imports (`contracts/wit/runner/runner.wit`,
//! `host`), each call checked against the action's grants. A call outside them, past
//! the call's deadline, or after the attempt was told to stop traps the runner; the
//! store data keeps why, so the call ends with it whatever the runner does next.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal as Unix};
use nix::unistd::Pid;
use runspore_types::failpoint;
use runspore_types::store::ProcessRecord;
use wasmtime::ResourceLimiter;

use super::grants::{Denied, Grants, Places};
use super::runspore::runner::host::{Host, Liveness, Process, Signal, Spawn};
use crate::activity::ActivityContext;
use crate::command;
use crate::process::{self, Seen};

/// What `request` reads at most.
const MAX_REPLY: usize = 1024 * 1024;
/// What `read-file` reads at most.
const MAX_FILE: u64 = 16 * 1024 * 1024;
/// How often a `sleep` looks whether the attempt was told to stop.
const SLEEP_SLICE: Duration = Duration::from_millis(50);

/// Why a call trapped, when the host made it trap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Halt {
    Denied(Denied),
    /// The call's deadline passed: `component.timeout`.
    Timeout,
    /// The lease was lost or the engine shuts down: `component.stopped`.
    Stopped,
}

/// The store data of one call.
pub(crate) struct State {
    pub ctx: ActivityContext,
    pub grants: Grants,
    pub places: Places,
    pub deadline: Instant,
    /// Whether this is `start`: its programs get the attempt's token.
    pub start: bool,
    pub halt: Option<Halt>,
    pub max_memory_bytes: usize,
    /// Whether memory growth was ever refused.
    pub refused: bool,
}

impl State {
    fn trap(&mut self, halt: Halt) -> wasmtime::Error {
        let message = match &halt {
            Halt::Denied(denied) => denied.message(&self.ctx.action_id),
            Halt::Timeout => "the call ran past its deadline".to_string(),
            Halt::Stopped => "the attempt was told to stop".to_string(),
        };
        self.halt.get_or_insert(halt);
        wasmtime::Error::msg(message)
    }

    /// Traps when the attempt was told to stop or the deadline has passed.
    fn live(&mut self) -> wasmtime::Result<()> {
        if self.ctx.is_stopped() {
            return Err(self.trap(Halt::Stopped));
        }
        if Instant::now() >= self.deadline {
            return Err(self.trap(Halt::Timeout));
        }
        Ok(())
    }

    fn granted<T>(&mut self, check: Result<T, Denied>) -> wasmtime::Result<T> {
        check.map_err(|denied| self.trap(Halt::Denied(denied)))
    }

    /// The time left before the deadline.
    fn left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// The program `argv0` names: the step files' copy of a `./<p>` asset, else as it
    /// is, a bare name looked up on the `PATH` when it starts.
    fn executable(&self, argv0: &str) -> Result<PathBuf, String> {
        if argv0.starts_with("./") {
            return self
                .ctx
                .files
                .as_ref()
                .and_then(|f| f.program(argv0))
                .ok_or_else(|| format!("`{argv0}` is not a file bundled with the action"));
        }
        Ok(PathBuf::from(argv0))
    }
}

fn record_of(p: &Process) -> ProcessRecord {
    ProcessRecord {
        host: p.host.clone(),
        boot: p.boot.clone(),
        pgid: p.pgid,
        started: p.started.clone(),
    }
}

impl Host for State {
    fn start_process(&mut self, spawn: Spawn) -> wasmtime::Result<Result<Process, String>> {
        self.live()?;
        let Some(argv0) = spawn.argv.first().cloned() else {
            return Ok(Err("`argv` is empty".to_string()));
        };
        let program = self.grants.program(&argv0);
        self.granted(program)?;
        let executable = match self.executable(&argv0) {
            Ok(executable) => executable,
            Err(e) => return Ok(Err(e)),
        };
        let cwd = spawn
            .cwd
            .as_deref()
            .map_or_else(|| self.places.base.clone(), |c| self.places.resolve(c));
        let mut child = std::process::Command::new(executable);
        child
            .args(&spawn.argv[1..])
            .current_dir(cwd)
            .envs(spawn.env.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // A group of its own: no signal to the worker's group reaches it.
            .process_group(0);
        for (name, value) in command::variables(&self.ctx) {
            match value {
                Some(value) => child.env(name, value),
                None => child.env_remove(name),
            };
        }
        match (&self.ctx.token, self.start) {
            (Some(token), true) => {
                child
                    .env("RUNSPORE_TOKEN", token)
                    .env("RUNSPORE_RESULT_CHANNEL", &self.ctx.result_channel);
            }
            _ => {
                child
                    .env_remove("RUNSPORE_TOKEN")
                    .env_remove("RUNSPORE_RESULT_CHANNEL");
            }
        }
        let mut child = match child.spawn() {
            Ok(child) => child,
            Err(e) => return Ok(Err(format!("starting `{argv0}`: {e}"))),
        };
        let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
        let record = process::record(pid);
        failpoint::hit("host.component.after-spawn");
        // The worker is its parent while it lives: reap it, so it never lingers as a
        // zombie that still looks alive.
        std::thread::spawn(move || child.wait());
        Ok(record
            .map(|r| Process {
                program: argv0,
                host: r.host,
                boot: r.boot,
                pgid: r.pgid,
                started: r.started,
            })
            .map_err(|e| format!("reading the started process: {e}")))
    }

    fn process_alive(&mut self, p: Process) -> wasmtime::Result<Liveness> {
        self.live()?;
        let program = self.grants.program(&p.program);
        self.granted(program)?;
        Ok(match process::look(&record_of(&p)) {
            Seen::Running => Liveness::Alive,
            Seen::Gone | Seen::Rebooted => Liveness::Dead,
            Seen::Unchecked => Liveness::Unknown,
        })
    }

    fn signal_process(
        &mut self,
        p: Process,
        signal: Signal,
    ) -> wasmtime::Result<Result<bool, String>> {
        self.live()?;
        let program = self.grants.program(&p.program);
        self.granted(program)?;
        if process::look(&record_of(&p)) != Seen::Running {
            return Ok(Ok(false));
        }
        let unix = match signal {
            Signal::Term => Unix::SIGTERM,
            Signal::Kill => Unix::SIGKILL,
            Signal::Interrupt => Unix::SIGINT,
            Signal::Hangup => Unix::SIGHUP,
        };
        Ok(match killpg(Pid::from_raw(p.pgid), unix) {
            Ok(()) => Ok(true),
            Err(Errno::ESRCH) => Ok(false),
            Err(e) => Err(format!("signalling process group {}: {e}", p.pgid)),
        })
    }

    fn request(
        &mut self,
        socket: String,
        body: Vec<u8>,
        timeout_ms: u32,
    ) -> wasmtime::Result<Result<Vec<u8>, String>> {
        self.live()?;
        let granted = self.grants.socket(&socket, &self.places);
        let path = self.granted(granted)?;
        let asked = Duration::from_millis(u64::from(timeout_ms).max(1));
        let limit = asked.min(self.left()).max(Duration::from_millis(1));
        let ended = Instant::now() + limit;
        let exchange = || -> Result<Vec<u8>, String> {
            let mut stream =
                UnixStream::connect(&path).map_err(|e| format!("connecting to `{socket}`: {e}"))?;
            stream
                .set_write_timeout(Some(limit))
                .and_then(|()| stream.set_read_timeout(Some(limit)))
                .map_err(|e| e.to_string())?;
            stream
                .write_all(&body)
                .map_err(|e| format!("writing to `{socket}`: {e}"))?;
            let mut reply = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = stream
                    .read(&mut buf)
                    .map_err(|e| format!("reading from `{socket}`: {e}"))?;
                if n == 0 {
                    return Ok(reply);
                }
                reply.extend_from_slice(&buf[..n]);
                if let Some(end) = reply.iter().position(|b| *b == b'\n') {
                    reply.truncate(end);
                    return Ok(reply);
                }
                if reply.len() > MAX_REPLY {
                    return Err(format!("the reply from `{socket}` passed 1 MiB"));
                }
                if Instant::now() >= ended {
                    return Err(format!("no reply from `{socket}` in time"));
                }
            }
        };
        let reply = exchange();
        if reply.is_err() && Instant::now() >= self.deadline {
            return Err(self.trap(Halt::Timeout));
        }
        Ok(reply)
    }

    fn sleep(&mut self, ms: u32) -> wasmtime::Result<()> {
        self.live()?;
        let until = Instant::now() + Duration::from_millis(u64::from(ms));
        loop {
            let now = Instant::now();
            if now >= until {
                return Ok(());
            }
            std::thread::sleep((until - now).min(SLEEP_SLICE));
            self.live()?;
        }
    }

    fn read_file(&mut self, path: String) -> wasmtime::Result<Result<Vec<u8>, String>> {
        self.live()?;
        let granted = self.grants.folder(&path, &self.places);
        let file = self.granted(granted)?;
        let read = || -> Result<Vec<u8>, String> {
            let metadata = std::fs::metadata(&file).map_err(|e| e.to_string())?;
            if !metadata.is_file() {
                return Err(format!("`{path}` is not a file"));
            }
            if metadata.len() > MAX_FILE {
                return Err(format!("`{path}` is over 16 MiB"));
            }
            std::fs::read(&file).map_err(|e| format!("reading `{path}`: {e}"))
        };
        Ok(read())
    }

    fn list_folder(&mut self, path: String) -> wasmtime::Result<Result<Vec<String>, String>> {
        self.live()?;
        let granted = self.grants.folder(&path, &self.places);
        let folder = self.granted(granted)?;
        let list = || -> Result<Vec<String>, String> {
            let mut names: Vec<String> = std::fs::read_dir(&folder)
                .map_err(|e| format!("listing `{path}`: {e}"))?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            Ok(names)
        };
        Ok(list())
    }
}

impl ResourceLimiter for State {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let allowed = desired <= self.max_memory_bytes;
        self.refused |= !allowed;
        Ok(allowed)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 10_000)
    }
}
