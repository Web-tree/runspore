# build-0-2: building spore 0.2 with spore

The 0.2 build plan is a Runspore workflow, run on the spore it builds. A fast lane
builds R1 and R3 first; from checkpoint 0 on, spore reads the workflow folders as
they are, and every checkpoint can move the build onto the newest spore.

| Workflow | Folder | What one run is |
| --- | --- | --- |
| `build-0-2` | `.spore/workflows/build-0-2` | The whole build: a fast lane, 15 waves, 8 checkpoints |
| `ticket` | `.spore/workflows/ticket` | One ticket: set up a worktree, implement, test, ship (PR, CI, squash-merge) |

A wave step starts one `ticket` run per ticket and finishes when every ticket's
issue is closed. A checkpoint step opens a GitHub issue assigned to you, saying what
to try, and waits for your answer. The step after a checkpoint closes its issue once
you answer `go`. The steps create the `checkpoint` and `ready-for-agent` labels they
use. The step scripts are `bin/run-wave` and `bin/ticket-step`; nothing runs around
spore.

## Run it

Use a checkout of its own, so tickets merging into `main` don't change the scripts
under the running build:

```sh
git -C ~/git/webtree/runspore worktree add --detach ~/git/webtree/build-0-2/orchestrator origin/main
cd ~/git/webtree/build-0-2/orchestrator
cargo install --locked --path crates/runspore-cli
spore start build-0-2 --key build-0-2           # once; the same key names the same run
spore worker                                    # in a pane of its own; Ctrl-C stops it, run it again to go on
spore list                                      # the build run's ID
```

The build uses the orchestrator's own database, `.spore/runspore.db`. Ticket
worktrees go to `~/git/webtree/build-0-2/<repo>-<ticket>`. A ticket run is started
with `--input '{"ticket": {"id", "issue", "title"}}'`; the `ticket` folder declares
that shape as its `inputSchema`, so `spore start ticket` without one is refused with
`input.invalid` and starts nothing.

## Answer a checkpoint

```sh
spore signal <run> checkpoint-c --outcome go
spore signal <run> checkpoint-c --outcome changes --data '{"changes": ["Errors should name the settings key", "skills: the menu needs a hint line"]}'
```

`changes` makes one ticket per change (runspore, or the skills repo with `skills:`),
builds them, then asks again. spore checks the answer against the checkpoint's
output schema in `.spore/workflows/build-0-2/settings.json`: a `changes` answer needs
at least one change, none of them empty. An answer that does not fit is refused,
with each error on its own line, and nothing is recorded; answer again. An answer
sent before the checkpoint waits is held until it does. Sending the same answer
twice is two answers, unless both carry the same `--id`.

## Move onto the newest spore

At a checkpoint: stop the worker (Ctrl-C), then

```sh
git pull
cargo install --locked --path crates/runspore-cli
spore worker
```

The newer spore goes on with the same database and the same run.

### Once: from the database per spore commit

Before this, `bin/build --upgrade` kept one database per spore commit, named in
`.spore/build-state/engine.json`. To move the running build onto the project
folder's database, at a checkpoint, with the worker stopped:

```sh
db="$(python3 -c 'import json; print(json.load(open(".spore/build-state/engine.json"))["db"])')"
for f in "" -wal -shm .files; do [ ! -e "$db$f" ] || mv "$db$f" ".spore/runspore.db$f"; done
rm -f .spore/build-state/engine.json .spore/build-state/answers.json
```

Then install the newest spore and run `spore worker` as above.

## See where it is

```sh
python3 work/build-0-2/track.py                 # JSON
python3 work/build-0-2/track.py --page out.html # the page, with the workflow drawn from its DOT
spore status <run>
```

## When something goes wrong

- A ticket step that can't recover (its tries ran out) is stuck: the ticket run
  waits, and the wave comments on the issue with the error and the commands that
  answer it. Fix the cause and run `spore resolve <run> --retry` for a new try,
  `--complete ok` if the step's work is done, or `--fail "<why>"` to end that run.
  Or finish the ticket by hand and close the issue.
- A ticket whose checks or CI stay red through three fixes in a row (the `test` and
  `ship` steps' loop limits) goes to the `stuck` step, which waits for you; the wave
  comments on the issue with the last failure. `spore signal <run> stuck --outcome
  keep-going` runs the checks again with a fresh count, `--outcome stop` ends that
  run. Or finish the ticket by hand and close the issue.
- If the build run itself fails, `spore start build-0-2 --key build-0-2:2` starts a
  new try (`:3` after that). Merged waves pass at once, and each checkpoint asks
  again.

## Test it

`work/build-0-2/test/e2e.sh` drives both workflows with plain `spore` commands, a
fake `gh` and a fake `claude`, through checkpoint B and an upgrade onto a newer
spore binary. It sets its own `RUNSPORE_DB`, so running it inside a ticket step
never touches the build's database.

## Gaps in spore this works around

Each one is a 0.2 ticket or a later map. The ticket deletes the workaround as part of
its own acceptance criteria, so the build uses each fix from the next upgrade on.

| Workaround here | Gap | Where it is fixed |
| --- | --- | --- |
| A wave step starts ticket runs and polls them | No child runs or parallel steps | later map (Temporal child workflows, n8n Execute Workflow) |
