# Running workflows

A workflow is a definition; a run is one execution with its own saved state.
`spore` commands either inspect that state, record an answer, or act as a driver
that moves the run forward. Nothing needs to remain running while a workflow waits
for a signal, but a driver must run again to consume the answer and perform the
next steps.

The examples use the `hello` and `review` workflows from
[Getting started](getting-started.md) and [Authoring workflows](authoring-workflows.md).
Where a command uses `$RUN_ID`, set it to an actual `run_…` ID printed by `run`,
`start`, or `list`:

```sh
RUN_ID=run_your_id_here
```

## Choose how to drive the work

| Command | Use it when | How long it runs |
| --- | --- | --- |
| `spore run WORKFLOW` | You want to start a run and work on it now | Until it finishes or parks |
| `spore start WORKFLOW` | You want to create a run for a worker to pick up | Returns after creation |
| `spore worker` | You want to keep processing runs in this database | Until interrupted |
| `spore continue RUN_ID` | You want to do the next thing an existing run needs | Until it can explain the next required action |
| `spore continue RUN_ID --wait` | You want to follow through held asynchronous work | Until it finishes or needs an outside decision/action |
| `spore worker --run RUN_ID --until-parked` | You want to work on one saved run directly | Until that run finishes or parks |

`run` and `start` accept a workflow folder, its `workflow.dot`, a `.wasm` workflow
artifact, or a workflow's graph name under `.spore/workflows/`. They accept input
as `--input` JSON or `--input-file`, never both. Without either, input is `{}` and
must still satisfy any input schema.

### Work in the foreground

```sh
spore run hello --key greeting-1 --input '{"message":"Hello"}'
```

### Use a worker in another terminal

In one terminal, from the project folder:

```sh
spore worker
```

In another terminal in the same project:

```sh
spore start hello --key greeting-2 --input '{"message":"Hello again"}'
```

`start` records the run without performing its actions. The worker picks it up.
The worker's stderr shows its progress. `worker --run` and `--until-parked` must be
used together when targeting one run; `worker --run` alone is not valid syntax.

## Use start keys deliberately

A start key deduplicates creation **within one database**. Repeating
`run hello --key greeting-1` with the original input returns or continues that run.
It never reruns a completed run. Omitting `--key` generates a key and creates a new
run; use an explicit key when retrying a start request should be safe.

If the input or workflow package differs, the CLI refuses the reused key with
`run.key-reused`. To continue the existing run, use its ID:

```sh
spore continue "$RUN_ID"
```

To execute the changed workflow, choose a new key. A run always uses its recorded
artifact, input, and base folder. Editing the source folder cannot change it.

## Find out what is happening

```sh
spore list --unfinished
spore status "$RUN_ID"
spore step show "$RUN_ID"
spore events "$RUN_ID"
```

Use these for different questions:

- `list --unfinished` finds work that has not completed, failed, or been cancelled.
  `list` also includes finished runs and shows parent/child relationships.
- `status` shows the position, result, error, and what the run needs. It prints
  commands you can use to move the run on.
- `step show` displays the current step's instructions, possible outcomes, and
  output schemas without claiming it. A finished run has no current step.
- `events` shows the recorded history and the revisions that consumed events.

`status` can probe a held asynchronous step and record what the probe finds. Use
`spore status "$RUN_ID" --no-probe` when you want to read the stored status without
running a probe. See [External and asynchronous steps](external-steps.md).

`continue` without an ID chooses the only unfinished run. When several exist, it
lists them and asks you to name one; it does not guess. When none exists, it tells
you there is nothing to continue and lists workflows it can find.

## Answer a signal

The `review` workflow stops at an approval:

```sh
spore run review --key review-2 --input '{"message":"Please review"}'
```

Copy this run's ID into `$RUN_ID`. Read its step, then choose **one** answer:

```sh
spore step show "$RUN_ID"
spore signal "$RUN_ID" approval --outcome approved --id review-2-approval
spore continue "$RUN_ID"
```

To reject instead, send the alternative command to the waiting run:

```sh
spore signal "$RUN_ID" approval --outcome rejected \
  --data '{"reason":"Needs changes"}' --id review-2-approval
```

Then continue it, or let an already-running worker consume the signal. The signal
name, outcome, and any data must match the workflow. Omitting `--outcome` means
`received`; it does not automatically choose `approved`.

