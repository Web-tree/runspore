# Watch a run, and get told when it needs you

A run works in the background and sometimes stops for a person: at a step that waits
for your answer, or at a step that keeps failing. spore has four tools for that:

- `spore worker` prints a line for each change of each run it works, so you can watch
  the runs move;
- `spore list` shows every run as a tree, each with where it is and for how long;
- `spore status` says what a run waits for and prints the commands that move it on;
- `onIntervention` in `settings.json` runs an action of yours, a notification say,
  each time a run stops for a person or fails, so you do not have to keep looking.

This page walks through them on the example in
[`examples/watch-a-run/`](../../examples/watch-a-run/). Every output below is what
`spore` printed; your run IDs, digests and times will differ.

## The example

Two workflows. `ship` ships one service: it builds it, waits for you to approve, and
deploys it. Its `deploy` fails while a file named `down` is in the project folder, as
if the target were down, and gives up after two tries.

```text
build ──▶ approve ──approved──▶ deploy ──▶ done
             │
             └──rejected──▶ stopped
```

`release` starts one `ship` run per service, `api` and `web`, with `spore start`, then
waits until you say the release is out.

```text
start_ships ──▶ released ──ok──▶ done
```

Each `ship` run is started from inside a `release` step, so it records that run as its
parent, and `spore list` shows it below it.

## Set it up

From your project's root:

```sh
spore init
cp -R <runspore>/examples/watch-a-run/ship <runspore>/examples/watch-a-run/release .spore/workflows/
spore validate ship
spore validate release
```

`<runspore>` is a checkout of this repository. `spore` must be on `PATH`, since the
`start_ships` step calls it.

## Watch the runs work: `spore worker`

Open a second terminal in the same folder and start a worker. It works every run in
the project's database until you press Ctrl-C:

```sh
spore worker
```

In the first terminal, start the release:

```sh
spore start release --key release-1.4 --input '{"version": "1.4"}'
```

```text
created run run_c0e091bdadf6ade0e772f2a3dd8b7340 (start key release-1.4)
```

`start` only creates the run; the worker does the work, and prints each change on
stderr as it commits it:

```text
2026-10-14T22:53:11Z  release-1.4  step start_ships started, attempt 1
2026-10-14T22:53:28Z  api-1.4  step build started, attempt 1
2026-10-14T22:53:28Z  api-1.4  build returned ok -> approve
2026-10-14T22:53:28Z  api-1.4  waiting at approve for signal approval
2026-10-14T22:53:44Z  web-1.4  step build started, attempt 1
2026-10-14T22:53:44Z  release-1.4  start_ships returned ok -> released
2026-10-14T22:53:44Z  release-1.4  waiting at released for signal released
2026-10-14T22:53:44Z  web-1.4  build returned ok -> approve
2026-10-14T22:53:44Z  web-1.4  waiting at approve for signal approval
```

A line is the time in UTC, the run's start key and the change. The changes are: a step
started, with its try; a try failed, with its error; a step's outcome and the arrow
the run took; waiting at a node, for which signal; needs intervention, and why;
completed or failed, and where; and quarantined. `api-1.4` and `web-1.4` are the two
`ship` runs, worked by the same worker as their parent.

`spore run` prints the same lines for the runs it works, then the run's status on
stdout. With `--json`, each line is one JSON object, for a script to read. A worker
prints only what it does itself: a run another worker moves shows on that worker's
terminal.

## See every run: `spore list`

```sh
spore list
```

```text
run_c0e091bdadf6ade0e772f2a3dd8b7340    waiting             released          0s  release-1.4
  run_69521f33daea0013880016867d12b6ab  waiting             approve          16s  api-1.4  from start_ships
  run_536a8063c8fa3da941b15c8b193833ce  waiting             approve           0s  web-1.4  from start_ships
```

Each line is a run, its status, the node it stands at, how long it has stood there,
its start key, and for a run another run started, the parent's node that started it.
A run's children are indented below it. A finished run is at no node and shows `-`.
`spore list --json` gives the same as `{"db", "runs": [...]}`, each run with its
`parent` and `depth`.

