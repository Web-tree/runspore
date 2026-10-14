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
to try, and waits for your answer.

## Run it

Use a checkout of its own, so tickets merging into `main` don't change the scripts
under the running build:

```sh
git -C ~/git/webtree/runspore worktree add --detach ~/git/webtree/build-0-2/orchestrator origin/main
cd ~/git/webtree/build-0-2/orchestrator
work/build-0-2/bin/build --upgrade   # in a pane of its own; Ctrl-C stops it
work/build-0-2/bin/build             # goes on after a stop
```

`--upgrade` checks the folder out at `origin/main` (or the commit you name),
installs that commit's spore into `~/.local/share/runspore-build/<commit>`, and
starts the run on it in a database of its own. Ticket worktrees go to
`~/git/webtree/build-0-2/<repo>-<ticket>`. The worker runs up to 4 steps at once:
a wave step plus up to 3 tickets. A ticket run is started with
`--input '{"ticket": {"id", "issue", "title"}}'`; the `ticket` folder declares that
shape as its `inputSchema`, so `spore start ticket` without one is refused with
`input.invalid` and starts nothing.

## Answer a checkpoint

```sh
work/build-0-2/bin/answer a go
work/build-0-2/bin/answer a changes "Errors should name the settings key" "skills: the menu needs a hint line"
```

`changes` makes one ticket per change (runspore, or the skills repo with `skills:`),
builds them, then asks again.
spore checks the answer against the checkpoint's output schema in
`.spore/workflows/build-0-2/settings.json`: a `changes` answer needs at least one
change, none of them empty. An answer that does not fit is refused, with each error
on its own line, and nothing is recorded; answer again.

## Move onto the newest spore

At a checkpoint: stop the build (Ctrl-C), answer, then `work/build-0-2/bin/build
--upgrade`. The new run passes the merged waves at once and gets every `go` you
gave up front. It prints the commit it came from; `build --upgrade <that commit>`
goes back to it and its database. Checkpoint 0 is the first of these: from there on
the build runs on a spore built from this repository's `main`.

## See where it is

```sh
python3 work/build-0-2/track.py                 # JSON
python3 work/build-0-2/track.py --page out.html # the page, with the workflow drawn from its DOT
python3 -c 'import json; print(json.load(open(".spore/build-state/engine.json")))'   # which spore, which database
```

## When something goes wrong

- A ticket step that can't recover (its tries ran out) is stuck: the ticket run
  waits, and the wave comments on the issue with the error and the commands that
  answer it. Fix the cause and run `spore --db <db> resolve <run> --retry` for a new
  try, `--complete ok` if the step's work is done, or `--fail "<why>"` to end that
  run. Or finish the ticket by hand and close the issue.
- A ticket whose checks or CI stay red through three fixes in a row (the `test` and
  `ship` steps' loop limits) goes to the `stuck` step, which waits for you; the wave
  comments on the issue with the last failure. `spore --db <db> signal <run> stuck
  --outcome keep-going` runs the checks again with a fresh count, `--outcome stop`
  ends that run. Or finish the ticket by hand and close the issue.
- If the build run itself fails, `work/build-0-2/bin/build --restart` starts a new
  try in the same database. Merged waves pass at once, and every checkpoint you
  answered `go` gets that answer again up front.

## Gaps in spore this works around

Each one is a 0.2 ticket or a later map. The ticket deletes the workaround as part of
its own acceptance criteria, so the build uses each fix from the next upgrade on.

| Workaround here | Gap | Where it is fixed |
| --- | --- | --- |
| A wave step starts ticket runs and polls them | No child runs or parallel steps | later map (Temporal child workflows, n8n Execute Workflow) |
