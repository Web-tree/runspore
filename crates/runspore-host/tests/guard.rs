//! The guard on its own: the program runs only after the host says go, and a host
//! that goes away stops the program's group.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::killpg;
use nix::unistd::Pid;
use runspore_host::guard::{Ending, Guarded};

const GUARD: &str = env!("CARGO_BIN_EXE_runspore-guard");

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("runspore-guard-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn spawn(dir: &Path, script: &str, grace: Duration) -> Guarded {
    let argv = ["/bin/sh".to_string(), "-c".to_string(), script.to_string()];
    Guarded::spawn(Path::new(GUARD), grace, &argv, |command| {
        command
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    })
    .await
    .expect("guard started")
}

fn alive(group: i32) -> bool {
    killpg(Pid::from_raw(group), None) != Err(Errno::ESRCH)
}

async fn wait_gone(group: i32, limit: Duration) {
    let deadline = Instant::now() + limit;
    while alive(group) {
        assert!(Instant::now() < deadline, "group {group} still alive");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_program_runs_only_after_go() {
    let dir = temp("go");
    let mut guarded = spawn(&dir, "echo ran > ran.txt", Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(guarded.pid()), "the held process is not there");
    assert!(!dir.join("ran.txt").exists(), "ran before go");
    guarded.go().await.unwrap();
    assert_eq!(guarded.wait().await, Ending::Exited(0));
    assert!(dir.join("ran.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_host_gone_before_go_leaves_nothing_running() {
    let dir = temp("never");
    let guarded = spawn(&dir, "echo ran > ran.txt", Duration::from_secs(5)).await;
    let group = guarded.pid();
    drop(guarded);
    wait_gone(group, Duration::from_secs(2)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!dir.join("ran.txt").exists(), "ran without go");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_program_that_cannot_run_is_reported_at_go() {
    let dir = temp("missing");
    let argv = ["/no/such/program".to_string()];
    let mut guarded = Guarded::spawn(Path::new(GUARD), Duration::from_secs(1), &argv, |c| {
        c.current_dir(&dir);
    })
    .await
    .unwrap();
    assert!(guarded.go().await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_host_gone_mid_run_stops_the_group_with_sigterm_first() {
    let dir = temp("gone");
    let script = "trap 'echo saved > saved.txt; exit 0' TERM; sleep 30 & wait";
    let mut guarded = spawn(&dir, script, Duration::from_secs(5)).await;
    guarded.go().await.unwrap();
    let group = guarded.pid();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(guarded);
    wait_gone(group, Duration::from_secs(5)).await;
    assert_eq!(
        std::fs::read_to_string(dir.join("saved.txt")).unwrap(),
        "saved\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_stop_kills_a_group_that_ignores_sigterm_after_the_grace() {
    let dir = temp("stop");
    let grace = Duration::from_millis(500);
    let mut guarded = spawn(&dir, "trap '' TERM; sleep 30", grace).await;
    guarded.go().await.unwrap();
    let group = guarded.pid();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let asked = Instant::now();
    assert_eq!(guarded.stop().await, Ending::Signaled(9));
    assert!(asked.elapsed() >= grace, "killed before the grace");
    wait_gone(group, Duration::from_secs(1)).await;
    let _ = std::fs::remove_dir_all(&dir);
}
