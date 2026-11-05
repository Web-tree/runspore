# CLI reference

The executable is `spore`. Use `spore --help`, `spore COMMAND --help`, or
`spore step COMMAND --help` for the options on your installed binary. This reference
describes the current CLI contract `runspore.cli/0.2`; software releases have a
separate version number, shown by `spore --version`.

Uppercase words below are placeholders. Replace `RUN_ID` with the `run_…` ID printed
by `start`, `run`, or `list`; replace `TOKEN` with the token returned by a step
claim. A workflow argument can be a folder, its `workflow.dot`, a workflow artifact,
or a graph name under `.spore/workflows/`, except where a command specifically
requires a source folder or an artifact.

## Create and check workflows

| Command | Options | Purpose |
| --- | --- | --- |
| `spore init` | None | Creates the project folder and workflow directory; opens no database |
| `spore validate WORKFLOW` | `--short`, `--strict`, `--fix` | Validates without running actions; `--fix` applies safe source edits |
| `spore explain [CODE]` | None | Explains a diagnostic offline; without a code lists the catalogue |
| `spore build FOLDER` | `-o FILE` / `--output FILE`, `--strict`, `--composed` | Builds a source folder or its `workflow.dot` into a workflow artifact |
| `spore inspect ARTIFACT` | None | Prints an artifact's manifest and package for debugging |
| `spore inspect --run RUN_ID --file PATH` | Both named options required together | Reads a file from the run's stored artifact |

By default, `build` writes `FOLDER.wasm` beside the source folder. With
`--composed`, it writes `FOLDER-composed.wasm` containing the workflow artifact and
kernel component. Normal CLI use does not require a composed artifact. `inspect`
is a debugging interface; its output is not a stable API.

Tutorials: [Getting started](getting-started.md),
[Authoring workflows](authoring-workflows.md), and
[Validation](../validation.md).

## Start and drive runs

| Command | Options | Purpose |
| --- | --- | --- |
| `spore start WORKFLOW` | `--key KEY`, `--input JSON` or `--input-file FILE`, `--base-dir DIR` | Creates the run and returns |
| `spore run WORKFLOW` | Same as `start` | Starts or finds the keyed run, then works it until it finishes or parks |
| `spore worker` | `--run RUN_ID --until-parked` | Works all runs until interrupted, or one run until it parks when both options are supplied |
| `spore continue [RUN_ID]` | `--wait`, `--exit-status` with `--wait` | Does the next thing the run needs; without an ID selects the only unfinished run |

`--key` is a caller-chosen start key scoped to the selected database. Omitting it
generates a key. Reusing it with the same input and workflow identifies the same
run; a changed input or package is refused. A completed run is not restarted.

`--input` and `--input-file` are mutually exclusive. Without either, input is `{}`.
`--base-dir` records where the run's commands operate, and must name an existing
directory. Later continuations keep that recorded folder.

`continue --wait` follows through held asynchronous work until an outside
decision/action is needed or the run ends. In JSON mode it normally exits 0 after
printing a run's status; add `--exit-status` when the caller should receive the
run's exit code. `--exit-status` without `--wait` is invalid.

Guide: [Running workflows](running-workflows.md).

## Inspect and answer runs

| Command | Options | Purpose |
| --- | --- | --- |
| `spore list` | `--unfinished` | Lists runs, their workflow, position, and what they need |
| `spore status RUN_ID` | `--no-probe` | Shows status, result, waiting/stuck details, and available next commands |
| `spore events RUN_ID` | None | Shows the run's recorded event history |
| `spore step show [RUN_ID]` | None | Describes the current step without claiming it; selects the only unfinished run when omitted |
| `spore signal RUN_ID NAME` | `--outcome OUTCOME`, `--data JSON`, `--id MESSAGE_ID` | Records an answer for a named signal |
| `spore resolve RUN_ID [INVOCATION_ID]` | Exactly one of `--retry`, `--complete OUTCOME`, or `--fail MESSAGE`; optional `--output JSON` with `--complete`, and `--id REQUEST_ID` | Resolves a stuck step or unknown activity result |
| `spore release RUN_ID` | None | Clears quarantine after its cause is fixed |
| `spore cancel RUN_ID` | `--reason TEXT`, `--id REQUEST_ID` | Ends an unfinished run as cancelled and retains its history |

`status` may probe held asynchronous work. `--no-probe` reads stored status without
running that probe. `list`, `events`, and `step show` do not perform work on the run.

