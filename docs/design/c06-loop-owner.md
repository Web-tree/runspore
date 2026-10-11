# C06 — Who owns the loop between steps

Design record of the grill held on 2026-10-11 for the map ticket **C06 — Who owns the loop between in-session steps** (Web-tree/runspore#11, map #5). It settles who moves a run between steps and how the person learns that a run moved. The answer is that nobody owns the loop. A run moves whenever a **driver** calls `spore`. A driver can be a Claude session following the run, a separate process, or an outside agent that visits runs when it decides to. Several drivers can act on one run at once, and C03's leases keep that safe. Nothing has to stay running between `spore` calls. Runspore gains one generic option, `spore continue --wait`, which drives a run until it needs a person. The plugin uses it to follow a run from the person's own Claude session. Its entry points are namespaced skills (`/ml-workflow:<name>`), and the pop-up menu stays as `/ml-workflow:menu`. An always-on spore daemon is optional, comes later and is out of this map. So is an orchestrator agent. No visual was drawn.

Two rules held throughout. Runspore stays generic: `continue --wait` names no Claude, and drivers are plain `spore` CLI users. Ask Max only what changes the outcome: two of the three round-1 questions were withdrawn into the settled block at Max's prompt ("do you remember about the asking me only the questions where my answers brings value?").

## Terms

- **Driver** — Whoever keeps a run moving between steps by running `spore`: a Claude session following the run, a separate process, or an outside agent that visits it. Several may act on one run; leases decide who does each step. *Avoid:* owner, controller, scheduler.
- **Following a run** — A Claude session running `spore continue <runId> --wait` in the background and reacting when it returns. The plugin's default driver. *Avoid:* watching (a watch is C03's wait on a holder handle), attaching.
- **Sub-skill** — One of the plugin's namespaced skills, typed as `/ml-workflow:<name>`. *Avoid:* subcommand (a mod command is never namespaced, C05).

Existing terms used as `CONTEXT.md` defines them: worker, run, unfinished run, async action, holder, watch, probe, entry menu, plugin, helper, interactive step, background step.

## Why

The ticket asked who runs the worker between steps: a worker the mod starts and supervises, `spore run --until-parked` called per action, or a worker the person starts in another terminal. It also asked how the mod learns that the run moved: by polling or by an event from the worker. Earlier tickets narrowed it down:

- **C03.** A worker exits at an async step (code 6), and probes run on demand. A watch needs a running worker.
- **C07.** An interactive step's result arrives only through a watch, so something must run while it works.
- **C08.** `spore continue` works one run in the foreground and left the background case to this ticket.
- **C05.** A process the mod starts is stopped on a clean session exit but survives a SIGKILL.

Max's words during the grill:

