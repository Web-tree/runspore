# Checkpoint 0: Switch the build to spore 0.2

Comes after the fast lane (R1, then R3): spore now reads a workflow folder
(`workflow.dot` + `settings.json`), so it can run this build without `to-0.1`.

**Try:** move the build onto spore from `main`, from the orchestrator folder:

1. Stop the worker with Ctrl-C.
2. `git pull`, then `cargo install --locked --path crates/runspore-cli`.
3. `spore signal <run> checkpoint-0 --outcome go`, with the build run's ID from `spore list`.
4. `spore worker`

The new spore goes on with the same database and the same run: wave 1 starts on it.

**Feedback asked for:** Did the folders validate and start as they are? Was
anything around spore still needed only because of 0.1?

Answer from the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-0 --outcome go` starts wave 1, which closes this issue.
- `spore signal <run> checkpoint-0 --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
