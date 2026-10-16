# CLI 0.2

One binary, `spore`, the Runspore command line. It embeds the kernel component, the SQLite store, and the
engine. It never listens on a network.

Version 0.2 replaces 0.1 outright. `spore` 0.2 reads only workflow folders of dialect
`runspore/0.2` and the artifacts built from them, and only databases written by 0.2
(§Databases from another version); it tells you what to do with anything older. Rules
marked **0.2** below are new since 0.1.

## Global options

| Option | Default | Meaning |
| --- | --- | --- |
| `--db <path>` | `$RUNSPORE_DB`, else an existing `.spore/runspore.db` of the project folder, else the repository's shared `$SPORE_HOME/db/<repo key>/runspore.db` (§Project folder) | Database file |
| `--json` | off | One JSON object on stdout instead of text |
| `--native-kernel` | off | Call the kernel natively instead of through Wasmtime; for debugging |
| `--lease-ms`, `--heartbeat-ms`, `--poll-ms`, `--grace-ms` | engine defaults | Worker tuning; mainly for tests |
| `--stop-grace-ms` | 5000 | Time a stopped command's process group gets between SIGTERM and SIGKILL (host.md §3, `stop_grace_ms`); used by `worker`, `run` and `continue` |
| `--max-watches` | 16 | **0.2.** Watches of held async steps one worker runs at once, apart from its slots (host.md §3, Async commands); used by `worker`, `run` and `continue` |

## Commands

| Command | Does |
| --- | --- |
| `init` | **0.2.** Creates the project folder (§Project folder) |
| `validate <workflow> [--short] [--strict] [--fix]` | Builds the workflow and checks it and its actions without touching the database. **0.2:** reports every problem of a folder at once (§Validation) |
| `explain [<code>]` | **0.2.** Prints the help page of a validation code, or lists every code (§Validation) |
| `build <folder> [-o <file>] [--composed] [--strict]` | **0.2.** Builds a workflow folder (or its `workflow.dot`), checks it as `validate` does, and writes the workflow artifact to `<file>`, by default `<folder>.wasm` beside the folder. With `--composed`, the composed artifact (artifact.md §7), by default `<folder>-composed.wasm` |
| `inspect <artifact>` | **0.2.** Loads an artifact and prints its digest, manifest and package as JSON |
| `inspect --run <runId> --file <path>` | **0.2.** Prints the file the run's artifact holds at `<path>`, for example `workflow.dot` |
| `start <workflow> [--input <json> \| --input-file <path>] [--key <startKey>] [--base-dir <dir>]` | Creates a run and returns its run ID. A missing key is generated; the same key always names the same run in this database. `--base-dir` (0.2) overrides the run's base folder. The input is checked against the workflow's `inputSchema` (0.2) |
| `run <workflow> [--input …] [--key …] [--base-dir …]` | `start`, then works until the run is terminal or parked |
| `worker [--run <runId>] [--until-parked]` | Works on all runs until interrupted. With `--run --until-parked`, on one run until it is terminal or parked |
| `status <runId> [--no-probe]` | Status, position and the arrow into it, result and the end node, quarantine reason, outstanding invocation. **0.2:** what a waiting, stuck or running run waits for, since when, and the commands that move it on; the run's parent and the runs below it (§Runs that start runs). It first probes a held async step; `--no-probe` reads the store only (§Steps outside the worker) |
| `list [--unfinished]` | All runs with status. **0.2:** as a tree, parents first, each unfinished run with its node and how long it has been there (§Runs that start runs); each with its workflow and what it needs. `--unfinished`: only the unfinished runs, newest first (§What a run needs) |
| `continue [<runId>] [--wait [--exit-status]]` | **0.2.** Does the next thing a run needs, then prints exactly what a person must type. With `--wait`, follows the run past held async steps until it needs a person or has ended; with `--json` it then exits 0, or with the run's code under `--exit-status` (§What a run needs) |
| `events <runId>` | The run's events in order, with the revision that consumed each and its diagnostics. **0.2:** in text, each with its time, its node, its outcome, the arrow the run took and the first line of its error or output (§JSON output) |
| `signal <runId> <name> [--outcome <o>] [--data <json>] [--id <messageId>]` | Delivers a signal. A missing ID is generated; repeating an ID is a no-op |
| `resolve <runId> [<invocationId>] (--complete <outcome> [--output <json>] \| --retry \| --fail <message>) [--id <requestId>]` | Answers a stuck step, or an invocation whose outcome is unknown. Without an invocation ID it answers the run's outstanding invocation; a run with none exits 2 (`run.no-invocation`) |
| `release <runId>` | Lifts a quarantine |
| `cancel <runId> [--reason <text>] [--id <requestId>]` | **0.2.** Ends an unfinished run as `cancelled`, stops its running command and runs its held step's stop (§Cancelling a run) |
| `step claim [<runId>] [--actor <label>] [--holder <json>]` | **0.2.** Claims a ready external step and prints what the actor needs (§Steps outside the worker) |
| `step renew <token>` | **0.2.** Extends the claimed attempt's lease, never past its deadline |
| `step complete <token> --outcome <o> [--output <json>]` | **0.2.** Reports the step done with an outcome |
| `step fail <token> --error <message> [--code <c>] [--retryable]` | **0.2.** Reports the step failed |
| `step abandon <runId>` | **0.2.** Frees the run's held async step at once |
| `step show [<runId>]` | **0.2.** Describes the step the run is at, without claiming, probing or changing anything (§What a run needs) |
| `capabilities` | **0.2.** Prints what this `spore` runs: its version, the version of each contract, and its capability set (§Capabilities) |

A `<workflow>` is a workflow folder or its `workflow.dot`, which is built in memory
as [dot.md](dot.md) describes, or a workflow artifact (a file starting with the
WebAssembly magic bytes). Nothing else is read: any other file, such as a JSON
workflow document of 0.1, exits 2 with `workflow.folder` and a message saying that a
workflow is a folder holding `workflow.dot` and `settings.json`, or an artifact built
from one. A folder of dialect `runspore/0.1`, or whose `settings.json` says
`runspore.workflow/0.1`, is a build problem that names the fix (dot.md §3). A folder
with problems exits 2 with every one of them (dot.md §6, §Validation below); an
artifact that does not load exits 2 with its code (artifact.md §6). JSON workflow
documents remain an internal form: the package inside an artifact, and a test fixture.

`start` and `run` from a folder build its artifact in memory, load it like a
prebuilt one, and store it with the run; from an artifact they store that file. Either
way the run never reads the folder or the file again: editing or deleting them
changes nothing a started run sees. Every run `spore` starts has an artifact.

`build` writes `<folder>.wasm` next to the folder, named after the folder (so
`spore build deploy/` writes `deploy.wasm`), never inside it; `-o` names another file.
It overwrites an existing file. The same folder built by the same `spore` gives the
same bytes. Its JSON output is `{"artifact", "artifactDigest", "packageDigest",
"bytes"}`: the file written, its digest, the package digest and the file's size.

`build --composed` writes the composed artifact (artifact.md §7) instead:
`<folder>-composed.wasm` beside the folder, or the file `-o` names. The kernel inside
is the one this `spore` embeds, so the output carries `"composed": true` and
`"kernelDigest"`, the `kernel_digest()` of that kernel; `artifactDigest` is the digest
of the composed file. A composed artifact is a `<workflow>` like any other artifact:
`validate`, `start`, `run` and `inspect` read its `reader` interface and run the
package with the embedded kernel of the `spore` at hand, never the one in the file. A
run started from a composed artifact records that file's digest, and gives the same
decisions as a run from the plain artifact of the same folder.

`inspect` is for debugging; its output is not a stable interface. `inspect --run`
reads the run's stored artifact, never the folder: it prints any entry's raw bytes to
stdout, and with `--json` an object `{runId, path, kind, digest, text}`, where `text`
is the file as a string, or null when it is not UTF-8. A path the artifact does not
hold, or a run started without an artifact, exits 2 (`artifact.no-file`,
`artifact.none`). `inspect` needs either an artifact or `--run` with `--file`.

**0.2.** A `<workflow>` may also be a bare workflow name: an argument that is not an
existing path and contains no `/`. It names the folder under `.spore/workflows/` of the
project folder found by walking up from the current folder (§Project folder) whose
`workflow.dot` has that graph id. No project folder, or no folder with that graph id,
is `workflow.not-found`; two folders with it are `workflow.ambiguous`. Both exit 2,
and `workflow.not-found` lists the names that exist.

`run` and `worker --until-parked` are the same loop; `run` on an existing key resumes it.
**0.2:** the loop works its one run only, with the runs below it in `list`'s tree
(§Runs that start runs; host.md §2, `run_until_parked`); other runs of the database
wait for `spore worker` or their own `run`, `continue` or `worker --run`.
The same key with another input or workflow is refused with the ways on (§Keys used
again).
While they work, `run` and `worker` print the changes of the runs they work on stderr
(§Progress).
Run IDs are random (host §2): `start` and `run` print the run ID the engine returned,
and the same key in another database names another run.
`worker --run` and `--until-parked` are only valid together. `release` on a run that is
not quarantined succeeds and changes nothing.

