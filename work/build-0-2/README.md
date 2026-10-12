# build-0-2: building spore 0.2 with spore

The 0.2 build plan is a Runspore workflow, run on the spore it builds. A fast lane
builds R1 and R3 first; from checkpoint 0 on, spore reads the workflow folders as
they are, and every checkpoint can move the build onto the newest spore.

| Workflow | Folder | What one run is |
| --- | --- | --- |
| `build-0-2` | `.spore/workflows/build-0-2` | The whole build: a fast lane, 11 waves, 8 checkpoints |
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
a wave step plus up to 3 tickets.

## Answer a checkpoint

```sh
work/build-0-2/bin/answer a go
work/build-0-2/bin/answer a changes "Errors should name the settings key" "skills: the menu needs a hint line"
```

`changes` makes one ticket per change (runspore, or the skills repo with `skills:`),
builds them, then asks again.

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

- A ticket run that fails is tried once more. After that the wave comments on the
  issue and waits; finish the ticket by hand and close the issue.
- If the build run itself fails, `work/build-0-2/bin/build --restart` starts a new
  try in the same database. Merged waves pass at once, and every checkpoint you
  answered `go` gets that answer again up front.

## Gaps in spore this works around

Each one is a 0.2 ticket or a later map. The ticket deletes the workaround as part of
its own acceptance criteria, so the build uses each fix from the next upgrade on.

| Workaround here | Gap | Where it is fixed |
| --- | --- | --- |
| `fix` and `fix_ci` are separate nodes, so each can read the output of the step before it | A step can't read why it was entered | R12 (`from`, `$get` default) |
| The step scripts `cd` into the worktree from their input | `cwd` is fixed per action | R25 |
| Over the visit limit the ticket run fails; the wave tries once more, then asks you | No limit arrow, no stuck step | R10, R11 |
| A wave step starts ticket runs and polls them | No child runs or parallel steps | later map (Temporal child workflows, n8n Execute Workflow) |
| Checkpoints are answered with `bin/answer`; output schemas are checked there, not by spore | No output schemas on signals | R13 |
