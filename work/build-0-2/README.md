# build-0-2: building spore 0.2 with spore

The 0.2 build plan is a Runspore workflow, run by spore 0.1 until 0.2 can run it.

| Workflow | Folder | What one run is |
| --- | --- | --- |
| `build-0-2` | `.spore/workflows/build-0-2` | The whole build: 11 waves, 7 checkpoints |
| `ticket` | `.spore/workflows/ticket` | One ticket: set up a worktree, implement, test, ship (PR, CI, squash-merge) |

A wave step starts one `ticket` run per ticket and finishes when every ticket's
issue is closed. A checkpoint step opens a GitHub issue assigned to you, saying what
to try, and waits for your answer.

## Run it

Use a checkout of its own, so tickets merging into `main` don't change the scripts
under the running build:

```sh
git -C ~/git/webtree/runspore worktree add ~/git/webtree/build-0-2/orchestrator dogfood/build-0-2-workflow
cd ~/git/webtree/build-0-2/orchestrator
cargo install --locked --path crates/runspore-cli --root ~/.local/share/runspore-0.1   # a fixed spore 0.1
work/build-0-2/bin/build        # in a pane of its own; Ctrl-C stops it, running it again goes on
```

Ticket worktrees go to `~/git/webtree/build-0-2/<repo>-<ticket>`. The worker runs
up to 4 steps at once: a wave step plus up to 3 tickets.

## Answer a checkpoint

```sh
work/build-0-2/bin/answer a go
work/build-0-2/bin/answer a changes "Errors should name the settings key" "skills: the menu needs a hint line"
```

`changes` makes one ticket per change (runspore, or the skills repo with `skills:`),
builds them, then asks again.

## See where it is

```sh
python3 work/build-0-2/track.py                 # JSON
python3 work/build-0-2/track.py --page out.html # the page, with the workflow drawn from its DOT
~/.local/share/runspore-0.1/bin/spore --db .spore/runspore.db list
```

## When something goes wrong

- A ticket run that fails is tried once more. After that the wave comments on the
  issue and waits; finish the ticket by hand and close the issue.
- If the build run itself fails, `work/build-0-2/bin/build --restart` starts a new
  try. Merged waves pass at once, and every checkpoint you answered `go` gets that
  answer again up front. The same command moves the build onto 0.2 once 0.2 reads
  workflow folders.

## Gaps in spore this works around

Each one is a 0.2 ticket or a later map; the workaround goes when it lands.

| Workaround here | Gap | Where it is fixed |
| --- | --- | --- |
| `bin/to-0.1` converts the DOT folders to 0.1 JSON | No DOT reader | R3 |
| `fix` and `fix_ci` are separate nodes, so each can read the output of the step before it | A step can't read why it was entered | R12 (`from`, `$get` default) |
| The step scripts `cd` into the worktree from their input | `cwd` is fixed per action | R25 |
| Over the visit limit the ticket run fails; the wave tries once more, then asks you | No limit arrow, no stuck step | R10, R11 |
| A wave step starts ticket runs and polls them | No child runs or parallel steps | later map (Temporal child workflows, n8n Execute Workflow) |
| Checkpoints are answered with `bin/answer`; output schemas are checked there, not by spore | No output schemas on signals | R13 |