A step of a 0.2 workflow that gives up with no `failed` route is stuck (kernel.md
§7.4): the run stops with exit 4 and waits for a person. `resolve --retry` runs a new
try of the same invocation, with the same effect key, even past the action's
`maxAttempts`. `--complete <outcome>` continues as if the step had returned that
outcome. `--fail <message>` ends the run as failed with the step's kept error; the
message is recorded in the event only, and the run never pauses on that step again.

**0.2: telling someone.** A workflow whose settings name an `onIntervention` action
(dot.md §4, host.md §2, Telling a person) has it run each time a run stops for a
person (exit 4) or fails (exit 1). `run` and `worker --until-parked` wait for it before
they exit. What it returns never changes the run; when it fails, `events` shows an
`on-intervention.failed` event with its error, and nothing else happens.

**0.2: output schemas.** `signal --data` and `resolve --complete --output` are
checked against the schema of their outcome ([schema.md](schema.md), host.md §2);
missing data is `null`. Data that does not fit exits 2 with code `output.invalid` and
records nothing, so the same command can be sent again with corrected data. Text mode
prints the refusal and then one error per line, path first, `(root)` for the whole
value:

```text
spore: output.invalid: the data for outcome "rejected" does not fit its schema
(root): expected type "object", got null
```

In JSON mode the error object carries `details`:
`{"outcome": "rejected", "errors": [{"path": "", "message": "expected type \"object\", got null"}]}`.
A folder's schema outside the subset is the diagnostic `schema.outside-subset` (dot.md
§6).

**0.2: unrouted outcomes.** While the run waits at an await-signal node for the
signal, `signal` with an outcome the node does not route (`received` when `--outcome`
is missing) exits 2 with code `signal.outcome-unrouted` and records nothing (host.md
§2). Text mode prints the refusal and then the outcomes the node accepts, the same ones
`status` lists:

```text
spore: signal.outcome-unrouted: the run waits at review for signal approval, which does not route outcome "approve"
accepted outcomes: approved, rejected
```

In JSON mode the error carries `details`: `{"nodeId": "review", "signal": "approval",
"outcome": "approve", "outcomes": ["approved", "rejected"]}`. A signal sent while the
run does not wait for it is recorded as before; if the node it reaches does not route
its outcome, `events` shows it dropped with the diagnostic `signal.outcome-unrouted`.

## Steps outside the worker (0.2)

An external step (host.md §2, Steps outside the worker) is done by an actor, who
claims it, works, and reports back with `spore step`. `run` and `worker
--until-parked` stop when the run reaches one, with exit 6: the worker has nothing to
do until an actor reports or the attempt expires. A worker still sweeps the step's
attempt: once its lease or deadline passes, it is expired and the action's effect
decides (a repeatable step is ready again, an `unsafe` one needs intervention).

**0.2: async commands.** A `command` with an `async` object (host.md §3, Async
commands) runs a start command that starts the work elsewhere, prints a holder handle
and exits; the attempt is then held outside the worker, like a claimed external step.
The start command gets the attempt's token as `RUNSPORE_TOKEN`, so the work reports
with `spore step complete`, `fail` and `renew` like an actor. `run` and `worker
--until-parked` stop there with exit 6, unless the action declares a watch: then they
run it and go on when it reports, and stop with exit 6 when it prints that its holder
waits for a person, or when it ends without a result. `worker` runs at most
`--max-watches` watches at once.

An **async step** is an external step or a held async command. Its action's probe
(host.md §3) runs on demand: `status` probes the run's held async step before it reads
the run, `step claim <runId>` probes a held step of that run before it claims, and a
worker's sweep probes each held async step at most every 30 s. Only a `dead` answer
acts: it expires the attempt at once, and the action's effect decides. `status
--no-probe` runs no probe and changes nothing, for a display that refreshes often.

**`step claim [<runId>] [--actor <label>] [--holder <json>]`.** With `<runId>`, claims
that run's ready external step; a run that has none exits 2 with `step.not-ready`.
Without it, claims the oldest ready matching step in the database, like a task queue;
when there is none it exits 0 and prints `{"claimed": false}` (text: `nothing to
claim`). Without `--actor` it takes only steps whose action has no `actor`; with
`--actor <label>`, only steps with that label. `--holder` is the holder handle,
canonical JSON of at most 4 KiB kept with the attempt and shown by `status` (exit 2,
`holder.invalid`, otherwise). It prints:

```json
{"format": "runspore.cli/0.2", "claimed": true, "token": "…",
 "runId": "run_…", "nodeId": "review", "actionId": "review", "invocationId": "inv_…",
 "attempt": 1, "input": {…}, "params": {…}, "outcomes": ["approved", "changes"],
 "outputSchemas": {"changes": {…}}, "deadlineMs": "…", "leaseUntilMs": "…",
 "resultChannel": "store", "files": "/…/steps/…/review", "instructions": "/…/review.md",
 "workDir": "/…/work/run_…/inv_…"}
```

`resultChannel` is the result channel the run recorded for the action at `start`
(§Capabilities).

`params` is the node's evaluated `params`, `{}` when it has none (kernel.md §2).
`files`, `instructions` and `workDir` are the step's files, its instructions file and
its work folder (host.md §3), each null when it has none. The token is the claim's
credential; treat it as opaque. Text mode prints the step, its outcomes, its deadline,
the token and the commands that report with it, and then the instructions themselves,
so a person at a terminal can start at once.

**`step renew <token>`.** Extends the lease by the action's `leaseMs` (or `timeoutMs`),
never past the deadline, and prints `{runId, leaseUntilMs, deadlineMs}`.

**`step complete <token> --outcome <o> [--output <json>]`.** Reports success.
Missing output is `null`. It exits 2 and records nothing when the outcome is not one
the action declares (`step.undeclared-outcome`), the output is not canonical JSON
(integers only, depth at most 32; `step.output-not-canonical`), the result is over
64 KiB (`step.output-too-large`), or the output does not fit the outcome's schema
(`output.invalid`, printed as for `signal`). The same report sent again is a
`duplicate`.

**`step fail <token> --error <message> [--code <c>] [--retryable]`.** Reports failure
with code `step.failed`, or `<c>`, which must match `[a-z0-9][a-z0-9.-]{0,63}` (exit 2,
`step.bad-code`). With `--retryable` the kernel may try again, up to the action's
`maxAttempts`; without it, or once they are used up, the run takes the node's
`failed` arrow or the step is stuck.

`complete` and `fail` print `{runId, invocationId, attempt, disposition}`. A report
with a token that names no attempt of the run's external step exits 2 with
`step.token-invalid`. A report for an attempt that has already ended, because it was
abandoned, expired or reported, exits 2 with `lease.lost`; spore keeps what it said
as late evidence, and the run does not move.

A report whose attempt a watch has already finished is refused the same way: the
first result from any channel wins.

**`step abandon <runId>`.** Expires the run's held async attempt at once, whatever
its lease, and fences its holder out: a later report from it is `lease.lost`. The
action's effect then decides, as for any expiry. A run with no held async step
exits 2 with `step.not-held`. It prints `{runId, invocationId, attempt, disposition}`.

The `step` commands but `step show` are writing commands (§Project folder).

## Capabilities (0.2)

`spore capabilities` says what this `spore` runs, so a tool that drives it can check
before it starts anything. It reads no database and needs no project folder:

```json
{"format": "runspore.cli/0.2", "version": "0.2.0",
 "contracts": {"artifact": "0.1", "cli": "0.2", "dot": "0.2", "host": "0.2",
               "kernel": "0.2", "schema": "0.2", "store": "0.2"},
 "actionKinds": ["command", "external", "native"],
 "asyncKinds": ["command", "external"], "resultChannels": ["store"]}