- Q1 (does the run keep moving after you close Claude): "it's really depends of case. we can have different situations, and workflow in the different places. we can have skill, that will run spore as cli and watch it's progress, react when it will return something to it. it can be a separate process, where we run spore and it starting claude only when node need it. or it can be an agent, that come to workflow when it decides to do so. For example, if we have some "orchestrator" agent, that watch different workflows and check it's status, maybe control it manually because of some external events and decisions based on that."
- On the settled block, which first proposed an always-on worker: "we don't HAVE TO run it constantly. It's up to the skill to decide. And how to run it. in general, we can create a daemon for the spore that will watch for the workflows that runs. but it should not be required."
- Q6 (the menu's name): "/ml-workflow:{sub-skills}".

## Locked decisions

### L1. No single owner: any driver may move a run (Q1, Max)

**Decision.** Spore has no notion of who owns a run's loop. Three ways of driving are first-class, and they all use only the `spore` CLI:

1. **A Claude session follows the run.** A skill runs `spore` in the background and reacts each time it returns (L3).
2. **A separate process.** It runs `spore continue --wait`, `spore run` or `spore worker` and starts Claude only when a step needs it, as C04 and C07's step commands do.
3. **An outside agent visits runs when it decides to.** An orchestrator reads `spore list` and `status`, and acts with `signal`, `resolve`, `cancel` or `continue` based on outside events.

Any mix of these on one run is safe: claims, leases and fences (kernel 0.1, C03) make sure each step's try runs once. When two drivers watch one held step, the store keeps the first result and records the second as late evidence (C03 R3).

**Rejected.**
- *A worker the plugin starts and supervises for the session's lifetime.* C05 measured that it survives a SIGKILL anyway, and it forces one way of driving on every workflow.
- *A worker the person must start in another terminal.* This is still possible as way 2, but it is not required.
- *One owner per run, recorded in the store.* That would mean a new lock with its own expiry, and it buys nothing over the existing leases.

### L2. Nothing has to run between `spore` calls; a daemon is optional and later (settled block, Max)

**Decision.** Every `spore` command picks up where the last one stopped: it sweeps due leases, probes held steps on demand (C03) and coordinates pending events. So a run needs no process alive while it waits. A spore daemon that keeps all of a repository's runs moving is a possible later addition. Nothing in this map depends on it, so it is out of this map (map Out of scope).

**Rejected.** *A detached `spore worker --until-idle` that the plugin starts on demand, one per database, stopping itself when every run waits on a person.* This was first proposed as standard practice, following how Bazel and Gradle run their background servers. Max: "we don't HAVE TO run it constantly … it should not be required." Plugin-side polling of `spore list` every 15 s went with it: the session that follows a run learns that the run stopped when `continue --wait` returns.

### L3. The menu's runs are followed by your Claude session (Q4 → A, Max)

**Decision.** When the person starts or continues a run from the plugin, their current Claude session follows it. Claude runs `spore continue <runId> --wait` as a background shell command and is woken when it returns. Claude then prints one line: the run needs you (and what for), it failed, or it is done. For a decision, Claude offers it right there. Closing Claude stops the background command and pauses the run. The run resumes when someone drives it again, for example with Continue in the menu or `spore continue` in a terminal.

**Rejected.**
- *B, a detached `continue --wait` per run that goes on after Claude closes.* It leaves a process nobody watches, spending money while the person is away.
- *C, ask on every start.* It adds a question to every start for a choice most runs don't need.

### L4. Entry points are namespaced sub-skills; the pop-up stays as `/ml-workflow:menu` (Q6 Max, Q7 → B, Max; my recommendation was A)

**Decision.** Everything the person types is a skill of the `ml-workflow` plugin, shown as `/ml-workflow:<name>`. Typing `/ml-workflow:` lists them all. C08's pop-up menu stays, opened by `/ml-workflow:menu`. The sub-skills:

| Sub-skill | What it does |
|---|---|
| `menu` | Opens C08's pop-up: unfinished runs and what each needs, workflows to start, write a new workflow |
| `start <workflow>` | C08's start flow (check, draft input, `spore start`), then follows the new run (L3) |
| `status [<runId>]` | Lists unfinished runs and what each needs (`spore list --unfinished --json`), or one run's state |
| `continue [<runId>]` | Follows a run (L3); with several unfinished and no ID, asks which |
| `cancel <runId>` | `spore cancel`, after a one-line confirmation |
| `author [<workflow>]` | C09's interview, writing a new workflow or changing one (C10 name) |

This replaces the bare `/spore` command. That name was a placeholder carried over from C08 and kept in C10's settled block; Max never chose it. A mod's own `$.command.register` command can't carry the plugin prefix (C05: `/spike-mod:spike` was "Unknown command"), so every entry point is a skill.

**Rejected.** *A, sub-skills only, with no pop-up, where typing `/ml-workflow:` serves as the menu.* Max kept the pop-up.

## Routine choices

- **Orchestrator agent (Q5 → A).** It comes later, as its own effort. This map only makes sure spore gives it what it needs: `list`, `status`, `events`, `step show` and the acting commands, all with `--json`. Recorded in the map's Out of scope.
- **Hearing that a run needs you (was Q2, withdrawn as standard).** It takes one line in the session that follows the run, printed when `continue --wait` returns: needs you, failed or done. Routine progress prints nothing, and there are no OS notifications. A run nobody follows shows in C08's session-start line and in `status`.
- **After a crash (was Q3, withdrawn).** It follows from L2 and L3. A crash of the session or the machine leaves the run as it was. The next session-start line names it (C08 R1), and Continue in the menu or `/ml-workflow:continue` resumes it. Claude steps resume their own conversation (C13), so nothing is redone from scratch. If only the session was SIGKILLed, the orphaned `continue --wait` may still be driving (C05); it exits when the run needs a person, and a second driver started meanwhile is safe (L1).
- **How the pop-up opens from a skill.** The mod hooks `skill.prompt` for `ml-workflow:menu`. The hook opens the pane with `$.ui.open` and returns a one-line text for the model ("The workflow menu is open."). If opening a pane from that hook is refused, the skill's own text tells Claude to call a mod tool, `open_menu`, registered with `$.tool.register`, which opens it. Both mechanisms appear in C05 (§Verified facts); opening a pane from `skill.prompt` was not measured.
- **What the pop-up's rows run.** C08's table holds, with two changes. "Continue" (`needs: working`) runs `$.command.run({command: 'ml-workflow:continue', args: runId})`, so Claude follows the run. A chosen workflow runs `ml-workflow:start <workflow>`. Decide, retry, release and cancel run their `spore` commands directly, as C08 lists them, and then hand to `ml-workflow:continue` when the run can move.
- **Sub-skill names.** `menu`, `start`, `status`, `continue`, `cancel` and `author` are my defaults, chosen to match the `spore` commands they wrap, with `author` from C10.

## Specification

### CLI 0.2: `spore continue --wait`

`spore continue [<runId>] --wait` behaves as C08's `continue`, except at held async steps:

1. **A held async step whose action declares a watch.** It runs the watch (C03 §Probe and watch contracts) instead of exiting 6. On a result, it finishes the attempt and goes on.
2. **A held async step with no watch.** It waits. While it waits it does three things:
   - It reloads the run every `poll_ms` (default 200 ms), so a `spore step complete` from the actor is seen.
   - It sweeps and probes the attempt at most every 30 s, as the worker's sweep does (C03).
   - It expires the attempt at its deadline or lease. The run then goes on by its effect class.
3. **It returns when the run needs a person or has ended.** Exit codes are as for `continue`:

| Exit | Meaning |
|---|---|
| 0 | Completed |
| 1 | Failed |
| 3 | Waiting for a signal |
| 4 | Needs intervention: a stuck step |
| 5 | Quarantined |
| 6 | Only for an external step that is ready and unclaimed, since a person must claim it |
| 7 | Cancelled |

4. **SIGINT or SIGTERM** stops it as the worker stops (cli.md §Signals): it stops claiming, gives in-flight work a short grace period, and exits. A running watch is stopped with its process group, and the held attempt stays held for the next driver.
5. With several unfinished runs and no `<runId>`, it lists them and exits 2, as `continue` does.
6. **`--json`** prints the same final report as `continue --json`.

`spore continue` without `--wait` is unchanged. So are `spore run` and `spore worker --until-parked`.

### Plugin: following a run

1. The `start` and `continue` skills tell Claude to run `spore continue <runId> --wait --json` as a background shell command, with `RUNSPORE_DB` set when the repository's database is not found by walking up.
2. When it returns, Claude reads the exit code and the JSON:
   - 3, a signal: Claude shows the step (`spore step show`) and offers the outcomes.
   - 4, stuck: Claude shows the kept error and the three `resolve` forms.
   - 6, an external step: Claude names the actor label and `spore step claim`.
   - 0, 1 or 7: one line with the end state.
   - 5, quarantined: the reason and `spore release`.
3. After the person acts, for example a decision sent with `spore signal`, the skill runs `continue --wait` again.
4. Steps keep the plugin's `bin/` on their `PATH`. Claude's own shell tool already has it (C05), so a `continue --wait` run from Claude's shell passes it on to the step commands it starts.

## Verified facts

Established by reading the repo, the C05 findings and the Claude Code 2.1.296 type file, not by asking:

- **`spore` 0.1.** `spore worker [--run <id>] [--until-parked]` works all runs until interrupted, or one run until it parks. `run` and `worker --until-parked` are the same loop (spec/cli.md). On SIGINT or SIGTERM the worker stops claiming, gives in-flight work a grace period, and exits.
- **C03 (§2, L-items).**
  - A worker exits 6 at an async step.
  - Probes run on demand from `status`, `continue` and `step claim`, and in a worker's sweep at most every 30 s per attempt.
  - A watch runs only while a worker runs.
  - Two result channels can report, and the store keeps the first.
- **C07 R15.** An interactive step reports only through its watch: herdr quiet plus a result file.
- **C08.** `continue` works one run in the foreground and exits as `spore run` does. The menu's "Continue" row deferred foreground versus background to C06.
- **C05 (measured).**
  - A mod's `$.command.register` command is bare: `/spike` worked and `/spike-mod:spike` was "Unknown command".
  - A plugin skill is namespaced (`/spike-mod:hello`).
  - `$.command.run({command: 'spike-mod:hello', args})` runs a skill.
  - `$.ui.open` opens a pane from a command, and a `Select` in it returns the pick.
  - `$.tool.register` tools reach a `tool.call` hook.
  - A mod-started process is stopped on a clean session exit and survives a SIGKILL.
- **Type file (Claude Code 2.1.296).** `skill.prompt` fires when a skill's prompt is expanded, whether it was typed as `/name`, called through the Skill tool or preloaded. A hook may return `{text}` to replace what the model reads.

## Risks

- **Opening the pop-up from `skill.prompt` is not measured.** The fallback is a mod tool. Each `/ml-workflow:menu` costs one short model turn, where C08's bare command cost none.
- **Clean exit stops a background shell: assumed, not measured.** L3 assumes Claude Code stops the background command when the session exits cleanly, as C05 measured for `$.process.spawn` children. If it doesn't, a run keeps moving after Claude closes. That is safe by L1, but it spends money unwatched.
- **Long background commands.** An interactive step may hold `continue --wait` for up to its 4 h deadline (C07). This relies on Claude Code's background shell having no time limit of its own.
- **A run nobody follows sits still.** It waits until a person continues it. That is the intended trade-off of L2 and L3, and the session-start line is the only reminder.
- **Two followers of one run** (two Claude sessions, or a session plus a terminal) each print the outcome. They are safe, but noisy.

## Deferred

None.

## Out of scope (added to the map)

- **An always-on spore daemon** that keeps all runs of a repository moving. It is optional and nothing depends on it (L2, Max).
- **An orchestrator agent** that watches many runs, possibly across repositories, and steps in on outside events (Q5, Max). This map keeps the CLI it would use complete.

## Changes to earlier records

- **C08** (`docs/design/c08-entry-menu.md`).
  - The command is `/ml-workflow:menu`, not `/spore`, and R1's example line says so.
  - The "Continue" row runs the `continue` sub-skill, which follows the run in the background (L3).
  - Starting a workflow ends by following the new run.
- **C10** (`docs/design/c10-plugin-home.md`). R7's `/spore` row: the menu is the skill `menu` (`/ml-workflow:menu`), not a mod command. Its risk "the menu's typed name is not known yet" is closed.
- **C03 and C07.** "A watch runs only while a worker runs" now also covers `continue --wait`, which runs watches for its one run.
- **Map.** Out of scope gains the daemon and the orchestrator. The spec split gains `cli.md` 0.2 `continue --wait`. The C05 line's "bare `/spore`" is superseded.