## Ask what a run waits for: `spore status`

```sh
spore status run_69521f33daea0013880016867d12b6ab
```

```text
run        run_69521f33daea0013880016867d12b6ab
start key  api-1.4
status     waiting
revision   2
package    sha256:b61fbe0b054e0a2b42580ee99e08be22d8fe1d05a645651951d94aada3e2e302
artifact   sha256:8130a594b8bda4fae335ed807dcdaeb61c4b5448208f6f25719ae8f910f5183f
parent     run_c0e091bdadf6ade0e772f2a3dd8b7340 at start_ships
position   approve (approve/1, from build on ok)
waiting    for signal approval since 2026-10-14T22:53:28Z, for 16s
outcomes   approved
           rejected
answer     spore signal run_69521f33daea0013880016867d12b6ab approval --outcome approved
           spore signal run_69521f33daea0013880016867d12b6ab approval --outcome rejected
```

`position` is where the run is and the arrow that brought it there. `waiting` is the
signal it waits for and since when; `answer` lists one command per outcome, ready to
copy. An outcome whose data must fit a schema shows the schema, and its command ends
in `--data <json>`. `status` of the parent lists its children at the end.

## When a step keeps failing

Take the target down and approve `api`:

```sh
touch down
spore signal run_69521f33daea0013880016867d12b6ab approval --outcome approved
```

```text
signal approval applied (message ID msg_692b99097479922c)
```

The worker deploys, fails twice, and stops the run for a person:

```text
2026-10-14T22:53:44Z  api-1.4  approve returned approved -> deploy
2026-10-14T22:53:44Z  api-1.4  step deploy started, attempt 1
2026-10-14T22:53:44Z  api-1.4  step deploy attempt 1 failed: command.exit: exit code 1
2026-10-14T22:53:44Z  api-1.4  step deploy started, attempt 2
2026-10-14T22:53:44Z  api-1.4  step deploy attempt 2 failed: command.exit: exit code 1
2026-10-14T22:53:44Z  api-1.4  needs intervention at deploy: stuck: command.exit: exit code 1
```

`status` now says why, shows what the command printed, and lists the ways on:

```sh
spore status run_69521f33daea0013880016867d12b6ab
```

```text
run        run_69521f33daea0013880016867d12b6ab
start key  api-1.4
status     needs-intervention
revision   5
package    sha256:b61fbe0b054e0a2b42580ee99e08be22d8fe1d05a645651951d94aada3e2e302
artifact   sha256:8130a594b8bda4fae335ed807dcdaeb61c4b5448208f6f25719ae8f910f5183f
parent     run_c0e091bdadf6ade0e772f2a3dd8b7340 at start_ships
position   deploy (deploy/1, from approve on approved)
invocation inv_bb1f611bd6b72d3ac6f4ffaa08683f16 attempt 2 of 2 stuck
error      command.exit: exit code 1
stderr     cannot reach the api target
needs      a person: step deploy gave up and has no failed arrow since 2026-10-14T22:53:44Z, for 0s
resolve    spore resolve run_69521f33daea0013880016867d12b6ab --retry
           spore resolve run_69521f33daea0013880016867d12b6ab --complete ok
           spore resolve run_69521f33daea0013880016867d12b6ab --fail <message>
```

Bring the target back and try again:

```sh
rm down
spore resolve run_69521f33daea0013880016867d12b6ab --retry
```

```text
resolution applied (request ID req_a61cbd8ce8fb50db)
```

```text
2026-10-14T22:53:45Z  api-1.4  step deploy started, attempt 3
2026-10-14T22:53:45Z  api-1.4  deploy returned ok -> done
2026-10-14T22:53:45Z  api-1.4  completed at done
```

`--complete ok` would carry on as if the deploy had worked, and `--fail <message>`
would end the run as failed.

## Get told: `onIntervention`

You saw the stuck deploy because you were watching. `onIntervention` tells you when
you are not. `ship`'s [`settings.json`](../../examples/watch-a-run/ship/settings.json)
names an action of the workflow to run, and its `params`:

```json
"onIntervention": {
  "action": "notify",
  "params": { "service": { "$get": ["input", "service"] } }
}
```

`notify` is an ordinary `command` action. It writes a line to `notifications.log` in
the project folder, then shows a desktop notification with `osascript` on macOS or
`notify-send` on Linux:

```json
"notify": {
  "kind": "command",
  "argv": ["sh", "-c", "msg=\"$1: $RUNSPORE_RUN_ID stopped at $RUNSPORE_NODE_ID\"; echo \"$msg\" >> notifications.log; if command -v osascript >/dev/null; then osascript -e 'on run argv' -e 'display notification (item 1 of argv) with title \"spore\"' -e 'end run' \"$msg\"; elif command -v notify-send >/dev/null; then notify-send spore \"$msg\"; fi", "notify", { "$get": ["params", "service"] }],
  "effect": "unsafe"
}
```

When `deploy` got stuck above, the worker ran it once, and a notification said:

```text
api: run_69521f33daea0013880016867d12b6ab stopped at deploy
```

A run that fails tells you the same way. Reject `web`:

```sh
spore signal run_536a8063c8fa3da941b15c8b193833ce approval --outcome rejected
```

```text
2026-10-14T22:53:45Z  web-1.4  approve returned rejected -> stopped
2026-10-14T22:53:45Z  web-1.4  failed at stopped: ship.rejected: The release was rejected.
```

and the notification says `web: run_536a8063c8fa3da941b15c8b193833ce stopped at
stopped`.

**When it runs.** Each time a run enters `needs-intervention` (exit 4) or ends
`failed` (exit 1), the `spore worker` or `spore run` that moved it there runs the
action once; `spore run` waits for it before it exits. A run that stays stuck is not
told again, and one that is retried and gets stuck again is. A completed run, a run
waiting for a signal and a quarantined run tell nobody.

**What it gets.** `params` is a mapping over the run's `input`, `nodes` (the latest
result of each node) and `run` (`{"id", "tenant"}`); a command reads it in its `argv`
under `params`. On stdin it gets JSON:

| Field | Value |
| --- | --- |
| `run` | `{"id", "tenant"}` |
| `node` | The node the run is at; for a failed run, the node its error names, or null |
| `status` | `needs-intervention` or `failed` |
| `reason` | `stuck` (a step gave up with no `failed` arrow), `unknown` (an `unsafe` step's outcome is unknown) or `failed` |
| `error` | The error that stopped the run, or null |

It also gets `RUNSPORE_RUN_ID`, `RUNSPORE_NODE_ID` and `RUNSPORE_DB`, and runs from
the run's base folder, so it can call `spore status "$RUNSPORE_RUN_ID"` itself.

**When it fails.** What the action returns never changes the run. If it fails, it is
not retried: `spore events <run>` shows an `on-intervention.failed` event with its
error, and nothing else happens.

**Known limit.** Telling is at most once: if the worker stops before the action ends,
nobody runs it again. The run's status still says it needs you; `onIntervention` only
says so sooner.

### A notification on your phone

To be told away from the desk, post to a push service instead. With
[ntfy](https://ntfy.sh), the action sends the JSON it gets on stdin as the message:

```json
"notify": {
  "kind": "command",
  "argv": ["curl", "-fsS", "-H", "Title: a spore run needs you", "--data-binary", "@-", "https://ntfy.sh/<your-topic>"],
  "effect": "unsafe"
}
```

A Slack or Teams incoming webhook takes a JSON body of its own: have the action run a
small script that reads stdin and posts that body. `onIntervention` plays the part of
n8n's Error Workflow and Argo's `onExit`.

## Reference

- [spec/cli.md](../../spec/cli.md): §Progress for the worker's lines and their JSON
  form, §Runs that start runs for `list` and parents, §JSON output for what `status`
  reports.
- [spec/host.md](../../spec/host.md) §2, Telling a person: `onIntervention` in full.
- [spec/dot.md](../../spec/dot.md) §4: `onIntervention` in `settings.json`.
- [claude-step.md](claude-step.md): a workflow whose step is done by Claude Code.
