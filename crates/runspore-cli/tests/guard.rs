//! H12, H14, H15: a worker SIGKILLed while its command runs. The guard stops the
//! command's process group (SIGTERM, `--stop-grace-ms`, SIGKILL), and the next try
//! starts only after the group is gone.

mod common;

use std::time::{Duration, Instant};

use common::Dir;
use nix::errno::Errno;
use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};

/// A one-step run whose command logs `<attempt> <process group>` to `tries.log`, then
/// runs `body`.
fn flow(body: &str) -> Value {
    let script = format!("echo \"$RUNSPORE_ATTEMPT $$\" >> tries.log; {body}");
    json!({
        "format": "runspore.workflow/0.2", "name": "guarded", "start": "work",
        "actions": {"work": {"kind": "command", "effect": "idempotent",
                             "retry": {"maxAttempts": 3, "backoffMs": "10"},
                             "argv": ["/bin/sh", "-c", script]}},
        "nodes": {
            "work": {"kind": "activity", "action": "work", "outcomes": {"ok": "done"}},
            "done": {"kind": "complete"}
        }
    })
}

/// The process groups of the tries so far, in order.
fn tries(dir: &Dir) -> Vec<i32> {
    std::fs::read_to_string(dir.file("tries.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split(' ').nth(1)?.parse().ok())
        .collect()
}

fn alive(group: i32) -> bool {
    killpg(Pid::from_raw(group), None) != Err(Errno::ESRCH)
}

/// Starts a worker on `body`, waits for try 1, and SIGKILLs the worker. Returns the
/// try's process group and when the worker was killed.
fn kill_worker_mid_command(dir: &Dir, body: &str, flags: &[&str]) -> (i32, Instant) {
    dir.write("flow.json", &flow(body));
    let mut argv = vec!["--native-kernel", "--poll-ms", "20"];
    argv.extend_from_slice(flags);
    argv.extend_from_slice(&["run", "flow.json", "--key", "g"]);
    let mut worker = dir.spawn(&argv);
    let deadline = Instant::now() + Duration::from_secs(10);
    while tries(dir).is_empty() {
        assert!(Instant::now() < deadline, "try 1 never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    let group = tries(dir)[0];
    // Let the command set up its traps and children.
    std::thread::sleep(Duration::from_millis(200));
    kill(Pid::from_raw(worker.id() as i32), Signal::SIGKILL).unwrap();
    let killed = Instant::now();
    let _ = worker.wait();
    (group, killed)
}

/// Waits until `group` is gone and returns how long after `since` that was.
fn gone_after(group: i32, since: Instant, limit: Duration) -> Duration {
    while alive(group) {
        assert!(
            since.elapsed() < limit,
            "group {group} still alive after {limit:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    since.elapsed()
}

#[test]
fn h12_a_killed_workers_command_is_stopped_before_the_next_try() {
    let dir = Dir::new("h12");
    let flags = ["--lease-ms", "2000", "--stop-grace-ms", "500"];
    let (group, killed) = kill_worker_mid_command(&dir, "sleep 30 & wait", &flags);
    // The next worker starts at once; its try waits for the lease.
    let mut argv = vec!["--native-kernel", "--poll-ms", "20"];
    argv.extend_from_slice(&flags);
    argv.extend_from_slice(&["run", "flow.json", "--key", "g"]);
    let mut next = dir.spawn(&argv);
    let mut gone = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while tries(&dir).len() < 2 {
        assert!(Instant::now() < deadline, "try 2 never started");
        if gone.is_none() && !alive(group) {
            gone = Some(killed.elapsed());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = next.kill();
    let _ = next.wait();
    let gone = gone.expect("try 2 started while try 1's group was alive");
    assert!(
        gone < Duration::from_millis(500 + 1000),
        "the group outlived the grace by {gone:?}"
    );
}

#[test]
fn h14_a_command_ignoring_sigterm_is_killed_after_the_grace() {
    let dir = Dir::new("h14");
    let flags = ["--lease-ms", "5000", "--stop-grace-ms", "1000"];
    let (group, killed) = kill_worker_mid_command(&dir, "trap '' TERM; sleep 30", &flags);
    std::thread::sleep(Duration::from_millis(500).saturating_sub(killed.elapsed()));
    assert!(alive(group), "the group went before the grace was over");
    let gone = gone_after(group, killed, Duration::from_millis(1000 + 1000));
    assert!(gone >= Duration::from_millis(1000), "gone after {gone:?}");
}

#[test]
fn h15_a_command_trapping_sigterm_saves_its_work_before_it_goes() {
    let dir = Dir::new("h15");
    let flags = ["--lease-ms", "5000", "--stop-grace-ms", "3000"];
    let body = "trap 'echo saved > saved.txt; exit 0' TERM; sleep 30 & wait";
    let (group, killed) = kill_worker_mid_command(&dir, body, &flags);
    let gone = gone_after(group, killed, Duration::from_millis(3000 + 1000));
    assert!(gone < Duration::from_millis(3000), "took the whole grace");
    assert_eq!(
        std::fs::read_to_string(dir.file("saved.txt")).unwrap_or_default(),
        "saved\n"
    );
}
