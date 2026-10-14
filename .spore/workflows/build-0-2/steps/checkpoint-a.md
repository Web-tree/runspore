# Checkpoint A: Draw a workflow and run it

Comes after wave 2 (R1 and R3 came first, in the fast lane).

**Try:** Write a small `workflow.dot` + `settings.json` folder and run `spore validate` / `spore run` on it. Typos come back with file and line.

**Feedback asked for:** Is the DOT format pleasant to write? Are the error messages clear?

Answer from the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-a --outcome go` starts the next wave, which closes this issue.
- `spore signal <run> checkpoint-a --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
