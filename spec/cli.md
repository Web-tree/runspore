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
| `--db <path>` | `$RUNSPORE_DB`, else the project folder's database, else `./runspore.db` (§Project folder) | Database file |
| `--json` | off | One JSON object on stdout instead of text |
| `--native-kernel` | off | Call the kernel natively instead of through Wasmtime; for debugging |
| `--lease-ms`, `--heartbeat-ms`, `--poll-ms`, `--grace-ms` | engine defaults | Worker tuning; mainly for tests |
| `--stop-grace-ms` | 5000 | Time a stopped command's process group gets between SIGTERM and SIGKILL (host.md §3, `stop_grace_ms`); used by `worker` and `run` |

## Commands

| Command | Does |
| --- | --- |
| `init` | **0.2.** Creates the project folder (§Project folder) |
| `validate <workflow>` | Builds the workflow and checks it and its actions without touching the database |
| `build <folder> [-o <file>] [--composed]` | **0.2.** Builds a workflow folder (or its `workflow.dot`), checks it as `validate` does, and writes the workflow artifact to `<file>`, by default `<folder>.wasm` beside the folder. With `--composed`, the composed artifact (artifact.md §7), by default `<folder>-composed.wasm` |
| `inspect <artifact>` | **0.2.** Loads an artifact and prints its digest, manifest and package as JSON |
| `inspect --run <runId> --file <path>` | **0.2.** Prints the file the run's artifact holds at `<path>`, for example `workflow.dot` |
| `start <workflow> [--input <json> \| --input-file <path>] [--key <startKey>] [--base-dir <dir>]` | Creates a run and returns its run ID. A missing key is generated; the same key always names the same run in this database. `--base-dir` (0.2) overrides the run's base folder. The input is checked against the workflow's `inputSchema` (0.2) |
| `run <workflow> [--input …] [--key …] [--base-dir …]` | `start`, then works until the run is terminal or parked |
| `worker [--run <runId>] [--until-parked]` | Works on all runs until interrupted. With `--run --until-parked`, on one run until it is terminal or parked |
| `status <runId>` | Status, position and the arrow into it, result and the end node, quarantine reason, outstanding invocation |
| `list` | All runs with status |
| `events <runId>` | The run's events in order, with the revision that consumed each and its diagnostics |
| `signal <runId> <name> [--outcome <o>] [--data <json>] [--id <messageId>]` | Delivers a signal. A missing ID is generated; repeating an ID is a no-op |
| `resolve <runId> [<invocationId>] (--complete <outcome> [--output <json>] \| --retry \| --fail <message>) [--id <requestId>]` | Answers a stuck step, or an invocation whose outcome is unknown. Without an invocation ID it answers the run's outstanding invocation; a run with none exits 2 (`run.no-invocation`) |
| `release <runId>` | Lifts a quarantine |

A `<workflow>` is a workflow folder or its `workflow.dot`, which is built in memory
as [dot.md](dot.md) describes, or a workflow artifact (a file starting with the
WebAssembly magic bytes). Nothing else is read: any other file, such as a JSON
workflow document of 0.1, exits 2 with `workflow.folder` and a message saying that a
workflow is a folder holding `workflow.dot` and `settings.json`, or an artifact built
from one. A folder of dialect `runspore/0.1`, or whose `settings.json` says
`runspore.workflow/0.1`, is a build error that names the fix (dot.md §3). A build
error exits 2 and names the file and line (dot.md §6); an artifact that does not load
exits 2 with its code (artifact.md §6). JSON workflow documents remain an internal
form: the package inside an artifact, and a test fixture.

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
`validate`, `start` and `run` refuse a schema outside the subset with exit 2 and
`workflow.invalid-schema` (dot.md §6).

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
and `run` with `workflow.invalid-schema`, like any schema outside the subset. A
workflow without `inputSchema` accepts any input.

## Project folder (0.2)

A repository keeps its workflows and its runs in one project folder, `.spore/`, at its
root:

```text
.spore/
├── .gitignore              # written by spore
├── workflows/
│   └── coding-loop/        # a workflow folder (dot.md), committed
├── runspore.db             # not committed
└── runspore.db.files/      # not committed
```

**Database.** Every command uses the first of:

1. `--db`;
2. `$RUNSPORE_DB`;
3. `<dir>/.spore/runspore.db`, where `<dir>` is the nearest folder at or above the
   current one that holds a `.spore/` directory;
4. `./runspore.db`, when no `.spore/` exists up to the filesystem root.

