# C08 — The entry menu and spore continue

This is the design record of the grill held on 2026-10-11 for the map ticket **C08 — The entry menu and spore continue** (Web-tree/runspore#13, map #5). It settles three things: where a repository keeps its workflows and runs, what `spore continue` does, and what the Claude Code menu shows.

**Runspore side.** A repository keeps everything in one **project folder**, `.spore/`, at its root.
- The workflows are under `.spore/workflows/<name>/` and are committed.
- The run database and its step files sit beside them and are git-ignored.
- `spore` finds the folder by walking up from where it stands, as git finds `.git`.

A repository may have several **unfinished runs** side by side. Keeping them from clashing is the workflow's own business, for example a step that creates a worktree; Runspore takes no position.

Three generic additions:
- `spore cancel` ends a run as cancelled. It is the cancellation that 0.1 left out.
- A workflow may declare an `inputSchema` that `spore start` checks.
- `spore continue [<runId>]` does the next thing a run needs, or says exactly what a person must do.

**Plugin side.** When a session opens it prints one line naming any waiting run. The menu opens only on the plugin's command. Each menu action is a shortcut for a `spore` command. Starting a run asks for the workflow's declared inputs, and Claude offers a draft taken from the conversation.

There is no visual.

## Terms

- **Project folder** — The `.spore/` folder at a repository's root, holding the repository's workflows, which are committed, and its run database, which is not. *Avoid:* workspace, config dir, `.runspore`.
- **Unfinished run** — A run that has neither completed, failed nor been cancelled. It is working, waiting for a signal, paused at a stuck step, held by an outside actor, or quarantined. *Avoid:* open run, active run, pending run.
- **Entry menu** — What the person sees after typing the plugin's command in their own Claude session. It lists the unfinished runs, each with what it needs, then the workflows that can be started, then "write a new workflow". *Avoid:* dashboard, launcher.
- **Cancel** — Ending an unfinished run on purpose. The run's status becomes `cancelled`, its history is kept, and a step in progress is told to stop. *Avoid:* abort (a crash), terminate, drop.
- **Input schema** — The shape of the input a workflow accepts, declared in its `settings.json` and checked when a run starts. *Avoid:* parameters (C14's node parameters belong to one node), arguments.

## Why

The ticket asks what the mod's entry command shows and what `spore continue` does. It names four open points:
- what each action shows before and after it is chosen;
- how the menu finds workflows and runs, and so where workflows and the database live in a repository;
- what `continue` does with zero, one or several unfinished runs;
- whether the menu opens on session start or only on the command.

Inputs from closed tickets:
- **C07:** the person's own session never takes a step; the menu shows stuck steps and interactive steps; "start a run" runs the plugin's check first.
- **C13:** start keys should not let two databases share a conversation.
- **C16:** run IDs come from spore, never from hashing a start key.
- **C14:** `spore step show` gives a waiting step's description without claiming it.

Max's words that shaped the result:
- On how many runs a repository may have, Max chose several side by side (Q3 → B). This overturned my recommendation of one at a time.
- On a second run in the same checkout (Q9): "it should be a workflow definition. For example, if workflow has a step with worktree creation, then it will be created. Runspore and claude code node and other parts not opinionated about that."
- Max asked during Q7: "do you think that we haven't drifted and still stick to the neutral workflows, not stick to AI-agents specifics?" The check found two places where plugin words had leaked into Runspore behaviour: "pane" in `continue`, and "closes the pane" in cancel. Both were rewritten in generic terms (R2, L4). R8 records the split.

## Locked decisions

### L1. One project folder per repository (Q1 → A, Max)

**Decision.** A repository keeps its workflows and its runs in one hidden folder at its root, `.spore/`.
- Workflows live in `.spore/workflows/<name>/` and are committed. Each is a C02 workflow folder.
- The run database `.spore/runspore.db` and C14's step-files folder `.spore/runspore.db.files/` sit beside them. They are kept out of git by an ignore file that spore writes in `.spore/`.

The database lives inside the checkout, so runs belong to that copy of the code. Two checkouts of one repository never share a run.

**Rejected.**
- *B — workflows in a visible top-level folder, the database in a hidden one.* Lost because it spreads one tool over two places that must be found, ignored and deleted separately.
- *C — workflows in the repository, the database in the home folder.* Lost because it loses the tie between a run and the checkout whose files it edits.

### L2. Several unfinished runs may share a repository (Q3 → B, Max; my recommendation was A)

**Decision.** Any number of runs may be unfinished at once.
- The menu lists them all and always offers to start another.
- `spore continue` with several asks for one by ID.
- The map's lock "start a run of a workflow when none is running" is amended to "start a run of a workflow".

**Rejected.**
- *A — one at a time; the menu hides "start" while a run is unfinished and offers "drop this run".* My recommendation, because two runs editing one checkout overwrite each other. It lost to Max's call: runs need not edit the checkout, and keeping runs apart is the workflow's job (L3).
- *C — one at a time with no way to drop a run.* Lost for the same reason, and because an unwanted run would block the repository.

### L3. Keeping runs apart is the workflow's business (Q9, Max's text answer)

**Decision.** When a second run starts in a checkout where another is working, nothing warns or intervenes. A workflow that needs isolation does it in its own steps, for example a first step that creates a git worktree. Runspore, the menu and the Claude node take no position on it.

A run started from the main checkout keeps its records in that checkout's `.spore/` even when its steps work in a worktree. The run belongs to where it was started, not to where its files are edited.

**Rejected.**
- *A — it starts, but the menu first names the run already working in this checkout.* My recommendation. Lost because it makes the plugin opinionated about what a workflow does to files.
- *B — the menu offers a separate git worktree for the new run.* Lost for the same reason, and because the new copy would have its own database (L1), so its runs would vanish from this menu.

### L4. Runspore gains cancel (Q10, standard practice, not asked)

**Decision.** `spore cancel <runId>` ends an unfinished run with status `cancelled`, at once and in one store operation. Its events and invocations are kept, and a step in progress is told to stop:
- **The run is closed in the store, not by the kernel.** Like a quarantine, the cancel is recorded on the run, and from that moment no worker can coordinate, dispatch, claim, heartbeat or finish anything of it. A quarantined run can be cancelled too: the cancel clears the quarantine as part of the same operation. Pending events stay unconsumed, as evidence.
- **A running command:** its worker's next heartbeat is refused, so it stops the command's process group, as on a lost lease (host §2), and stores what it knows as late evidence.
- **A held async step:** the attempt is expired with reason `cancelled`, which fences out the holder. If the action declares an optional `async.stop` command, `spore cancel` then runs it with the holder handle. It is declared like `probe` and `watch`, and is best effort.
- **The plugin's `claude-spore stop`:** this is what closes an interactive step's herdr pane.

This is Temporal's *terminate*, not its *cancel*: the workflow gets no say and runs no clean-up steps. The kernel is unchanged. Cancelling a run that has already ended succeeds and changes nothing. The menu offers cancel on every unfinished run. This reverses ADR M12 ("no cancellation in 0.1") for 0.2.

**Why it was settled, not asked.** Every workflow engine has it: Temporal cancel and terminate, Step Functions StopExecution, Argo stop. L2 makes it necessary, because without it an abandoned run would sit in the menu and in the session-start line forever.

**Rejected.**
- *No cancel; end a run through its own steps (fail the stuck step, reject at the approval).* It forces a fake ending and does not work for a run that is waiting on an outside actor.

## Routine choices

- **R1. The session-start line (Q2 → A, Max).** When a session opens in a repository with unfinished runs, the plugin prints one line, for example "1 run waiting: `coding-loop` at `approve` (decision), 2 h. /spore to open". The menu opens only on the command. A session with no unfinished runs prints nothing.
  - *Rejected:* opening the menu by itself, which interrupts every session in that repo; printing nothing, which lets runs sit forgotten.
- **R2. What `continue` does (Q4, standard practice).** It does the next thing the run needs, or says exactly what a person must do. Zero, one and several runs are covered in [§`spore continue`](#spore-continue).
  - It knows nothing of Claude. A held step is reported with its actor label and holder handle (C03).
  - It mirrors `git rebase --continue` and Terraform: resume, or name the blocker.
- **R3. Run IDs come from spore (Q5, settled by C16).** The menu and `continue` take run IDs from spore's output or `spore list`, never from a start key. C13's risk of two databases sharing a conversation is gone: run IDs are random per run (C16 R6).
- **R4. Spore finds the project folder by walking up (Q6, standard practice).** The order is:
  1. `--db`;
  2. `$RUNSPORE_DB`;
  3. `.spore/runspore.db` in the nearest folder at or above the current one that holds a `.spore/` directory;
  4. `./runspore.db`, as in 0.1, when no `.spore/` exists up to the filesystem root.

  It works as git, cargo and npm find their project, so the menu and plain `spore` agree on one database wherever you stand in the repo.
- **R5. Starting a run asks for declared inputs, with Claude's draft (Q7 → A, Max).** The workflow lists the inputs it needs (R6). The menu asks for each one, and Claude offers a draft from the conversation, which the person confirms or edits. Only the draft is plugin behaviour.
  - *Rejected:* asking with no draft, which makes the person retype what they just explained; asking in the first step, which puts a question inside a run that may be working where nobody sees it.
- **R6. `inputSchema` (Q11, standard practice).**
  - `settings.json` may hold `inputSchema` in C12's schema subset.
  - `spore start` refuses input that does not fit it with `input.invalid`, exit 2, whoever calls it.
  - A workflow without one accepts any input, as in 0.1.
  - Each property's `description` is the question the menu asks.
  - Source: GitHub `workflow_dispatch` inputs, Step Functions input validation and Argo parameters all check at the door. Reusing C12's subset keeps one schema language per workflow.
- **R7. What the menu shows (Q8, my default; updated after Q3 and the drift check).** It lists the unfinished runs first, then the workflows to start, then "write a new workflow". Each run's actions map one to one onto `spore` commands ([§Plugin side](#plugin-side-the-menu)). After a choice the menu closes and prints one line: what happened and where to watch it.
- **R8. Generic versus plugin (the drift check on Q7).** These are plain Runspore and usable with no Claude: the project folder, walk-up, `continue`, several runs, `cancel`, `async.stop` and `inputSchema`. These are plugin only: the session-start line, the menu, Claude's input draft, "open its pane" and `claude-spore stop`. Runspore's text never says pane, session, Claude or agent.
- **R9. "Start" checks before it starts (from C07 R10 and C14).** The menu runs `claude-spore check <folder>` before `spore start`, which validates C14's node parameters. A failing check shows its errors and starts nothing.

## Spec

Normative wording to carry into `spec/cli.md` 0.2, `spec/store.md` 0.2, `spec/host.md` 0.2, the package format `runspore.workflow/0.2`, and the profile spec. Names marked provisional are C10's.

### The project folder

```text
.spore/
├── .gitignore              # written by spore: runspore.db, runspore.db-*, runspore.db.files/
├── workflows/
│   └── coding-loop/        # a C02 workflow folder, committed
│       ├── workflow.dot
│       ├── settings.json
│       └── …               # instructions and assets it names
├── runspore.db             # not committed
└── runspore.db.files/      # C14 files_dir: steps/ and work/, not committed
```

- `spore init` creates `.spore/`, `.spore/workflows/` and `.spore/.gitignore` at the repository root: the nearest folder at or above the current one that holds `.git`, or the current folder when there is none. It refuses, with exit 2, in three cases:
  - a `.spore/` already exists at or above the current folder, at a different level from the root it would use;
  - a 0.1 database (`runspore.db`, its `-wal`/`-shm` files or `runspore.db.files/`) exists at the root; the error prints the exact moves into `.spore/`;
  - the target is otherwise unusable.

  It never overwrites a file, and running it twice changes nothing. The menu's "write a new workflow" (C09) runs it when no project folder exists.
- **The ignore file.** It must exclude `runspore.db`, `runspore.db-*` and `runspore.db.files/`. `init`, and any command that writes the database, add whichever of these lines are missing to `.spore/.gitignore`, keep every other line, and create the file if it is absent.
- **Only writing commands create a database.** These are `start`, `run`, `init`, `signal`, `resolve`, `cancel` and the `step` commands that write. Read-only commands (`list`, `status`, `events`, `step show`, and `continue` when there is nothing to continue) treat a missing database as empty and never create one.
- **Steps reach the same database.** The host sets `RUNSPORE_DB` to the absolute path of the database it runs on for every program it starts (command, async start, probe, watch, stop). So a `spore` call from inside a step, even in a git worktree with its own `.spore/`, reaches the run's own database, because `$RUNSPORE_DB` beats walk-up (R4). The plugin passes it on to the Claude session it starts.
- Database resolution follows R4. `spore` prints the path it chose in `status --json` and `list --json` as `"db"`.

### `spore continue`

`spore continue [<runId>]` drives one unfinished run.

| Unfinished runs (or the named one) | What it does | Exit |
|---|---|---|
| none | Prints "nothing to continue" and the workflows under `.spore/workflows/` (JSON: `{"runs": [], "workflows": [...]}`) | 0 |
| several, no `<runId>` | Lists them as `list --unfinished` does and asks for one by ID | 2 |
| one, or the named one | See the steps below | As `spore run`: 0, 1, 3, 4, 5, 6, or 7 if it was cancelled meanwhile |

For a single run, `continue`:
1. **Quarantined:** prints the reason and `spore release <runId>`, then exits 5. It works nothing.
2. **Held async step:** first expires the attempt if its deadline or lease has passed, as the worker's sweep would. Then it probes the attempt (C03). A certain `dead` expires the attempt with `probe-dead`. Either way the run goes on. Otherwise it prints the actor label, the holder handle, the deadline and `spore step abandon <runId>`, then exits 6.
3. **Otherwise:** works the run exactly as `spore run` resumes it, until it is terminal or parked.
4. **Then it prints what is needed:**
   - a signal: the node, `spore step show`, and the `spore signal` line with the node's allowed outcomes;
   - a stuck step (C07 L3): the kept error and the three `spore resolve` forms;
   - a ready external step: `spore step claim <runId>`.

It works the run in the foreground. Whether the mod calls it in the background, and who keeps the run moving between a person's actions, is C06.

### `spore list`

- `--unfinished` filters to unfinished runs.
- Each row gains `workflow` (the package name), `nodeId`, `needs`, `since` (when the run entered its current state, epoch ms as a decimal string) and `actor` (for a held or ready external step).
- `needs` is one of `working`, `signal`, `stuck`, `external-ready`, `held`, `quarantined`.
- Rows are ordered newest first.

### `spore cancel`

`spore cancel <runId> [--reason <text>] [--id <requestId>]` appends a cancel event. The request ID is derived from `--id`, which is generated if missing, so a repeat is a duplicate. Exit 0 once the run is `cancelled`, or when it had already ended.

- **Store 0.2: `cancel_run(key, reason, requestId)`.** One atomic operation, idempotent by request ID:
  - it records `cancelled {reason, at}` on the run, and clears a quarantine if there is one;
  - it expires a held async attempt with reason `cancelled`.

  After it, the run is invisible to `load_turn`, `scan_ready` and `scan_due`; `commit_turn` and `claim_attempt` refuse with `stale` / `run.cancelled`; `heartbeat_attempt` and `finish_attempt` answer `lease.lost`. Read views show status `cancelled`. On a run that has already ended, it changes nothing.
  - New conformance rows: cancel at a waiting signal, at a stuck step, while quarantined, during a running command (its next heartbeat is refused), during a held async step, from two processes at once, and after completion.
- **Kernel.** Unchanged. The snapshot keeps the state it had, and `cancelled` lives beside it in the store, as a quarantine does.
- **Host 0.2.** A worker whose heartbeat answers `lease.lost` already stops its runner and stores late evidence (host §2), so a running command stops within one `heartbeat_ms`. After the store operation, `spore cancel` runs the action's `async.stop` for an expired held attempt, if declared (tagged like `probe`: `{"command": {"argv": [...]}}`). It runs once, with the holder handle on stdin and the C14 environment; its output is recorded as late evidence and its failure is ignored.
- **CLI 0.2.** New exit code 7, "the run was cancelled", for `run`, `worker --until-parked` and `continue`.

### `inputSchema`

```json
{
  "inputSchema": {
    "type": "object",
    "required": ["task"],
    "properties": {
      "task": {"type": "string", "description": "What should this run build or fix?"}
    }
  }
}
```

- It is a top-level field of `settings.json`, copied into the package (`runspore.workflow/0.2`) and covered by its digest.
- It uses C12's schema subset, with one restriction: its root is `"type": "object"`, so a run's input always has named fields.
- `validate` rejects a malformed schema with `workflow.invalid-schema`.
- `start` and `run` check `--input` / `--input-file` against it before `create_run`. On a misfit they return `input.invalid` with the failing path and exit 2.
- With no `inputSchema`, any input is accepted, as in 0.1.

### Plugin side: the menu

Command name `/spore` (provisional, C10). The widget that carries the menu is C05's finding: `$.ui.ask`, a pane, or command output.

**Session start.**
1. The mod's `session.start` hook runs `spore list --unfinished --json` within the 10 s hook budget.
2. If any runs are listed, it prints the one-line notice of R1.
3. On error it prints nothing; the hook fails open.

**The menu, top to bottom.**
1. **Unfinished runs**, newest first. Each row shows the workflow, its node, how long it has waited, and what it needs. Each row offers these actions:

   | `needs` | Action | Runs |
   |---|---|---|
   | `signal` | Decide: shows the step's instructions, then asks for an outcome (and data if the node's schema asks for it) | `spore step show <runId>`, then `spore signal <runId> <name> --outcome <o> [--data …]` |
   | `stuck` | Retry / mark done / fail | `spore resolve <runId> <inv> --retry` / `--complete <o> [--output …]` / `--fail <message>` |
   | `held`, holder handle has the plugin's interactive form | Open its pane | herdr focus on the handle's agent name. The form is C07 Q20, still deferred; until it is settled this row falls back to the next one |
   | `held`, other holder | Shows the holder and deadline | `spore step abandon <runId>` on request |
   | `external-ready` | Shows the step | `spore step show <runId>`. A person claims it with `spore step claim` |
   | `working` | Continue | `spore continue <runId>` (foreground or background per C06) |
   | `quarantined` | Shows the reason, offers release | `spore release <runId>` |
   | any | Cancel | `spore cancel <runId>` |

2. **Workflows to start**: each folder under `.spore/workflows/`, shown by name, always offered (L2). Choosing one:
   1. Runs `claude-spore check <folder>` (R9).
   2. Reads `inputSchema` from the folder's `settings.json`.
   3. For each top-level property, it shows the property's `description`, or the property's name when there is no description, together with Claude's draft from the conversation. The person confirms or edits it (R5). Properties that are not required can be left out. A property whose type is not `string` is entered as JSON. `spore start` checks the whole input either way.
   4. Runs `spore start <folder> --input-file <tmp>`.
   5. Prints the new run ID and where to watch it.
3. **Write a new workflow**: hands off to C09's interview, which runs `spore init` when needed.

## Verified facts

Established by reading the repo and the research notes, not by asking:

- CLI 0.1: the database is `--db`, else `$RUNSPORE_DB`, else `./runspore.db`. `run` on an existing key resumes the run. Exit codes go up to 5, plus 10 and 130. C03 adds 6 (spec/cli.md).
- ADR M12, "no cancellation in 0.1" (cited in C02 Verified facts).
- C03: probes run on demand from `status`, `continue` and `step claim`, and `step abandon` frees a held step by hand (c03 R5, §2).
- C14: `files_dir` defaults to `<db path>.files`. So under L1 it is `.spore/runspore.db.files/`.
- Mods (docs/research/claude-code-mods.md §4):
  - `session.start` fires once per loaded mod before the first prompt, not after `/clear` or `/resume`.
  - A hook's own run time is capped at 10 s.
  - `$.process.run` defaults to 30 s.
  - `$.command.register` with `immediate: true` runs a command with no Claude turn.
- C07 Q20 defers the interactive step's holder handle. Its proposal carries the herdr agent name, which herdr clears on exit or manual release. So "open its pane" depends on Q20, and on the name still being live.

## Changes outside C08

- **Map #5, "Locked during charting".** "start a run of a workflow when none is running" becomes "start a run of a workflow" (L2).
- **ADR M12** (docs/02): cancellation added in 0.2 as a store-level close (L4). The kernel stays unchanged.
- **C07 Q20.** The interactive holder handle needs a form the menu can recognise, for "open its pane".
- **C03.** `continue`, named there as C08's, is now specified. `async` gains optional `stop`, beside `probe` and `watch`.
- **C06.** `continue` works a run in the foreground. Whether the mod runs it in the background, and how it learns the run moved, stays with C06.
- **C09.** A new workflow is written to `.spore/workflows/<name>/` after `spore init`. The interview should ask for the run's inputs and write `inputSchema`.
- **C10.** Provisional names: `/spore`, `claude-spore stop`, `spore init`, exit code 7.
- **C11.** The reference loop declares `inputSchema` with a `task` string. Whether it isolates itself in a worktree (L3) is its author's choice, to make there.
- **CONTEXT.md.** New terms: Project folder, Unfinished run, Entry menu, Cancel, Input schema.

## Risks

- **Two runs in one checkout can overwrite each other's files, silently** (L3, Max's call). The workflow author has to choose isolation. A reference loop without a worktree step is exposed when two runs run at once.
- **Deleting a checkout deletes its run history** (L1). This is intended: runs belong to the checkout.
- **Walk-up can find an unexpected `.spore/`**, for example a repository nested inside another that has one. `status --json` prints the chosen `db`, and `--db` overrides it.
- **A step's spore calls follow `$RUNSPORE_DB`.** A program that clears its environment, or a person typing in an interactive step's pane, falls back to walk-up and may reach a worktree's database.
- **Cancel's stop is best effort.** If herdr is unreachable, `claude-spore stop` fails and an interactive session may keep editing files after its run is cancelled. Its later result is only late evidence, but the edits stay.
- **The session-start line depends on `spore list` answering within the hook budget.** A locked or very large database could exceed it, and the line is then skipped.

## Deferred

None.

## Open threads

- **The menu's widget** (`$.ui.ask`, `ui.select`, a pane, or command output) is C05's measurement.
- **Command names** (`/spore`, `claude-spore stop`) are C10's.
- **Whether `spore start` takes a bare workflow name** (resolving to `.spore/workflows/<name>/`) was not raised. Today it takes a path, and the menu passes the path.
