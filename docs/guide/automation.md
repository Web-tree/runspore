# Automation and integrations

Use `spore` from scripts, CI or another program through its JSON output and exit
codes. The same commands you use interactively start runs, deliver answers and move
them forward. Runspore does not require a server, and the CLI does not expose a
network API.

This guide describes the current `runspore.cli/0.2` JSON contract. Check
`spore --version` and `spore --json capabilities` when connecting an integration to a
different build. Runtime versions, workflow dialects and JSON format versions are
separate values; do not infer one from another.

## Read JSON, keep progress separate

Add the global `--json` option to obtain structured output:

```sh
spore --json list --unfinished
spore --json status "$RUN_ID" --no-probe
spore --json step show "$RUN_ID"
```

Set `RUN_ID` to an actual run ID before using these commands. Normal JSON command
responses are one object on stdout. Each carries `"format":"runspore.cli/0.2"`;
errors carry an `error` object with `code`, `message` and sometimes `details`.
Revision and event-sequence values and timestamps are decimal strings. Attempt,
visit and maximum-attempt counts are JSON numbers. Follow the type of each field
instead of converting every number the same way. Treat IDs and tokens as opaque
strings.

Read `error.code` rather than matching human-readable text. Display `message` and
`details` when an operation needs attention. For example, `run.key-reused` means the
start key already names a run with different input or a different workflow; it is
not a reason to blindly retry with a new key.

Commands that work a run also write progress to stderr. In JSON mode these progress
messages are newline-delimited JSON, separate from the final object on stdout:

```sh
spore --json continue "$RUN_ID" --wait > run-status.json 2> run-progress.ndjson
```