```

- `version`: the version of the `spore` binary, the one `spore --version` prints.
- `contracts`: the version of each specification it implements: the kernel
  ([kernel.md](kernel.md)), the host ([host.md](host.md)), the store
  ([store.md](store.md)), this command line, the DOT dialect ([dot.md](dot.md)), the
  workflow artifact ([artifact.md](artifact.md)) and the schema subset
  ([schema.md](schema.md)). A contract gains a key when it is added; one that changes
  gets a new version.
- `actionKinds`, `asyncKinds`, `resultChannels`: the host's capability set
  (host.md §2, The handshake): the action kinds it runs, those it runs async, and the
  result channels an async action can report through.

Text mode prints one line per field, such as `async kinds     command, external`.

`validate`, `build`, `start` and `run` check every action against the set and refuse
a workflow that asks for something outside it, naming the action and what to
configure (`action.component-unsupported`, `action.no-result-channel`; dot.md §6).
`start` of an artifact also refuses an action of a kind this `spore` does not run or
does not run async (`action.kind-unregistered`, `action.async-unsupported`; host.md
§2). `start` records the result channel it picked for each async action with the run,
and `step claim` returns it as `resultChannel`.
When a later `spore` cannot act on a step the way the run recorded, it leaves the step
untouched: `status` shows the step as `capability-missing` with what to restore, and
`run` and `worker --until-parked` stop there with exit 6.

## Runs that start runs (0.2)

A step may start other runs, as a build starts one run per ticket. A run started with
`start` or `run` from inside a step records that step as its **parent**: the run and
node in `RUNSPORE_RUN_ID` and `RUNSPORE_NODE_ID`, which the host gives every command
(host.md §3). No flag is needed, and outside a step, where the two are not both set,
a run has no parent. A `start` whose key names an existing run keeps the parent
recorded first (store.md §2 `create_run`). Because a step's `spore` reaches the run's
own database (§Project folder), parent and child share one.

Every step of a child run gets the parent's run ID as `RUNSPORE_PARENT_RUN_ID`
(host.md §3), so a child's last step tells its parent it is done with no input naming
the parent:

```sh
spore signal "$RUNSPORE_PARENT_RUN_ID" ticket-done --outcome ok
```

`list` prints the runs as a tree: each run, then the runs it started, indented below
it, and theirs below them; siblings in the order they were created. A run whose parent
is not in the database is at the top. Each line shows the run, its status, its node
and how long it has been there, its start key, and for a child the node of the parent
that started it:

```text
run_4f…  waiting             hold              2m10s  build
  run_9a…  running           implement         1m58s  FC-146  from tickets
  run_c2…  completed         -                     -  FC-147  from tickets
```

The node is where an unfinished run is, and the time is since the run came to stand
there as it stands: the `sinceMs` that `status` reports in `waiting`, `intervention`
or `running`, found by the same replay of the run's events (§JSON output). A finished run, and a run that has not
taken its first turn, is at no node, shown `-`. `list` therefore costs one transition
per event of each unfinished run. A quarantined run shows `quarantined` as its status,
and at the end of its line its quarantine's code, the first line of its details and
`spore release <runId>`; JSON keeps the kernel status and sets `quarantined`.

In JSON, each entry of `runs` is `{"runId", "startKey", "status", "revision",
"quarantined", "parent", "depth", "nodeId", "sinceMs"}`, in the tree's order. `parent`
is `{"runId", "nodeId"}` or null; `depth` is 0 at the top and one more per level;
`nodeId` and `sinceMs` are null at no node. Each entry also carries `workflow`,
`needs`, `since` and `actor` (§What a run needs). `status` carries the run's `parent` and,
as `children`, the runs below it in the same form and order, its direct children at
depth 0; its text lists them under `children`.

## What a run needs (0.2)

An **unfinished run** is one that has not completed, failed or been cancelled
(§Cancelling a run). What it needs from someone is one of:

| `needs` | When |
| --- | --- |
| `quarantined` | The run is quarantined, whatever else holds |
| `working` | A worker has something to do: the run has not taken its first turn, it has an event not yet applied (a signal already sent to the node it waits at, for one), or it is `running` at a step a worker runs |
| `signal` | It waits at an await-signal node, with no signal for it stored |
| `child` | It waits as for `signal`, while a run it started has not ended: that run, or one below it, sends the signal, as a build's ticket run signals the build when it is done (§Runs that start runs). Nobody decides it; the runs below it say what they need |
| `stuck` | It needs intervention: a stuck step, or an unsafe step whose outcome is unknown |
| `external-ready` | It is at an external step nobody holds (§Steps outside the worker) |
| `held` | It is at an async step that is held: an external step an actor holds, or an async command whose started work has not reported |
| `capability-missing` | It is at a step this `spore` cannot act on the way the run recorded (§Capabilities) |

**`list --unfinished`.** Lists only the unfinished runs, newest first (by the time each
was created), one line each and not as a tree: the run, its workflow, its node, what
it needs, how long ago it last changed, its start key, the actor of an external
step, and a quarantine's code, details and release as in `list`:

```text
run_9a…  coding-loop  review     signal              2m10s  ticket-148
run_4f…  build        tickets    held                  12s  build  actor reviewer
run_7c…  coding-loop  fix        quarantined         1h09m  ticket-73  host.files-unwritable: No space left on device  spore release run_7c…
```

Each row of `list`, with or without `--unfinished`, carries beside the fields of §Runs
that start runs: `workflow`, the name of the run's workflow (its graph id, kernel.md
§2); `needs`, as above, null for a finished run; `since`, when the run last changed,
the run's `updatedAtMs` (store.md §2), so a claim or an expiry counts as a change; and
`actor`, the label of the external step it is at (`external-ready` or `held`), else
null. With `--unfinished`, `depth` is 0.

**`continue [<runId>]`.** Drives one unfinished run: it does the next thing the run
needs, or says exactly what a person must do. Without `<runId>`:

| Unfinished runs | What it does | Exit |
| --- | --- | --- |
| none | Prints `nothing to continue` and the workflows of the project folder (`.spore/workflows/`, found by walking up), so a person sees what can be started. JSON: `{"runs": [], "workflows": [{"name", "folder"}]}` | 0 |
| several | Changes nothing. Exits with `run.several`: the runs as `list --unfinished` lists them, and how to name one. JSON details: `{"db", "runs"}`, the rows of `list --unfinished` | 2 |
| one | Continues that run, as below | as below |

With `<runId>`, or the one unfinished run, it:

1. **Quarantined:** works nothing. It prints the quarantine's code and details and
   `spore release <runId>`, and exits 5.
2. **At a step another worker runs** (its attempt takes that worker's slot, as
   `status` shows it in `running` and `slots`): works nothing, and says which worker
   holds the step instead of waiting on it in silence, as `temporal workflow describe`
   names the worker on each pending activity. It prints `fix attempt 1 is running on
   worker <id> (pid <pid>, for 7m35s); waiting for it to finish`, leaving out the pid
   or the time when the store does not know them, then `spore continue <runId>
   --wait`, and exits 0. JSON: the `status` object with `needs` `working` and that
   command in `commands`. A worker known to be gone (it ran on this machine and its
   process no longer runs) runs nothing: its try waits for its lease to run out, as
   §Coming back after a kill says, and `continue` works the run.
3. Otherwise **works the run** exactly as `run` resumes it, until it is finished or
   parked, printing its progress on stderr (§Progress). Like `run`, it works that
   run only, with the runs below it in `list`'s tree: the runs it started, and theirs,
   as Temporal's worker runs a workflow's child workflows. A parent that waits for its
   child's signal is not parked while the child can be worked; it returns when each
   run of the tree is finished or parked, or the run itself has ended. It takes no
   turn of another run and claims none of its steps, so exiting when the run parks
   never cuts off another run's try or spends one it never started (host.md §2,
   `run_until_parked`). `spore worker` works the other runs. On the way it sweeps attempts
   as a worker does: a held async step whose lease or deadline has passed is
   expired, and its action's effect decides. A held step is probed when its action has
   a probe this host runs (host.md §2); a `dead` answer expires it with reason
   `probe-dead`, and the run goes on.
4. **Prints what is needed**, and exits as `run` does:

   | The run | It prints | Exit |
   | --- | --- | --- |
   | completed | the end node | 0 |
   | failed | the error | 1 |
   | cancelled | the reason, when one was given | 7 |
   | waiting for a signal | the node and the signal, `spore step show <runId>`, and one `spore signal` per outcome the node routes, as `status` gives them | 3 |
   | waiting on a run it started (`child`) | the node, the signal and the unfinished runs it started; then, for each unfinished run below it that is not itself waiting on a child, in `list`'s order, what that run needs as this table gives it, its commands among `commands`. It offers no `spore signal` to the run itself, as `temporal workflow describe` lists pending children instead of asking for their result | 3 |
   | stuck, or an unknown outcome | the step and its kept error, and the three `spore resolve` forms, as `status` gives them | 4 |
   | quarantined while it worked | as in 1 | 5 |
   | at a ready external step | the step and its actor label, and `spore step claim <runId>`, with `--actor <label>` when it has one | 6 |
   | at a held async step | the step, the actor label (for an async command, the work it started), the holder handle, the deadline, the message when its watch said that the holder waits for a person, and `spore step abandon <runId>` | 6 |
   | at a `capability-missing` step | the step and what this `spore` lacks and what to restore, as `status` gives it in `step.missing`; no command | 6 |

A run that is finished already is shown as above, and nothing is worked. The JSON is
the `status` object `run` prints, with `needs` (null when finished) and `commands`, the
commands of the table in the order given; `commands` is empty for a finished run. The
commands name the run and nothing else, as in `status`; for a run waiting on a child,
they name the runs below it. `continue` takes `run`'s
interrupts: SIGINT, SIGTERM or SIGHUP stop it as they stop `run`, with exit 130.

**`continue [<runId>] --wait`.** Follows the run until it needs a person or has
ended, so a driver can start it in the background and act when it returns. It chooses
the run, and treats a quarantined or finished run, as `continue` does. At a step
another worker runs it prints the line `continue` prints, as a progress line on stderr
(§Progress, change `running-elsewhere`), and works the run, which waits for that
worker's result. It works the run as `continue` does, except at a held async step,
where `continue` would stop with exit 6 (host.md §2, `follow`):

- **The step's action declares a watch.** It runs the watch, as `run` does, and goes
  on when the watch reports. When the attempt stops being held first (an actor's
  `spore step complete` won, the lease or deadline passed), it stops the watch with
  its process group and goes on from the run's new state. A watch that ends without a
  result leaves the attempt held, and the step is waited on as below until the next
  sweep starts a watch again.
- **The watch says that its holder waits for a person.** It stops the watch with its
  process group, leaves the attempt held, and returns exit 6 with `needs` `held` and
  the step's `waiting` `person` and `message` (§JSON output). The next `continue
  --wait` starts the watch again.
- **The step has no watch, or no watch runs.** It waits. It reloads the run every
  `--poll-ms` (200 ms by default), so a report from elsewhere, such as `spore step
  complete`, is applied within one poll. Its sweep expires the attempt at its lease
  or deadline, after which the action's effect decides, and probes the attempt at
  most every 30 s; a `dead` answer expires it at once.

It works the runs below the run as `continue` does, each followed as the run is, and
does not return while a run it waits on can still be worked.
It returns, printing what is needed as `continue` does, only when the run has
completed (exit 0), failed (1), waits for a signal (3), needs intervention (4), is
quarantined (5), or is at a ready external step, at a held step whose holder waits for
a person, or at a `capability-missing` step (6), or was cancelled (7). `--json` prints the same object as
`continue --json`, and exits 0 whenever it printed the run's status: a follower reads
`needs` (null once the run has ended) and `status`, and a run that only waits for a
decision is not reported as a failed command. `--exit-status` keeps the codes above
for scripts, as `gh run watch --exit-status` does; without `--json` they always
apply. `--exit-status` without `--wait` exits 2 (`usage`). Errors keep their codes
(2, 10). SIGINT, SIGTERM or SIGHUP stop it as they stop `run`, with exit
130, with or without `--json`: it stops claiming, gives in-flight work its grace period, and stops a running
watch with its process group. The held attempt stays held for the next driver.

**`step show [<runId>]`.** Prints the **step description** of the step the run is at,
without claiming it, probing it or recording anything, so a person can read a
decision's instructions before answering it. Without `<runId>` it takes the one
unfinished run; with none it exits 2 with `run.none-unfinished`, and with several it
exits 2 with `run.several`, as `continue` does. JSON:

```json
{"format": "runspore.cli/0.2", "runId": "run_…", "nodeId": "review",
 "nodeKind": "await-signal", "activationId": "review/1", "actionId": null,
 "signal": "approval", "outcomes": ["approved", "rejected"],
 "outputSchemas": {"rejected": {…}}, "files": "/…/steps/…/review",
 "instructions": "/…/steps/…/review/instructions/review.md",
 "from": {"nodeId": "test", "outcome": "ok"}}
