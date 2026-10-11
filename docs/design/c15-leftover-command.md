# C15 — A dead worker's command keeps running

Design record of the grill held on 2026-10-11 for the map ticket **C15 — A dead worker's command keeps running** (Web-tree/runspore#25, map #5). It decides what happens to a command when the worker that started it dies without stopping it (SIGKILL, a crash, a closed terminal). In 0.1 the command's process group runs on: nobody records its result, and once the lease expires the next worker starts a new try beside it, so a repeatable step runs twice at once. For a Claude step that means two processes writing one conversation. From host 0.2 on, **spore stops a leftover command at once**, the same way for every command. A small guard process that spore puts between itself and each command, outside the command's process group, stops that group as soon as the worker dies. Before any new try of a step, spore also checks the earlier try's recorded process group and stops whatever is still running there on the same machine. A step therefore never runs twice at once on one machine. The step runs again when a driver next continues the run, and the action's effect class decides as before: a repeatable step retries, an `unsafe` one waits for a person. The kernel is unchanged; the changes land in host 0.2, store 0.2 and CLI 0.2. No visual was drawn.

Max answered the one question that changes the outcome (Q1 → A). The rest was settled from standard practice, shown on the grill page as settled and open to reopening, and Max pressed Finish without reopening any of it.

## Terms

- **Leftover command** — A command that is still running after the worker that started it has died, so nobody will record its result. *Avoid:* orphan (an OS term for any re-parented process), zombie (an exited process not yet reaped), stray process.
- **Guard** — The small process spore starts in front of each command it runs for a step. It stays outside the command's process group, in a group of its own, and stops the command's group when the worker dies. It is spore's own mechanism, never named in a workflow. *Avoid:* wrapper (the plugin's `claude-spore` role), supervisor, shim.
- **Process record** — What an attempt keeps about the process group doing its work: the machine, the boot, the group ID and the start time of the command that leads it. Spore writes and reads it itself. It is not a holder handle: a holder handle belongs to an async action and spore never reads it (C03 L3). *Avoid:* holder handle, PID file.

## Why

The ticket's question: "When a worker dies without stopping its command (SIGKILL, a crash), the command's process group keeps running. The lease then expires, and the next worker starts a new try beside it, so a repeatable step runs twice at once. Should `spore` stop a dead worker's leftover command before the retry, and how? Or does each command stay responsible, as C04's wrapper is now with its parent watch?"

C04 measured it (findings, §Kill and resume): "A SIGKILLed worker kills nothing, so the group runs on. Without a guard, the wrapper's `claude -p` finished its work after its worker was dead, and nobody recorded the result. The next worker expired try 1 after its lease (10 s by default) and started try 2, which resumes the same session. An orphan still running at that point shares the conversation with the new try (verified: no lock, both write one transcript)." C04 called it "a generic gap, not a Claude one. Any long command keeps running after its worker dies, and a repeatable effect then runs twice at once."

C06 made the gap more likely. A run moves whenever a driver calls `spore`, and the plugin's own Claude session follows a run with `spore continue --wait` in a background shell. A Claude session that crashes, or a terminal that closes, takes that worker down without a graceful shutdown. Max chose that closing Claude pauses the run (C06 L3), so a crash should pause it too, not leave work running unseen.

## Locked decisions

### L1. A dead worker's command is stopped at once (Q1 → A, Max)

**Decision.** When the worker that runs a step dies, spore stops the step's command right away. Nothing is recorded for that try; the attempt's lease runs out as in 0.1 and the attempt ends as `expired`. When a driver next continues the run, kernel 0.1's rule applies: a repeatable effect (`pure`, `read-only`, `idempotent`) gets a new try of the same visit, and an `unsafe` effect parks the run in `needs-intervention` for a person to `resolve`. A crash therefore acts like closing Claude (C06 L3): nothing keeps running or spending money unseen. A Claude step loses little, because its retry resumes the same conversation (C07, C13), so only the turn in flight is redone.

Rejected:
- *B — the command runs to the end and its result is kept; no new try starts while it runs.* This is Docker's live-restore and containerd's shim: the job outlives its daemon and reports when the daemon is back. It saves the in-flight work, but every command then needs a keeper that renews the lease and writes the result to the database on its own, which makes each command a small worker. It also keeps a two-hour Claude step spending money after the person's session crashed, against C06's "closing Claude pauses the run". A command that should outlive its worker already has a way to: declare it async (C03), as L5 says.
- *C — each command looks after itself, as C04's wrapper does with its parent watch.* Only commands written for it are covered; tests, scripts and every other command still run twice. It also contradicts the standing preference that Runspore's capabilities are generic.

### L2. Spore stops leftovers for every command, not each command on its own (standard practice)

