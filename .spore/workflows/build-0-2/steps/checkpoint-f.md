# Checkpoint F: See it

Comes after wave 10 (P6, P7).

**Try:** Session-start line, `/ml-workflow:menu` pane with its rows, the run line above the prompt, and the live view (picture, or dots in herdr).

**Feedback asked for:** Layout, wording, what the run line shows.

Answer from the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-f --outcome go` starts the next wave, which closes this issue.
- `spore signal <run> checkpoint-f --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