```

`nodeKind` is `activity` or `await-signal`. An activity has `actionId` and the
action's `outcomes`; an await-signal node has `signal` and the outcomes it routes but
`exhausted`, the same ones `status` lists. `outputSchemas` is the node's, `{}` without
them. `files` and `instructions` are the step's files folder and its instructions
file (host.md §3), null when it has none; the files are checked, or made again, as
before a dispatch. `from` is the arrow into the node, as `status` gives it in
`position.from`. Text mode prints the same, and then the instructions themselves. A
run that is finished, or not at an activity or await-signal node, exits 2 with
`step.none`. When the step files cannot be made, `step show` exits 10 with the code a
dispatch would quarantine with (`artifact.unreadable`, `host.files-unwritable`) and
leaves the run as it is; only a dispatch or a claim quarantines.

## Cancelling a run (0.2)

`spore cancel <runId> [--reason <text>] [--id <requestId>]` ends an unfinished run on
purpose: its status becomes `cancelled`, its history is kept, and a step in progress
is told to stop. It is one store operation (store.md §2 `cancel_run`), recorded on the
run beside its kernel state, as a quarantine is; it is not an event of the run, and the
workflow runs no steps of its own on the way out (Temporal's terminate, not its
cancel). A quarantined run can be cancelled; the cancel clears the quarantine.

- **A running command.** The store fences out its attempt. The worker running it is
  refused at its next heartbeat and stops the command's process group with the stop
  sequence, so the command is gone within `--heartbeat-ms` and `--stop-grace-ms` of the
  cancel, whichever process the worker is.
- **A held async step.** Its attempt is expired with reason `cancelled`, so the holder's
  later `step renew`, `complete` or `fail` exits 2 with `lease.lost` and what it says
  is kept. When the action declares `async.stop`, `spore cancel` then runs it once,
  under the guard, with the holder handle on stdin, the step's files and
  `RUNSPORE_DB` (host.md §3, Async commands), and waits for it, at most its timeout
  (10 s by default). Its output is kept as late evidence; a stop that fails is ignored.
- **Pending events**, a signal sent before the cancel or after it, stay unconsumed.

`--id` names the request; a missing one is generated. A repeat with the same ID is a
`duplicate` and runs no stop again; the same ID with another reason exits 2 with
`cancel.id-reused`, as for `signal` (§Keys used again). Cancelling a run that has
already completed, failed or been cancelled succeeds and changes nothing, disposition
`duplicate`. A run that does not exist exits 2 with `run.not-found`.

It exits 0 once the run is `cancelled`, or when it had already ended, and prints
`{"runId", "requestId", "disposition", "status", "cancel", "stops"}`: `status` is the
run's status afterwards, `cancel` its `{"reason", "at"}` as in `status`, null for a run
that ended otherwise, and `stops` one `{"invocationId", "attempt", "exit"}` per stop
that ran, `exit` null when it did not exit by itself. Text: `run <runId> cancelled
(request ID <id>)` and a line per stop, or for a `duplicate` `run <runId> had already
<status>; nothing changed` (`been cancelled` for a cancelled run).

`run`, `worker --until-parked` and `continue` working a run that is cancelled
meanwhile stop once their in-flight attempts of it have stopped, print its `status`,
and exit 7; `continue` on a cancelled run works nothing and exits 7. A cancelled run
is not unfinished: `list --unfinished` and `continue` leave it out, `list` shows it at
no node with `needs` null, and `step show` on it exits 2 with `step.none`. `events`
lists the cancel record after the run's events (§JSON output).

## Validation (0.2)

`validate` on a workflow folder builds it and reports **every problem and warning in
one run** (dot.md §6). Its diagnostics are messages, so they go to stderr; stdout
gets only the verdict, so `spore validate <folder> >/dev/null` still shows what is
wrong. In this order:

1. with `--fix`, each change it made (below);
2. one block per diagnostic, problems first, then in the order the build checks:
   `workflow.dot` alone, what joins the two files, `settings.json` alone, then the
   whole graph; by line within each. After 20 blocks it stops and says how many are
   left and that `spore validate --short <folder>` lists them all;
3. the count, last, because people read the end of the output first: `4 problems and 1
   warning in nightly-backup. Nothing was run.`, or `1 warning in nightly-backup.`;
4. on stdout, when there is no problem, `<workflow> is valid`.

A block is a heading with the file, the line and the code on the right; the message;
the author's lines with the spot marked; `To fix:` with the fix, and for some codes
the first fix applied to the author's lines (added lines marked `+`); then `Why:` and
`More: spore explain <code>` when the code has them:

```text
── Missing "effect" ────────────────── nightly-backup/settings.json:9  action.no-effect
Action "upload" does not say whether it is safe to run again after a crash.

  9 │     "upload": {
    │     ^^^^^^^^

To fix: add one of "pure" | "read-only" | "idempotent" | "unsafe" to action "upload". For example:

 11 │       "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"],
 12 │+      "effect": "idempotent"