**Decision.** The guard runs in front of every process spore starts for a step: a sync `command`, an async start command, a probe and a watch (C03). No action opts in or out, and no workflow names the guard. Only the processes that do a step's work (a sync command, an async start command) also get a process record (L3); a probe or watch is read-only, so a leftover one does no harm beyond its cost, and its late result is a stale duplicate the store already ignores (C03 R3). The plugin's wrapper may drop its own `getppid()` watch (C04); keeping it does no harm, since with the guard as its parent it never fires while the guard lives.

This is how supervisors treat their children: systemd's default `KillMode=control-group` stops every process of a unit when the unit stops, Kubernetes stops all of a pod's containers when the pod goes, and CI runners clean up a job's leftover processes after the job.

### L3. A step never runs twice at once on one machine (standard practice)

**Decision.** Before an attempt whose command is still recorded (a sync command, or an async start command that has not yet handed over its holder handle) is expired for a new try, whoever expires it reads its process record. If the earlier try's process group is still alive on this machine, spore stops it and waits until it is gone. Only then does it expire the attempt. The guard (L2) normally stops the group within seconds of the worker's death, long before the 10 s lease runs out. The check before the retry covers the case where the guard died too, for example `pkill -9 spore`, which matches both.

This is the fencing rule of leader-elected systems: the old holder must be gone before the new one starts. Rejected: relying on the guard alone. A guard can be killed with its worker, and then the retry would start beside the leftover again.

### L4. Stopping means: ask, wait, then force (standard practice)

**Decision.** Every time spore stops a process group (worker death, the check before a retry, a timeout, a stop on shutdown) it sends SIGTERM to the group, waits up to `stop_grace_ms` (default 5000), then sends SIGKILL. Host 0.1 sends SIGKILL right away; 0.2 changes that for all of these cases, so a command can save its state (a Claude transcript, a test report) before it goes.

The same sequence as `docker stop` (10 s), Kubernetes (30 s `terminationGracePeriodSeconds`) and systemd (`TimeoutStopSec`). Five seconds keeps a resumed run's wait well inside one lease.

### L5. A command meant to outlive its worker is async (standard practice)

**Decision.** A job that must survive a worker's death is declared async (C03): its start command prints a holder handle and exits, the job runs outside the start command's process group, and the action's own probe tells spore whether it is alive. The guard stops only the start command's group, so a job started this way is never touched by it. Its probe, watch and deadline govern it as C03 says. The two cases have different owners: a sync command's process is spore's own, so spore keeps its process record; an async job's identity is the action's, so spore keeps its holder handle without reading it.

### L5a. An async start is repeatable by its effect key (standard practice; added after review)

**Decision.** One window is left that no process record covers: the start command has detached its job and exited, but the worker dies before `hold_attempt` stores the holder handle. The job then runs with no identity in the store, and the new try runs the start command again. An async action declared repeatable (`idempotent` and below) must therefore make its start command repeatable by `RUNSPORE_EFFECT_KEY`, the same on every try of one visit: before starting a job, it looks for one it already started under that key and, if it finds a live one, prints that job's handle instead of starting a second. A start command that cannot do this belongs to an action declared `unsafe`, whose expired attempt waits for a person. This is the effect-class contract host 0.1 already states ("an idempotent command passes it to whatever it calls"), applied to the start; it is how Kubernetes Jobs and Temporal activities make a repeated create safe. C07's interactive Claude step meets it by naming its herdr pane and Claude session after the run and node (C13).

Rejected: a second handshake in which the start command keeps the job on a leash until spore confirms the handle is stored. It needs a new channel beside stdin (which carries the input and then closes) and moves the same burden into every start command.

### L6. A database shared by several machines is left open (standard practice; map fog)

**Decision.** A process record written on another machine, or by another OS user, cannot be checked from here. In that case the attempt is expired by its lease exactly as in 0.1, and the audit record notes `process.unchecked`. The risk is the one 0.1 already has, now limited to setups the map has not designed yet. It joins the map's open question on two sessions or repositories sharing one database. SQLite's own guidance is against sharing one database file over a network file system, so the case is rare today.

## Routine choices

