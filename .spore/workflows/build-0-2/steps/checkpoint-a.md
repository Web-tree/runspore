# Checkpoint A: Draw a workflow and run it

Comes after wave 2 (R1, R3).

**Try:** Write a small `workflow.dot` + `settings.json` folder and run `spore validate` / `spore run` on it. Typos come back with file and line.

**Feedback asked for:** Is the DOT format pleasant to write? Are the error messages clear?

Answer from the orchestrator folder:

- `work/build-0-2/bin/answer a go` starts the next wave.
- `work/build-0-2/bin/answer a changes "<change>" ["skills: <change>"]...` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