Why: spore runs a step again after a crash only when its effect says that is safe.
More: spore explain action.no-effect
```

A warning's heading ends with `(warning)`. A heading of up to 18 characters is
followed by 18 dashes, a longer one by 2. Paths are the folder as given, then the
file. The [validation guide](../docs/validation.md) walks through a whole report.

| Option | Does |
| --- | --- |
| `--short` | Prints one line per diagnostic instead of the blocks: `<file>:<line>:<column>: <code>: <message>` |
| `--strict` | Warnings fail too: exit 2, and `valid` is false |
| `--fix` | Applies the first fix of each diagnostic whose fix is `safe`, writes `workflow.dot` and `settings.json`, prints each change as its changed lines (`Fixed <code> in <file>:`, old lines `-`, new lines `+`), then checks the folder again and reports what is left. It never applies a `needs-review` or `needs-choice` fix: choosing an effect, for one, is the author's call |

Exit codes: 2 when there is a problem, or a warning with `--strict`; else 0. Warnings
alone never change the exit code.

`start`, `run` and `build` refuse a folder with problems the same way, with exit 2:
in text mode the blocks and the count on stderr, in JSON mode the error
`workflow.invalid` whose `message` is the count and whose `details` is the object
below. They print a folder's warnings on stderr and go on. `build --strict` refuses a
folder with warnings.

**JSON.** `spore --json validate` prints `{valid, problems, warnings, diagnostics}`,
with `packageBytes` when valid and `fixed` (`[{code, file}]`) with `--fix`. `problems`
and `warnings` are decimal strings. Each diagnostic:

| Field | Value |
| --- | --- |
| `severity` | `problem` or `warning` |
| `code` | The code, such as `action.no-effect` |
| `title` | The code's title, the same for every diagnostic of the code |
| `message` | The message, with the author's values in it |
| `fix` | The fix, as text |
| `why` | The reason, or null |
| `file` | The file as the text output shows it: the folder as given, then `workflow.dot` or `settings.json` |
| `range` | `{start: {line, column}, end: {line, column}}`, 1-based, the end excluded; columns count characters |
| `path` | The JSON Pointer into `settings.json`, or null for a place in `workflow.dot` |
| `args` | `[{name, kind, value, pointer}]`: each argument, its kind (`input`, `key`, `place`, `computed` or `code`), its value, and the pointer it was read from or null |
| `fixes` | `[{applicability, edits}]`, the first being the example; `applicability` is `safe`, `needs-review` or `needs-choice`. Each edit has `file`, `op` (`set`, `remove`, `rename`, `replace` or `text`), for all but `text` `at` (a pointer) and `value`, `to` (the new key) or `key` and `value` (the member put in its place), and for every edit the same change as text: `range` and `text`, the new text for that range of the file as it was read |
| `docs` | The address of the code's help page |

These map one to one onto an LSP `Diagnostic` (`range` less one on each number,
`severity`, `code`, `codeDescription.href` from `docs`, `message`) and its fixes onto
`CodeAction`s of kind `quickfix` with a `WorkspaceEdit` built from `range` and `text`,
the `safe` one `isPreferred`.

`validate` on an artifact checks it as `start` would and prints `{valid: true,
problems: "0", warnings: "0", diagnostics: [], packageBytes}`, or refuses it as `start`
would.

**`spore explain <code>`** prints the code's page, offline: its title and severity, its
explanation when it has one, the block its example folder gives, and the change its
fix makes. `--json` prints `{code, severity, title, page, docs}`. Without a code it
lists every code with its severity and title (`{codes: [{code, severity, title}]}`).
An unknown code exits 2 with `explain.unknown-code`. The same pages are in
[docs/codes](../docs/codes/README.md).

**0.2: input schema.** When the workflow declares `inputSchema`
([schema.md](schema.md) §4), `start` and `run` check the input (`--input`,
`--input-file`, or `{}` when neither is given) against it before the database is
touched. Input that does not fit exits 2 with code `input.invalid` and creates no run
and no database file: the same command can be sent again with corrected input. The
output has the shape of `output.invalid`'s, without an outcome:

```text
spore: input.invalid: the input does not fit the workflow's inputSchema
/task: required, but missing
/branch: expected type "string", got integer (7)
```

In JSON mode the error object carries `details`:
`{"errors": [{"path": "/task", "message": "required, but missing"}, …]}`. An
`inputSchema` whose root is not `"type": "object"` is refused by `validate`, `start`
and `run`: in a folder as the diagnostic `schema.input-not-object` (beside
`schema.input-outside-subset` for one outside the subset), in an artifact with
`workflow.invalid-schema`. A workflow without `inputSchema` accepts any input.

## Project folder (0.2)

A repository keeps its workflows in one project folder, `.spore/`, at its root, and
its runs in one database outside the checkout, shared by every worktree and clone of
the repository:

```text
.spore/                     # in the repository, committed
├── settings.json           # the project's settings, optional
└── workflows/
    └── coding-loop/        # a workflow folder (dot.md)

~/.spore/db/<repo key>/     # $SPORE_HOME/db/<repo key>/, outside the repository
├── runspore.db             # with runspore.db-wal and runspore.db-shm beside it
└── runspore.db.files/
```

The **project root** is `<dir>`, the nearest folder at or above the current one that
holds a `.spore/` directory, found by walking up. It comes from the current folder,
never from the database's path, so the settings, the workflows by name and a run's
base folder below hold wherever the database is.

**Database.** Every command uses the first of:

1. `--db`;
2. `$RUNSPORE_DB`;
3. `<dir>/.spore/runspore.db`, when that file exists: the old place, kept so a
   project that has its database there goes on using it. `status` says so (below);
4. the shared database `$SPORE_HOME/db/<repo key>/runspore.db`.

`$SPORE_HOME` is `~/.spore` when unset or empty, the way `CARGO_HOME` overrides
`~/.cargo`; a relative one is taken from the current folder. Without a home folder
and without `$SPORE_HOME`, a command that needs the shared database exits 2 with
`project.no-home`. The **repo key** is a relative path, worked out from the project
root, or from the current folder without one:

- the `origin` remote (`git config remote.origin.url`), normalised to
  `host/owner/name`: the scheme, the user, the password, the port and a trailing
  `.git` are dropped and the host is lower case, so `git@github.com:Web-tree/runspore.git`
  and `https://github.com/Web-tree/runspore` are both `github.com/Web-tree/runspore`.
  An `origin` that is a local path, plain or `file://`, is that absolute path. Empty,
  `.` and `..` parts are dropped;
- without `origin`, the path of the git common dir (`git rev-parse --git-common-dir`),
  which every worktree of one clone shares, with links resolved;
- outside git, or when `git` cannot be run, the folder's own absolute path, with links
  resolved.

An absolute path as a key is the path without its root: `/home/me/repo/.git` is the
key `home/me/repo/.git`. Two worktrees of one repository therefore list the same runs,
a run started in one is worked by a `worker` in the other, and removing a worktree
(`git worktree remove`) keeps its runs. A writing command creates the database's
folder when it is missing.

A relative `--db` or `$RUNSPORE_DB` is taken from the current folder. Every command,
from any subfolder or worktree of the repository, therefore reaches the same database.
`status --json`, `list --json` and the `--json` of `run` and `worker --run` print the
database used as `"db"`, an absolute path, so a command always shows which file it
used. On a database at the old place, `status --json` adds
`"dbOldPlace": {"shared": <path or null>}`, the shared database the repository uses
once the files are moved there, and `status` in text adds a `db` line naming both.

**Base folder.** `start` and `run` record the run's base folder with the run (store
§2 `create_run`). It is the project root, else the current folder; `--base-dir`
overrides both. The recorded folder is absolute. Every dispatch of the run, by any
later command started from any folder or worktree, runs the run's commands from it
(host §2), so a run continued from a subfolder runs where its first try ran. A `start`
whose key names an existing run keeps the folder recorded first. When the recorded
folder is gone, as a removed worktree's is, the run's next command step fails with
`command.base-dir-missing`, naming the folder (host §3), rather than failing to spawn.

**Settings.** `.spore/settings.json` holds the project's settings. It is optional;
`spore init` does not write it. Today it holds two settings:

```json
{"worker": {"slots": 8, "diskFloorMb": 1024}}
```

`worker.slots` is how many steps one worker runs at once (host.md §2,
`max_concurrent_activities`), a whole number from 1; without it, 4. It holds for every
worker on the project's database, `run` and `worker`, and `status` shows it as the
`slots` limit. `worker.diskFloorMb` is the free space, in MiB, the disk of the files
folder must keep for a worker to take a step (§A full disk), a whole number from 0;
without it, 1024; 0 never pauses. A command reads the settings of the project root
found by walking up from the current folder, whatever database it uses; a step runs
from its run's base folder, so its `spore` reaches the settings of its run's project.
A command outside a project folder has no settings. Every command that opens the
database refuses, with exit 2
and `project.settings-invalid`, a file that is not JSON, holds a key spore does not
know, a slot count that is not a whole number from 1, or a disk floor that is not a
whole number from 0; the message names the file and what is wrong.

**`spore init`.** It creates `.spore/` and `.spore/workflows/` at the repository
root: the nearest folder at or above the current one that holds `.git`
(a folder, or the file of a git worktree), or the current folder when there is none. It
prints the root and the paths it created, `{"root", "created": [...]}`. It never
overwrites a file, and running it twice changes nothing. It refuses with exit 2, and
creates nothing, when:

- `project.nested`: a `.spore/` exists at or above the current folder at another level
  than the root;