Signals default to outcome `received` and data `null`. Use the outcome and data that
the waiting node accepts. A repeated message or request ID with the same content
is a no-op; reusing that ID with different content is refused. If you omit the ID,
the CLI generates one.

`signal` and `resolve` record answers; a worker, `run`, or `continue` then drives the
next steps. `cancel` is final and does not undo completed external effects.

Guide: [Running workflows](running-workflows.md).

## External and asynchronous steps

| Command | Options | Purpose |
| --- | --- | --- |
| `spore step claim [RUN_ID]` | `--actor LABEL`, `--holder JSON` | Claims a ready external step and returns its token, input, and instructions |
| `spore step renew TOKEN` | None | Extends the claimed try's lease, up to its deadline |
| `spore step complete TOKEN` | Required `--outcome OUTCOME`, optional `--output JSON` | Reports the step's result |
| `spore step fail TOKEN` | Required `--error MESSAGE`, optional `--code CODE`, `--retryable` | Reports a failure; `--retryable` permits retries within the policy |
| `spore step abandon RUN_ID` | None | Releases a held asynchronous step so its effect/retry policy can decide what follows |
| `spore step propose TOKEN` | Required `--outcome OUTCOME`, optional `--output JSON` | Stores a candidate answer pending the action's stopped-work check |
| `spore step withdraw TOKEN` | None | Removes the current candidate answer |

Without a run ID, `claim` chooses eligible work across the selected database by
priority, then oldest due work. Without `--actor` it only claims unlabelled actions;
with a label it only claims matching actions. The label is a selection filter,
not access control. All commands using a token require that claim to remain valid.

Guide: [External and asynchronous steps](external-steps.md).

## Capabilities and upgrades

| Command | Purpose |
| --- | --- |
| `spore --version` | Reports the software version |
| `spore capabilities` | Reports version, contract versions, action kinds, async support, result channels, and component worlds |
| `spore migrate` | Explicitly upgrades an older supported database schema; refuses incompatible versions or live workers from another version |

Guide: [Projects and settings](project-settings.md#upgrade-the-cli-and-database).

## Global options

Global options can appear before or after the subcommand.

| Option | Default / behavior |
| --- | --- |
| `--db PATH` | Overrides `RUNSPORE_DB` and automatic database lookup |
| `--json` | Prints structured command output; progress remains on stderr |
| `--migrate` | Opts a writing command into a supported schema upgrade; read-only commands do not upgrade |
| `--native-kernel` | Uses the native kernel instead of Wasmtime, for debugging |
| `--lease-ms N` | Worker lease duration; default `10000` |
| `--heartbeat-ms N` | Worker lease renewal interval; default `3000` |
| `--poll-ms N` | Poll interval; default `200` |
| `--grace-ms N` | Shutdown grace for in-flight work; default `10000` |
| `--stop-grace-ms N` | Grace between SIGTERM and SIGKILL for a stopped command group; default `5000` |
| `--max-watches N` | Concurrent watches of held async steps per worker, separate from worker slots; default `16` |

Use project `worker.slots` and `worker.diskFloorMb` settings for normal concurrency
and free-space controls. Lease and heartbeat flags are advanced tuning, mainly
used by tests. [Projects and settings](project-settings.md) explains database
selection, run folders, and migration behavior.

## Exit codes and output

| Code | Meaning |
| --- | --- |
| `0` | Command succeeded; driving commands normally mean completed, with the `continue` exceptions below |
| `1` | Run failed |
| `2` | Usage, validation, input, or answer error |
| `3` | Run waits for a signal |
| `4` | Run needs intervention |
| `5` | Run is quarantined |
| `6` | External/async work or a missing capability needs attention |
| `7` | Run is cancelled |
| `10` | Store, schema-version, or internal error |
| `130` | Driver interrupted by SIGINT, SIGTERM, or SIGHUP |

These run-state codes describe `run`, `worker --run ... --until-parked`, and
`continue`; an inspection command can succeed while describing a failed run.
Ordinary `continue` also returns 0 if another worker is already working the run.
`continue --wait --json` returns 0 after a successful status response unless
`--exit-status` is supplied. Invalid commands and internal failures keep their
error codes.

Normal command JSON carries `"format":"runspore.cli/0.3"`. Use the JSON fields,
not human-readable output, in scripts. With `--json`, progress lines on stderr are
JSON objects too; do not mix stderr into stdout when consuming the final result.
`worker` is a long-running process, not a single-result query.

See [Automation](automation.md) for scripts and
the [full CLI contract](../../spec/cli.md) for field definitions and edge cases.