Do not merge the two streams and parse the result as one JSON document. `inspect`
is a debugging interface whose output is not a stable integration contract. The
supported JSON fields and progress changes are specified in the
[CLI contract](../../spec/cli.md#json-output).

## Follow a run until it needs attention

`continue --wait` works the run, follows held async work and returns when the run
ends or needs an external answer. It is an active driver, not a read-only status
stream. It can apply events, run commands, probe holders and expire attempts.

For an integration that reads the returned status, use:

```sh
spore --json continue "$RUN_ID" --wait
```

In this specific mode, a successfully returned status exits **0**, including a run
that failed, was cancelled or needs an answer. Inspect `status`, `needs` and
`commands`. Errors and interrupts keep their own nonzero exit codes. The `needs`
and `commands` fields are part of `continue`'s response; do not assume the current
`status` response has the same fields. `list` also reports `needs` per run.

For CI that wants the run's exit code, add `--exit-status`:

```sh
spore --json continue "$RUN_ID" --wait --exit-status
```

`--exit-status` requires `--wait`. Plain `run`, `worker --run ... --until-parked`,
and text-mode `continue` already use the run's exit codes.

| Code | Meaning for a command working one run |
| --- | --- |
| `0` | Completed; plain `continue` can also return while another worker is doing the step |
| `1` | Run failed |
| `2` | Invalid arguments, workflow, input or request |
| `3` | Waiting for a signal |
| `4` | Needs intervention for a stuck step or an unknown unsafe result |
| `5` | Run quarantined |
| `6` | External/async work needs attention, or a required capability is missing |
| `7` | Run cancelled |
| `10` | Store or internal error, including incompatible database versions |
| `130` | Interrupted after graceful shutdown |

For `continue --wait`, a normally held async step is followed instead of returning
6 immediately. It returns 6 for ready external work, a watch asking for a person,
or a missing capability. Waiting for a signal is an expected state; decide whether
your CI should pause, notify someone or require a fully completed run.

Here is a shell wrapper that saves both streams and preserves the code, including
in a script that enables `set -e`:

```sh
run_exit=0
spore --json continue "$RUN_ID" --wait --exit-status \
  > run-status.json 2> run-progress.ndjson || run_exit=$?

case "$run_exit" in
  0) echo 'Run completed' ;;
  3|4|5|6) echo 'Run needs attention; inspect run-status.json' ;;
  7) echo 'Run was cancelled' ;;
  *) echo "Run or command failed with exit $run_exit" ;;
esac
exit "$run_exit"
```

This policy treats a parked run as an incomplete CI job. If your system deliberately
hands off approvals, use the returned structured state to record that handoff and
schedule the next driver invocation.

## Make repeated requests safe

Choose a start key from the operation you intend to perform, and reuse it when
retrying that same start:

```sh
spore --json start .spore/workflows/external-review --key review-change-42 \
  --input '{"summary":"Change the release checklist"}'
```

The workflow above is the one from [External steps](external-steps.md). The same
key, workflow and input return the same run in this database. A new key starts a
different run; the same key in another database also starts a different run.
Store the returned run ID and use it for later calls. `start` records a run but does
not work it; use `continue` or a worker next.

Signals, resolutions and cancellation accept request IDs. Reuse an ID only for the
same logical request and payload. A new answer or a later visit needs a new ID.
For a workflow that declares an `approval` signal with an `approved` outcome:

```sh
spore signal "$RUN_ID" approval --outcome approved --id approval-42-visit-1
spore continue "$RUN_ID"
```

The signal example requires an await-signal node with that name and outcome; the
external-review example instead reports through `step complete`. Sending a signal
records it. A driver must then apply it. If another worker is already serving the
database, it will do that work.

Runspore preserves recorded results across restarts, but a command can be repeated
if it performed an effect before its result was recorded. For an `idempotent`
action, pass `RUNSPORE_EFFECT_KEY` to the service that performs the effect, or use it
to recognize work already done. A declaration does not make a non-idempotent
operation safe. See [Authoring workflows](authoring-workflows.md) for effect classes.

## Keep database and base folder explicit

All participants must use the same database. For isolated CI runs, set `RUNSPORE_DB`
to a chosen absolute path, or pass `--db` on every call. For ordinary project work,
the default database is shared by repository identity on the current machine. See
[Project settings and storage](project-settings.md) for the full lookup order.

A worker started by Runspore gives its commands `RUNSPORE_DB` automatically. Nested
`spore` calls therefore reach the parent's database even when the action runs in
another folder. Integrations that launch their own processes must preserve the
same selection. The `db` field in `list` and `status` shows the resolved path.

The run also records its base folder at start. Set `--base-dir` when the work should
run somewhere other than the detected project root. Resuming from another directory
does not change that recorded folder. Retain the folder for as long as the run needs
to execute commands from it.

Keep the database, its SQLite side files and its adjacent `.files` directory across
driver restarts when you need to resume work. A fresh ephemeral CI filesystem does
not carry previous run state. The current supported setup is local SQLite on one
machine under one OS user; do not turn the database into a shared multi-machine
queue through a network filesystem.

## Start child runs from a step

A command action can start another workflow using the regular CLI. Runspore passes
`RUNSPORE_RUN_ID` and `RUNSPORE_NODE_ID` to that command, and the new run records them
as its parent. Parent and child use the same database.

For example, inside a command step, with your own `ticket` workflow installed:

```sh
spore start ticket --key "$RUNSPORE_RUN_ID:ticket:42" --input '{"ticket":42}'
```

The child's final command can send a signal back to a matching await-signal node in
its parent:

```sh
spore signal "$RUNSPORE_PARENT_RUN_ID" ticket-done --outcome ok \
  --id "$RUNSPORE_RUN_ID:done"
```

The parent workflow must define that signal and outcome; the CLI does not create a
join automatically. `spore list` displays parent/child relationships as a tree, and
`spore status "$RUN_ID"` includes the run's children. `continue` works the selected
run and the runs below it; `spore worker` serves all runs in the selected database.

These are CLI-created child runs. Do not substitute an independent SWL subflow or
parallel-profile example into a Runspore workflow without a supported adapter.

## Connect an agent or another tool

An integration has three usual choices:

- Invoke a tool as a `command` action and translate its result into the declared
  outcome and output. [A Claude step with plain spore](claude-step.md) shows one
  existing recipe.
- Claim an [external step](external-steps.md), do the work and report with its
  token. This suits a program that already manages its own work lifecycle.
- Declare an async command with start/probe/watch/stop programs when Runspore should
  launch the work and follow it after the launching process ends.

For observation, use `status --no-probe`, `list`, `events` and `step show`. For
progress, use `continue --wait` or a worker. There is no `status --follow` command in
the current CLI.

## Next

- [CLI reference](cli-reference.md): commands and global options.
- [Project settings and storage](project-settings.md): worker slots, disk pressure and upgrades.
- [Watch a run](watch-a-run.md): existing monitoring and notification recipes.
- [CLI contract](../../spec/cli.md): detailed JSON fields, progress messages and request semantics.