- `project.legacy-database`: a 0.1 database (`runspore.db`, `runspore.db-wal`,
  `runspore.db-shm` or `runspore.db.files/`) exists at the root. The message says the
  file belongs to 0.1 and must be finished with the 0.1 binary or moved aside;
- `project.unusable`: one of the two paths exists but is not a folder, or it cannot
  be created.

`init` opens no database and ignores `--db` and `$RUNSPORE_DB`; the first writing
command creates the database. The database is outside the checkout, so `init` writes
no ignore file.

**The ignore file.** A database at `<dir>/.spore/runspore.db`, the old place or named
by `--db` or `$RUNSPORE_DB`, must stay out of git: `.spore/.gitignore` must then hold
the lines `runspore.db`, `runspore.db-*` and `runspore.db.files/`. Every writing
command whose database is there adds whichever of these lines are missing, keeps every
other line, and creates the file if it is absent.

**Only writing commands create a database.** The writing commands are `start`, `run`,
`worker`, `signal`, `resolve`, `release`, `continue` and `step` but `step show`. The
read-only commands `status`, `list`, `events` and `step show` treat a missing database
as empty: `list` shows no runs, `status`, `events` and `step show <runId>` exit 2 with
`run.not-found`, and `step show` alone finds no unfinished run. `continue` on a
missing database has nothing to continue, and says so as below. None of them creates
the file. `validate` and `init` open no database.

**Steps reach the run's database.** Every program the host starts gets `RUNSPORE_DB`,
the absolute path of the database the worker uses (host.md §3). A `spore` call from
inside a step therefore reaches the run's own database, even in a folder whose own
lookup would find another one, because `$RUNSPORE_DB` comes first.

**Step files and work folders.** The files folder is `<database path>.files`, beside
the database, so `--db x.db` uses `x.db.files/` and the ignore rules above cover it.
It holds each step's read-only step files and each visit's work folder (host.md §3,
Step files and work folders). Only commands that dispatch steps (`run`, `worker`)
write it. A run whose stored artifact cannot be read stops quarantined with code
`artifact.unreadable` (exit 5); `release` after repairing the database lets it go on.

## Databases from another version (0.2)

A database records which `spore` wrote its schema (store.md §3). Every command that
opens one, whether it reads or writes, first checks that version, and refuses the
file with exit 10 before it reads or changes anything else:

- a database written by `spore` 0.1 is `schema.too-old`. The message names the
  database, says it belongs to 0.1, and gives the fix: finish its runs with the 0.1
  binary, or move the file aside;
- a database written by a newer `spore` is `schema.too-new`. The message names the
  database and the version that wrote it, and gives the same fix with that version.

Two versions of `spore` therefore never share a database, and neither can damage the
other's runs. A database written by an earlier 0.2 `spore` is upgraded in place and
used. `init` opens no database; it refuses a 0.1 database at the root by its
name (`project.legacy-database`, above).

## Progress (0.2)

`run`, `worker` and `worker --until-parked` print one line on stderr for each change
of each run they commit, as they commit it, so a person can watch the runs work. A
line is the time in UTC, the run's start key and the change, two spaces apart:

```text
2026-10-14T19:22:03Z  ticket-148  step build started, attempt 1
2026-10-14T19:22:05Z  ticket-148  step build attempt 1 failed: command.exit: exit code 1
2026-10-14T19:22:05Z  ticket-148  step build started, attempt 2
2026-10-14T19:23:41Z  ticket-148  build returned ok -> test
2026-10-14T19:24:10Z  ticket-148  test returned red, took exhausted -> review
2026-10-14T19:24:10Z  ticket-148  waiting at review for signal approval
```

The changes are: a step started, with its node and attempt; a try failed, with its
error code and the first line of its message; a node's outcome and the arrow the run
took (`took exhausted` when the outcome used up a loop limit), one line per node the
run left in that transition; waiting at an await-signal node, with the signal's name;
needs intervention, with the reason (a stuck step's error, or an unknown outcome);
completed, at its complete node; failed, with the error that ended it; quarantined,
with its code and details; and paused, while the worker's disk is full (§A full disk):
`paused: disk full (<free> free on <mount>)` for a run whose step it did not take,
`paused at <node>: …` for a run whose try it set aside. Paused is told once per pause
and is not a committed change: no event records it. A transition that goes round a loop of buffered signals
shows each node it left once, with its last arrow. A result that comes too late to
count changes nothing and prints nothing.

With `--json` each line is one JSON object instead: `format`, `atMs`, `runId`,
`startKey`, `change`, and the change's fields:

| `change` | Fields |
| --- | --- |
| `step-started` | `nodeId`, `attempt` |
| `try-failed` | `nodeId`, `attempt`, `error: {code, message}` |
| `outcome` | `nodeId`, `outcome`, `arrow`, `to` |
| `waiting` | `nodeId`, `signal` |
| `needs-intervention` | `nodeId`, `reason` |
| `completed` | `nodeId` (null when not known) |
| `failed` | `nodeId` (null when not known), `error: {code, message}` |
| `quarantined` | `code`, `details` |
| `running-elsewhere` | `nodeId`, `attempt`, `worker`, `pid`, `forMs` (`pid` and `forMs` null when not known); printed once by `continue --wait` before it waits on a step another worker runs (§What a run needs) |
| `paused` | `nodeId` (null when no try was set aside), `reason` |

`message` is the first line of the error's message; `status` shows the whole error.
Stdout is unchanged: `run` and `worker --until-parked` still print the run's status
there when they return, in JSON mode one object. A worker prints only what it commits
itself; a run another worker moves shows on that worker's stderr. The planned `status --follow`
is to print the same lines, found by replaying the run's events.

## A full disk (0.2)

A step whose machine is out of disk fails for a reason that is not in its code: `cargo
test` gets `disk I/O error` from SQLite, a build cannot write its output. Counting that
try as the step's red result sends the run round its fix loop for nothing. As GitHub
Actions runners and the Kubernetes kubelet stop taking work on a node under disk
pressure, a worker whose disk is full takes no new step until space returns (host.md
§2, Disk pressure).

The disk is the one holding the files folder (§Project folder), and it is **full**
while its free space is under the project's floor, `worker.diskFloorMb`, 1024 MiB
without it. Then:

- `run`, `worker` and `continue` claim no step. Each run whose step waits shows
  `paused: disk full (<free> free on <mount>)` in their progress (§Progress), once per
  pause. They keep working everything else: signals, results, sweeps.
- A try that ends while the disk is full is not counted. A repeatable step (`pure`,
  `read-only`, `idempotent`) keeps its attempt, which the worker holds, and runs again
  in it once there is space; its progress shows `paused at <node>: disk full (…)`.
  An `unsafe` step is reported `unknown` with code `host.disk-full`, and waits for a
  person, as any unsafe step whose outcome is unknown.
- A step whose files cannot be written because the disk is full is not quarantined
  with `host.files-unwritable`; it waits like the others.

`status` of a run waiting for a worker (`needs: working`) and `list` show the pause as
their first line, and in JSON as `paused`:

```text
paused: disk full (312 MiB free on /System/Volumes/Data)
```

```json
"paused": {"reason": "disk-full", "message": "disk full (312 MiB free on /System/Volumes/Data)",
           "freeBytes": "327155712", "floorBytes": "1073741824", "mount": "/System/Volumes/Data"}
```

`paused` is null while the disk is not full, and for a run that does not wait for a
worker. `status` and `list` measure the disk when they run, so the line goes as soon
as space returns. A worker that stops while it holds a set-aside try leaves the
attempt to its lease, as after a crash.

## Keys used again (0.2)

A start key names one run, and a `signal` or `resolve` ID names one request on a
run. Using one again for the same thing repeats it; using it for something else is
refused with exit 2, an error that says what the key already names, and the ways on.
The store's `request.digest-mismatch` (store.md §1) is never what a person sees.

`start` or `run` with the key of a run started with another input, or from another
version of the workflow, is `run.key-reused`. The message names the run, what differs
(for the input, the input it started with) and the commands that go on with it: the
same `start` or `run` with that input, unless the workflow changed, and
`spore worker --run <runId> --until-parked`, which runs the run on the workflow it
started with. Another `--key` starts a new run instead:

```text
spore: run.key-reused: start key ticket-158 already names run run_3f…, started with another input: {"ticket":157}.
To go on with it: spore run build --key ticket-158 --input '{"ticket":157}'
              or: spore worker --run run_3f… --until-parked
To start a new run instead, use another --key.
```

In JSON mode the error's `details` are `{runId, startKey, differs, input, commands}`:
`differs` lists `input`, `workflow` or both, `input` is the run's input, and
`commands` the commands above.

`signal --id` with an ID the run already took for a signal with another name, outcome
or data is `signal.id-reused`; `resolve --id` with an ID it took for another
resolution is `resolve.id-reused`; **0.2:** `cancel --id` with an ID it took for a
cancel with another reason is `cancel.id-reused`. The message names the run and the ID and says to
send it with a new `--id`, or without one; `details` are `{runId, id}`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. For `run`, `worker --until-parked` and `continue`: the run completed; for `continue` without `--wait`, also a run at a step another worker runs. For `continue --wait --json` without `--exit-status`: it printed the run's status, whatever the run needs (§What a run needs) |
| 1 | The run failed |
| 2 | Usage or validation error; for `validate`, a problem, or a warning with `--strict` |
| 3 | The run is waiting for a signal |
| 4 | The run needs intervention: a stuck step, or an unsafe step with an unknown outcome |
| 5 | The run is quarantined |
| 6 | **0.2.** The run is waiting on an async step: an actor must claim it or report it, or the work an async command started has not reported; for a held step, a person may abandon it; or on a step this `spore` cannot act on (`step.state` is `capability-missing`, §Capabilities). For `continue --wait`, only a ready external step, a held step whose holder waits for a person (`step.waiting` is `person`), or a `capability-missing` step |
| 7 | **0.2.** For `run`, `worker --until-parked` and `continue`: the run was cancelled, before the command or while it worked (§Cancelling a run) |
| 10 | Store or internal error, including a database written by 0.1 or by a newer `spore` (§Databases from another version), and step files `step show` cannot make |
| 130 | Interrupted by SIGINT, SIGTERM or SIGHUP after a graceful shutdown |

## JSON output

Every object carries `"format": "runspore.cli/0.2"`, errors included; 0.1 wrote
`runspore.cli/0.1`. Counters and timestamps are decimal strings. `status` output:

```json
{"format": "runspore.cli/0.2", "runId": "run_…", "status": "needs-intervention",
 "revision": "4", "packageDigest": "sha256:…", "artifactDigest": "sha256:…",
 "position": {"nodeId": "deploy", "activationId": "deploy/1", "visit": 1,
              "from": {"nodeId": "test", "outcome": "ok"}},
 "invocation": {"invocationId": "inv_…", "attempt": 3, "maxAttempts": 3, "state": "stuck",
                "error": {"code": "command.exit", "message": "exit code 1", "nodeId": "deploy",
                          "details": {…}}},
 "waiting": null,
 "intervention": {"nodeId": "deploy", "reason": "stuck", "invocationId": "inv_…",
                  "attempt": 3, "maxAttempts": 3, "attemptStatus": "failure",
                  "error": {"code": "command.exit", …}, "sinceMs": "…", "forMs": "…",
                  "outcomes": ["ok"], "outputSchemas": {},
                  "commands": ["spore resolve run_… --retry",
                               "spore resolve run_… --complete ok",
                               "spore resolve run_… --fail <message>"]},
 "running": null,
 "slots": {"limit": 4, "held": []},
 "streaks": {"test": {"outcome": "red", "count": 2, "limit": 3}},
 "step": null, "result": null, "end": null, "quarantine": null, "cancel": null}
```

`position.from` is the arrow the run took into its position: the node whose result
routed there and the outcome it routed on, the same arrow a mapping reads as `from`
(kernel.md §4). It is `null` at the start node's first activation. `end` is `null`
unless the run is `completed` or `failed` (a `cancelled` run has none); then it is `{"nodeId", "from"}`: the node
the run ended at and the arrow into it, for example
`{"nodeId": "done", "from": {"nodeId": "approve", "outcome": "approved"}}`. The node is
the complete or fail node the run reached, or the node named by the error that ended
it (`result.error.nodeId`): a step that failed with no `failed` arrow, or a node it
could not enter or whose mapping did not resolve. `from` is `null` when a run ends at
its start node.

The kernel's state holds no arrow. `status` finds both by replaying the run's consumed
events through the kernel: for each transition that moved the run, it starts at the
node the run was parked at and follows the route of the result each node then holds,
until it reaches the new node. The cost is one transition per event of the run.

`invocation` is the run's outstanding invocation, as the kernel state holds it
(kernel.md §3), plus `maxAttempts` from its action's retry policy (1 when it has
none). `state` is `scheduled`, `unknown` or `stuck`; at a stuck step `error` is the
kept error. `run` and `worker --until-parked` print the same object.