Use an explicit `--id` when retrying delivery: the same ID with the same content is
a no-op after the first receipt. The same ID with different content is refused.
For a workflow that loops back to approval, use a new message ID for each distinct
answer; reusing the previous answer's ID will not answer the next visit.

A signal is a message to a workflow's waiting node. It is different from a
resolution of a stuck or interrupted activity.

## Recover an interrupted or stuck step

After a terminal closes or a process dies, return to the project and run:

```sh
spore list --unfinished
spore continue "$RUN_ID"
```

Completed steps remain completed. An interrupted repeatable action can run again
after its outstanding try is dealt with; the retry limit still applies. An `unsafe`
action with an unknown result stops for a person because repeating it could apply
the external effect twice. A step that exhausted its retries without a `failed`
route also stops for a person. These cases exit **4** and appear as
`needs-intervention`.

Inspect the state and the outside system before choosing a resolution:

| What you know | Command |
| --- | --- |
| Another try is appropriate | `spore resolve "$RUN_ID" --retry` |
| The work already completed and its outcome is known | `spore resolve "$RUN_ID" --complete ok --output '{"confirmed":true}'` |
| The run must end as failed | `spore resolve "$RUN_ID" --fail "Unable to complete the work"` |

Choose only one. Replace `ok` and the output with an outcome and data accepted by
that step; `spore step show` displays its schemas. `--retry` authorizes a new try
even when the normal attempt limit was reached. It does not undo or deduplicate an
earlier external effect.

After resolving, run `spore continue "$RUN_ID"` or leave a worker to process it.
`resolve` also accepts an explicit invocation ID and a request `--id` for callers
that need to identify the exact operation; see the [CLI reference](cli-reference.md).

## Handle quarantine and a full disk

Quarantine means Runspore could not safely process the run, for example because its
stored artifact could not be read or required step files could not be written.
Inspect the reason with `status`, fix the named cause, then lift the quarantine:

```sh
spore status "$RUN_ID" --no-probe
spore release "$RUN_ID"
spore continue "$RUN_ID"
```

`release` only clears the quarantine; it does not repair the underlying problem.
If that problem remains, the run can be quarantined again.

A worker that reaches its configured free-space floor pauses taking new steps.
Free space on the filesystem holding the run's files and let it resume. This disk
pause is different from a run's quarantine. The floor and worker concurrency are
configured in [Projects and settings](project-settings.md#worker-settings).

## Stop a driver or cancel a run

Pressing Ctrl-C in `run`, `continue`, or `worker` stops that driver with a graceful
shutdown; the process exits **130**. Saved runs remain available for a later driver.
An interrupted step may need a retry or resolution when continued, according to
its effect declaration. An ordinary command is guarded so it is stopped when the
worker dies; a deliberately asynchronous action has its own lifecycle.

To end a run intentionally:

```sh
spore cancel "$RUN_ID" --reason "No longer needed" --id cancel-request-1
```

Cancellation retains history, stops an in-progress command, and asks a held
asynchronous action to stop when it supplies a stop operation. It does not undo
completed steps or compensate external effects. A cancelled run cannot be
continued; `continue` on it exits **7**. A new run needs a new start key.

## Interpret the return code

For `run`, `worker --run ... --until-parked`, and ordinary `continue`, a nonzero
exit can describe work waiting for you rather than a CLI failure:

| Code | Meaning |
| --- | --- |
| `0` | Completed; ordinary `continue` also uses 0 when another worker is already doing the work |
| `1` | Run failed |
| `2` | Invalid command, workflow, input, or answer |
| `3` | Waiting for a signal |
| `4` | Stuck step or unknown unsafe result needs a resolution |
| `5` | Quarantined |
| `6` | External/asynchronous step or missing host capability needs attention |
| `7` | Cancelled |
| `10` | Store, schema-version, or internal error |
| `130` | Driver interrupted |

`continue --wait --json` deliberately exits 0 when it successfully prints a run's
status, even when that run is parked or failed. Add `--exit-status` to retain the
run's code. Invalid commands and internal errors still use their error codes.
[Automation](automation.md) shows how to handle this without treating an approval
wait as a broken script.

For notifications and progress across related runs, continue with
[Watch a run](watch-a-run.md). For an actor that performs a step itself, use
[External and asynchronous steps](external-steps.md).