A relative path is taken from the current folder. Every command, from any subfolder of
the repository, therefore reaches the same database. `status --json` and `list --json`
print the database used as `"db"`, an absolute path.

**Base folder.** `start` and `run` record the run's base folder with the run (store
§2 `create_run`). It is `<dir>` whenever the database path is
`<dir>/.spore/runspore.db`, however that path was given: walk-up, `--db` or
`$RUNSPORE_DB`. Otherwise it is the current folder. `--base-dir` overrides both. The
recorded folder is absolute. Every dispatch of the run, by any later command started
from any folder, runs the run's commands from it (host §2), so a run continued from a
subfolder runs where its first try ran. A `start` whose key names an existing run keeps
the folder recorded first.

**`spore init`.** It creates `.spore/`, `.spore/workflows/` and `.spore/.gitignore` at
the repository root: the nearest folder at or above the current one that holds `.git`
(a folder, or the file of a git worktree), or the current folder when there is none. It
prints the root and the paths it created, `{"root", "created": [...]}`. It never
overwrites a file, and running it twice changes nothing. It refuses with exit 2, and
creates nothing, when:

- `project.nested`: a `.spore/` exists at or above the current folder at another level
  than the root;
- `project.legacy-database`: a 0.1 database (`runspore.db`, `runspore.db-wal`,
  `runspore.db-shm` or `runspore.db.files/`) exists at the root. The message says the
  file belongs to 0.1 and must be finished with the 0.1 binary or moved aside;
- `project.unusable`: one of the three paths exists but is not of its kind (a file
  where a folder belongs, or the reverse), or it cannot be created.

`init` opens no database and ignores `--db` and `$RUNSPORE_DB`; the first writing
command creates the database.

**The ignore file.** `.spore/.gitignore` must hold the lines `runspore.db`,
`runspore.db-*` and `runspore.db.files/`. `init`, and every writing command whose
database is `<dir>/.spore/runspore.db`, add whichever of these lines are missing, keep
every other line, and create the file if it is absent.

**Only writing commands create a database.** The writing commands are `start`, `run`,
`worker`, `signal`, `resolve` and `release`. The read-only commands `status`, `list`
and `events` treat a missing database as empty: `list` shows no runs, and `status` and
`events` exit 2 with `run.not-found`. They never create the file. `validate` and
`init` open no database.

**Steps reach the run's database.** Every program the host starts gets `RUNSPORE_DB`,
the absolute path of the database the worker uses (host.md §3). A `spore` call from
inside a step therefore reaches the run's own database, even in a git worktree with
its own `.spore/`, because `$RUNSPORE_DB` comes before walk-up.

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

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. For `run` and `worker --until-parked`: the run completed |
| 1 | The run failed |
| 2 | Usage or validation error |
| 3 | The run is waiting for a signal |
| 4 | The run needs intervention: a stuck step, or an unsafe step with an unknown outcome |
| 5 | The run is quarantined |
| 10 | Store or internal error, including a database written by 0.1 or by a newer `spore` (§Databases from another version) |
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
 "streaks": {"test": {"outcome": "red", "count": 2, "limit": 3}},
 "result": null, "end": null, "quarantine": null}
```

`position.from` is the arrow the run took into its position: the node whose result
routed there and the outcome it routed on, the same arrow a mapping reads as `from`
(kernel.md §4). It is `null` at the start node's first activation. `end` is `null`
unless the run is `completed` or `failed`; then it is `{"nodeId", "from"}`: the node
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

**0.2:** `status` (and so `run` and `worker --until-parked`) and `list` carry `"db"`,
the absolute path of the database the command used (§Project folder). `list` prints
`{"db", "runs": [...]}`.

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

**0.2:** `packageDigest` is the run's identity, the digest of the package it runs.
`artifactDigest` is the digest of the file it was started from, a workflow artifact or
a composed artifact (artifact.md §4, §7). It is null only for a run an embedding host
started from a bare package (host.md §2); `spore` itself never starts one.

Text output is for people and is not a stable interface.

## Interrupts

On SIGINT, SIGTERM or SIGHUP the worker stops claiming, gives in-flight activities a
short grace period, stops the commands still running with the stop sequence (host.md
§3), and exits. Closing the terminal (SIGHUP) therefore shuts down like Ctrl-C. Work it
leaves behind is picked up by the next worker.

A worker that dies without a graceful shutdown (SIGKILL, a crash) still stops its
commands: each one's guard stops its process group. The guard is a hidden mode of
`spore` itself; it is not a command.