- **The guard is spore itself.** It is a hidden mode of the same binary, so there is nothing extra to install, and it is in the release and `cargo install` builds by construction (C10).
- **The guard learns of the worker's death through a pipe.** The worker keeps the write end; the guard reads it. When the worker dies for any reason, the operating system closes the write end and the guard reads end-of-file. This is portable across Linux and macOS. Linux's `PR_SET_PDEATHSIG` has no macOS counterpart, and polling `getppid()` costs a timer per command.
- **The guard stands outside the group it stops.** The guard runs in a session and group of its own, apart from both the worker's group and the command's. A signal to the command's group therefore never reaches the guard, so it can wait out `stop_grace_ms` and send SIGKILL; and a hangup sent to the worker's terminal group kills the worker but not the guard. (Changed after review: the first draft had the guard lead the command's group, where its own SIGTERM would have ended it.)
- **Order of a start.** The worker starts the guard. The guard forks the command's process, which makes itself the leader of a new group, resets its signal mask and dispositions, and waits on a pipe before running the program. The guard reports the process ID and start time to the worker; the worker records them with the attempt (`set_process`), then writes one byte to the guard's control pipe, and only then does the guard let the program run. A worker that dies before recording anything leaves a guard that sees end-of-file before the go byte; it kills the waiting child and exits, so there is never a working process the store does not know about.
- **The guard is transparent.** The command inherits the stdin, stdout and stderr pipes directly. The guard closes its own copies and reports the command's ending (its exit code, or the signal that ended it) to the worker on a second pipe, so host 0.1's result table is unchanged (`command.signaled` stays "a signal it did not get from the engine").
- **Same process, same machine.** The record keeps the host name, a boot identifier (`/proc/sys/kernel/random/boot_id` on Linux, `kern.boottime` on macOS), the process group ID (the command's own process ID) and the command's start time. A different boot means the group is gone. A group whose leader is alive with another start time belongs to someone else and is left alone. A group whose leader has exited but whose members remain (a shell that left children behind) counts as ours when every member started at or after the recorded start time: an operating system does not hand out a group's ID again while any member of it lives. (Changed after review: the first draft recorded the guard's start time, which is unreadable once the guard is gone, exactly the case the check exists for.) C03 gives the same advice to probe writers ("compare the process start time, not just the number").
- **SIGHUP.** The worker keeps the default action for SIGHUP (only SIGINT and SIGTERM are handled), so a closed terminal kills it without a graceful shutdown. That is acceptable now: the guard stops the command either way. Handling SIGHUP like SIGTERM is a routine CLI 0.2 refinement, not required by this record.

## Specification

### Host 0.2 — command processes

Replaces the last bullet of host 0.1 §3 `command` ("It runs in its own process group. On timeout or stop the whole group is killed."):

- Every process the host starts for an attempt runs under a guard. This covers a `command`'s `argv`, an async start command, `async.probe` and `async.watch`. The guard runs in its own session and group; the process it starts leads a new group of its own, with default signal dispositions and an empty signal mask, and runs its program only once the host has said go. The guard stops the process's group when the host's end of its control pipe closes without a clean stop.
- Stopping a group means SIGTERM to the group, a wait of up to `stop_grace_ms` (engine config, default 5000), then SIGKILL. It is used on timeout, on stop (lease lost, shutdown) and when the guard sees the host die. On a normal ending the group is stopped at once, as in 0.1, so nothing the command left behind outlives it.
- Before a sync command or an async start command is let run, the host records its process record with `set_process` (store 0.2). A failure to record is a `Failure`, `command.spawn`, retryable; the guard then kills the waiting process and exits. The record is cleared (`set_process` with null) when the command's group has been stopped after it ended, and by `hold_attempt` when an async start hands over its holder handle. Probes and watches are guarded but not recorded.
- **Before expiring an attempt with a process record** (the sweep's `expire_attempt` with reason `lease`), the host reads the record. Same host name and boot: if the group is ours (its leader is alive with the recorded start time, or it has lost its leader and every member started at or after that time), stop it (above) and wait until no process of the group remains (bounded by `stop_grace_ms` plus 1 s). Then expire. Another boot: expire. Another host, or a group the host may not signal (EPERM): expire, with `process.unchecked` in the audit record. A group that is still there after SIGKILL and the wait leaves the attempt unexpired; the sweep tries again on its next pass.
- New config: `stop_grace_ms` (5000).

### Store 0.2 — process record

- An attempt carries `process`: canonical JSON `{"host": "<name>", "boot": "<id>", "pgid": <int>, "started": "<decimal>"}` or null. It is written by `set_process(attemptRef, process)`, with the same owner, attempt and fence predicates as `heartbeat`. It is read through `InvocationView` and `scan_due`'s lease items, so the sweeping host has it without a second query.
- `process` and C03's `holder` are separate fields. Spore reads `process` itself and never reads `holder`.

### CLI 0.2

- `--stop-grace-ms` joins the worker tuning flags (`--lease-ms`, `--heartbeat-ms`, …) for `worker`, `run` and `continue`.
- No new exit codes. `status --json` shows an attempt's `process` record next to its `holder`.

### Evidence (host 0.2 required evidence)

| ID | Case | Expectation |
| --- | --- | --- |
| H6 | SIGKILL the worker while a sync command runs | The command's group is gone within `stop_grace_ms` + 1 s; the next worker's try starts only after it |
| H7 | SIGKILL the worker and its guard together, the command's group still running (with and without its leader) | The next sweep stops the recorded group before expiring the attempt; no two tries overlap |
| H7b | A command that ignores SIGTERM, worker killed | The guard survives its own SIGTERM to the group and sends SIGKILL after `stop_grace_ms` |
| H8 | A command that traps SIGTERM and writes a file | The file is written before the group goes; a command that ignores SIGTERM is gone after `stop_grace_ms` |
| H9 | A process record from another host name | The attempt is expired by its lease; the audit record says `process.unchecked` |
| H10 | An async start command that started a detached job, then the worker is killed after `hold_attempt` | The job survives; its probe and deadline govern it (C03) |
| H11 | The worker and guard are killed while an async start command still runs | The next sweep stops the start command's recorded group before the new try |

## Verified facts

- **Today's command runner.** `crates/runspore-host/src/command.rs` starts each command with `.process_group(0)` and `.kill_on_drop(true)` (lines 178–179). Its `kill_group` sends SIGKILL to the group right away (line 128): there is no polite stop today. `kill_on_drop` acts only while the worker process is alive to drop the child; a SIGKILLed worker drops nothing.
- **The worker's signals.** `crates/runspore-cli/src/cli.rs` installs handlers for SIGINT and SIGTERM only (line 492); SIGHUP keeps its default action and ends the worker with no graceful shutdown.
- **The leftover and the wrapper's fix were measured in C04.** Without a guard, `claude -p` finished after its worker was dead and its result was lost; try 2 resumed the same session while the leftover still wrote to it. With the wrapper polling `getppid()` every second, no Claude process remained 2 s after the SIGKILL, and try 2 resumed cleanly. ([findings, §Kill and resume](https://github.com/Web-tree/runspore/blob/research/c04-headless-agent-step/docs/research/c04-headless-agent-step.md#kill-and-resume))
- **A chatty command often dies on its own; a quiet one does not.** A leftover's stdout and stderr are pipes whose reading end died with the worker. Its next write gets SIGPIPE, or EPIPE where SIGPIPE is ignored, as in Rust programs. `claude -p` writes once at the end, which is why it ran on in C04. This is known POSIX behaviour, not measured here, and it is why L1 cannot rely on it.
- **Async jobs already leave the start command's group.** Host 0.1 stops the group when the command ends. A job an async start command leaves running (C03, C07's herdr pane or detached `claude -p`) must therefore already be in its own group or session, so the guard never reaches it.

## Risks

- **A guard killed with its worker leaves the command running until the next sweep.** The check before a retry (L3) still prevents two tries at once, but the leftover may run until a driver comes back. C06 says that may be much later. Its result is lost either way.
- **A wrong "same process" would stop a stranger's group.** The start-time check makes this need a reused group ID with an identical start time on the same boot. Fail-safe direction: when the check cannot be made, spore does not signal anything.
- **Several machines on one database.** Covered by L6 only as far as 0.1 was: a leftover on another machine can still run beside a retry. It joins the map's open question on sharing a database.
- **An async start that is not repeatable but is declared so.** L5a's window (the worker dies between the start's exit and `hold_attempt`, a few milliseconds) then starts a second job beside the first. The declaration is the author's promise; C09's authoring interview proposes the effect class.
- **A command that starts its own process group or session** (a daemon, `setsid`) escapes the guard, just as it escapes 0.1's timeout kill. A command that does this on purpose should be an async action (L5).
- **One more process per running step.** It costs a few megabytes of memory per guard and a fork at start, negligible next to the commands it guards.

## Deferred

None.

## Open threads

None.

## Changes to earlier records

- **C04 findings, §Kill and resume.** The wrapper's `getppid()` watch becomes optional. Spore stops the group itself (L2). The plugin decides whether to keep it, in `max-skills`.
- **C07, risk "a wrong *dead* puts two sessions on one conversation".** For sync background steps (`claude -p` run by a `command`), L3 now makes two tries at once impossible on one machine. For async steps it still rests on the probe (C03).
- **C03, §Spec, async start.** A start command of a repeatable action must be repeatable by `RUNSPORE_EFFECT_KEY` (L5a).
- **C03, §Spec, probe and watch.** These still run "with the `command` rules (own process group)". Under host 0.2 that includes the guard and L4's stop sequence.
- **Map, spec split.** C15 adds: `host.md` 0.2 the guard, `stop_grace_ms`, the stop sequence, the check before expiring an attempt with a process record, `process.unchecked`, the repeatable async start (L5a), evidence H6–H11; `store.md` 0.2 the attempt's `process` field and `set_process`; `cli.md` 0.2 `--stop-grace-ms`.
- **Map, concurrency fog.** It gains: "C15: a process record from another machine or OS user cannot be checked; such attempts expire by lease as in 0.1."
