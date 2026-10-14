# Checkpoint G: The destination

Comes after wave 11 (P9, R28).

**Try:** The full loop in real Claude Code survives killing the session or the machine at every row of the table. Docs and glossary match 0.2.

**Feedback asked for:** Lock the map; close the old skills-repo map.

Answer from the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-g --outcome go` closes this issue and ends the build.
- `spore signal <run> checkpoint-g --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