**0.2: what the run waits for.** `waiting`, `intervention` and `running` say, for the
run's status, what it waits for and how to move it on. The one named for the status
is an object; the other two are null, and all three are null for a `completed` or
`failed` run. Each has `sinceMs`, when the run came to stand so, and `forMs`, how long
ago that was when `status` read it. `sinceMs` is the accepted time of the event whose
transition last changed the run's status, its position or its invocation (its ID,
attempt or state); `status` finds it in the same replay as `position.from`. Nothing
here is recorded for `status`: it is read from the state, the package and the store.

- `waiting`, at an await-signal node: `{"nodeId", "signal", "sinceMs", "forMs",
  "outcomes", "outputSchemas", "commands"}`. `outcomes` are the node's outcomes but
  `exhausted`; `outputSchemas` is the node's (`{}` without them); `commands` has one
  `spore signal <runId> <signal> --outcome <o>` per outcome, ending ` --data <json>`
  when the outcome has a schema its data must fit.
- `intervention`, at a stuck step or an unsafe step whose outcome is unknown:
  `{"nodeId", "reason", "invocationId", "attempt", "maxAttempts", "attemptStatus",
  "error", "sinceMs", "forMs", "outcomes", "outputSchemas", "commands"}`. `reason` is
  `stuck` or `unknown`, the invocation's state. `attemptStatus` is the `status` of the
  invocation's latest consumed `activity.result` (`failure`, `success`, `unknown` or
  `expired`), or null. `error` is the kept error of a stuck step, else that result's
  error, null when it has none (an expired lease); its `details` carry the stream
  tails. `outcomes` are the action's, with the node's `outputSchemas`; `commands` are
  `spore resolve <runId> --retry`, one `--complete <o>` per outcome (with ` --output
  <json>` when it has a schema) and `--fail <message>`.
- `running`, at an activity: `{"nodeId", "actionId", "invocationId", "attempt",
  "maxAttempts", "claimed", "state", "worker", "pid", "queue", "sinceMs", "forMs"}`. `claimed`
  is whether a worker or an actor holds the attempt. `state` tells the kernel's one
  `scheduled` apart, as Temporal tells scheduled from started:
  - `started`: the attempt is claimed. `sinceMs` is when it was claimed (store.md
    §2); `worker` is the worker that claimed it, null for an actor's hold (`step`
    shows that); `pid` is the leader of the process group its program runs in, from
    the attempt's process record, null until the program has started.
  - `queued`: the try is due and nobody has claimed it: it waits for a worker with a
    free slot (`slots` below), or for any worker at all; at an external step, for an
    actor to claim it (`step`); at a `capability-missing` step, for a `spore` that
    can act on it (`step`). `sinceMs` is when the try
    was due: the later of the transition that scheduled it and the end of its
    backoff. **0.2:** `queue` is the step's place among the steps of the database
    that wait for a worker's slot, `{"place", "of"}`, `place` 1 being the step a
    worker takes next: the highest action `priority` first, then the one due longest
    (host.md §2, Dispatch). It is null at an external or `capability-missing` step,
    and in every other state.
  - `backoff`: the try is not due yet; `sinceMs` is when it will be, `forMs` is
    `"0"`.

  For `queued` and `backoff`, `worker` and `pid` are null. An attempt claimed by a
  `spore` before 0.2 recorded claim times runs from when it was due.

**0.2: worker slots.** A worker runs at most `worker.slots` steps at once (§Project
folder); each step a worker runs takes one of its slots until its try ends. `status`
carries `slots`: `{"limit", "held"}`. `limit` is the slot count of a worker of this
project, from its settings as `status` reads them. `held` lists every step a worker
runs now, in any run of the database: `{"worker", "runId", "startKey", "nodeId",
"attempt", "pid", "sinceMs", "forMs"}`, `sinceMs` being when the worker claimed it. A
step an actor holds takes no slot and is not listed. A queued step whose worker shows
`limit` steps in `held` waits for one of them to end; one with an empty `held` waits
for a worker to run.

Text mode shows `running` as `started by <worker> since …, for …, pid <pid>`,
`queued for a free worker slot, 3rd of 16 waiting since …, for …` (at an external step `queued for an
actor to claim it since …`, at a `capability-missing` step `queued for a spore that
can run it since …`) or `waiting to retry, due …`. Under it, except at an external or
`capability-missing` step, `slots` shows, per worker, `<n> of <limit> in use by <worker>` and one line per step it
runs: the run's start key, the node, the attempt, the pid and since when; with none,
`none of <limit> in use: no worker runs a step`.

The commands name the run and nothing else: a run in another database than the one
found by default needs `--db` added. Text mode shows the same under `waiting`, `needs`
and `running`, with the time in UTC and the duration in its two largest units, each
outcome with its schema, and the commands one per line; an unknown step's error and
its stream tails show there too.

