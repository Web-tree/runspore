# Checkpoint D: Write a workflow by talking

Comes after wave 6 (P8, R14, R25).

**Try:** `/ml-workflow:author` in Claude Code: describe a job, correct the numbered draft, say save, and get a folder that validates.

**Feedback asked for:** Does the interview ask the right things, and only those?

Answer from the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-d --outcome go` starts the next wave, which closes this issue.
- `spore signal <run> checkpoint-d --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