**0.2:** `status` carries `parent` and `children` (§Runs that start runs).

**0.2:** a cancelled run shows `status: "cancelled"` and `cancel: {"reason", "at"}`:
the reason `cancel` was given (or null) and when the run was cancelled, a decimal
string of milliseconds. `position`, `invocation`, `streaks` and `nodes` stay as the run
had them when it was cancelled; `waiting`, `intervention`, `running`, `step`, `result`
and `end` are null. Text mode shows `cancelled  <time> UTC: <reason>`. `cancel` is null
for every other run.

**0.2:** a quarantined run keeps its kernel status in `status` and shows
`quarantine: {"code", "details", "atMs", "commands"}`, `commands` being
`["spore release <runId>"]`; `quarantine` is null for every other run. Text mode
shows the quarantine as the status, in its first lines, as a person reads it: `status
quarantined since …, for …: <code>: <details>` and under it `release  spore release
<runId>`. It shows no `running` or `slots`, since nothing works a quarantined run
until it is released.

**0.2:** `status` (and so `run` and `worker --until-parked`) and `list` carry `"db"`,
the absolute path of the database the command used (§Project folder). `list` prints
`{"db", "paused", "runs": [...]}`. Both carry `paused` (§A full disk).

**0.2:** `streaks` is the kernel state's `streaks` (kernel.md §3), each entry with the
node's `limit` for that outcome from its `loopLimits`: `test` above has routed `red`
twice in a row, and its fourth `red` in a row would take the `exhausted` edge. It is
`{}` when no node has a streak. After an `exhausted` edge, `position.from` shows
outcome `exhausted`, the edge the run took; the mapping's `from` shows the outcome the
node returned (kernel.md §4).

**0.2:** `invocation.process` is the process record of the invocation's latest attempt
(store.md §2 `set_process`): `{"host", "boot", "pgid", "started"}`, or null when the
attempt has none. A running command shows its record; a command that ended has had its
record cleared; an attempt whose worker died keeps its record.

**0.2:** `step` describes the async step the run is at (§Steps outside the
worker), or a step this `spore` cannot act on (§Capabilities), or is null anywhere
else: `{"kind", "actor", "state", "attempt", "holder", "deadlineMs", "leaseUntilMs",
"missing"}`. `kind` is the action's kind: `external`, or `command` for an async
command. `actor` is the action's label or null. `state` is `ready` while nobody holds
an external step and `held` while a claimed attempt runs, or an async command's
attempt is held; `holder` is the handle given with `--holder` or printed by the start
command (or null), and the two times are those of the held attempt, null while ready.
`state` is `capability-missing` when this `spore` lacks what the step's action needs
(host.md §2, The handshake), and `missing` then says what it lacks and what to
restore; `missing` is null in every other state.
A held step whose watch, run by this `spore`, has said that its holder waits for a
person (host.md §3, Async commands) also carries `"waiting": "person"` and `message`,
the message of the watch's latest `needs` line (empty when the line had none). Only a
command that works the run (`run`, `worker --until-parked`, `continue`) runs watches,
so `status` never carries them.
It never shows the step's input or the token.

**0.2:** `packageDigest` is the run's identity, the digest of the package it runs.
`artifactDigest` is the digest of the file it was started from, a workflow artifact or
a composed artifact (artifact.md §4, §7). It is null only for a run an embedding host
started from a bare package (host.md §2); `spore` itself never starts one.

Text output is for people and is not a stable interface.

**0.2:** where an error carries the ends of a command's streams (`stdoutTail`,
`stderrTail`, host.md §3), the text of `status` shows them under the error, and the
text of `events` under the event that holds it, so a failed command's cause shows on
whichever stream it went to. The JSON carries them in the error's `details`.

**0.2:** the text of `events` shows, as Temporal's event history shows each event's
attributes, one line per event: its sequence, its accepted time in UTC, its kind, the
revision that consumed it, the node it was for, what it said and what the run then
did, its diagnostics and its ID. The node is the start node for `run.started`, the
step of the invocation a result or a resolution names (a stale result included), the
await-signal node a signal was taken at, and the step an `onIntervention` action told
about; `-` for a buffered signal or an event not consumed yet. What it said is its
outcome (a result's outcome, or `failure`, `unknown` or `expired`; a signal's
outcome; a resolution's action), the arrow the run took from the node, as in
`red -> implement` or `red, took exhausted -> stuck`, then the first line of its error
as `code: message` or of its output or data as JSON, cut to 100 characters, then the
status the run came to when the event changed it (`waiting for <signal>`,
`needs intervention`, `completed`, `failed`). A run going round a loop therefore
shows the same arrows again with fresh times; a run that hangs shows an old last
event. The node and the arrows are found by replaying the run's consumed events
through the kernel, as `status` finds `position.from`; the JSON is unchanged.

**0.2:** a cancelled run's cancel is a store record, not an event. `events` lists it
after the run's events: the text ends with a line `   -  <time>  run.cancelled
store  <reason>  <requestId>`, and the JSON carries `cancel: {"reason", "at",
"requestId"}` beside `events`, null for a run that is not cancelled.

## Interrupts

On SIGINT, SIGTERM or SIGHUP the worker stops claiming, gives in-flight activities a
short grace period, stops the commands still running with the stop sequence (host.md
§3), and exits. Closing the terminal (SIGHUP) therefore shuts down like Ctrl-C. Work it
leaves behind is picked up by the next worker.

A worker that dies without a graceful shutdown (SIGKILL, a crash) still stops its
commands: each one's guard stops its process group. The guard is a hidden mode of
`spore` itself; it is not a command.

## Coming back after a kill (0.2)

A run lives in the database, not in the process that drives it. When that process
goes, whatever took it (a closed terminal, SIGKILL, a crash, the machine going down),
the next `spore continue` finds the run where the database says and works it from
there. Nothing is replayed from the dead process.

A driver that dies without a graceful shutdown records nothing: the try it was running
keeps its lease, the guard or the next sweep stops the try's process group (host.md §3,
The process record), and once the lease runs out the next driver expires the try and
the action's effect decides (host.md §4). A driver stopped by SIGINT, SIGTERM or SIGHUP
records the try as `unknown` (§Interrupts), so the next try needs no lease to run out.
Either way a try that was cut off is no result: it counts against the action's
`maxAttempts`, and changes no visit count and no loop count (kernel.md §7.4).

The check is the reference coding loop, [examples/coding-loop/](../examples/coding-loop/):
`implement` and `test` are `idempotent` commands of 3 tries each, `test` routes `red`
back to `implement` with `"loopLimits": {"red": 3}` and its `exhausted` arrow to
`tests_stuck`, and `approve` and `tests_stuck` are await-signal nodes. Killed at each
point, the run is found as follows:

| Killed | The run afterwards | `needs` | `continue` | What runs again | What does not |
| --- | --- | --- | --- | --- | --- |
| Before `start` returned | No run, or a run with no turn taken | none, or `working` | With no run, `nothing to continue` and the workflows (exit 0); else it works the run from its start node | The start node's first try | — |
| During a try of `implement` before its last | `running` at `implement`, no result recorded | `working` | Works on: the next try, once the lease has run out | `implement`'s command, with `step.resumed` `true` and the same input | Recorded results, earlier visits, decisions, visit and loop counts |
| During the last try of `implement` | `running` at `implement`; once the lease has run out, the next driver's sweep makes it `needs-intervention`, error `activity.attempts-exhausted` | `working`, then `stuck` | The step, its error and the three `spore resolve` forms (exit 4); after `--retry`, a fourth try | `implement`'s command, after `--retry` | As above |
| After a result was recorded, before the next step started | `running`, the next step's try due | `working` | Starts the next step | Nothing | Everything recorded |
| After a command returned, before its result was recorded | `running`, the try cut off | `working` | The next try, once the lease has run out | The command, which finds what its cut-off try left on disk | As above |
| During a try of `test` | `running` at `test`, no result recorded | `working` | The next try, once the lease has run out | `test`'s command, from the start | `implement`'s result, the red count |
| During the last try of `test` | As for `implement`, at `test` | `working`, then `stuck` | As for `implement` (exit 4) | `test`'s command, after `--retry` | As above |
| While it waits at `approve` or `tests_stuck` | `waiting` | `signal` | The node, `spore step show` and one `spore signal` per outcome (exit 3) | Nothing | Everything |
| After a signal was sent, before it was applied | `waiting`, the signal stored | `working` | Applies the stored signal and goes on; the same signal again with the same `--id` is a `duplicate` | Nothing | The decision |
| After the run ended | `completed`, `failed` or `cancelled` | not listed | `nothing to continue` (exit 0); with the run ID, its end (exit 0, 1 or 7) | Nothing | — |

`spore cancel` at any of these points ends the run as `cancelled` (§Cancelling a run):
`list --unfinished` leaves it out, `continue` on it exits 7, and no step runs again.
