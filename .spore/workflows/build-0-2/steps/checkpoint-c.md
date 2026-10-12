# Checkpoint C: Claude does a step

Comes after wave 5 (R4, R15, R18, P1, P3).

**Try:** From a plain terminal: `spore run coding-loop --input '{"task": …}'` with the real `claude-spore run` as `implement`. Kill it mid-step, run again, and the conversation resumes. Its answer is held to the schema.

**Feedback asked for:** Is the background Claude step good enough to build the rest of the plugin on?

Answer from the orchestrator folder:

- `work/build-0-2/bin/answer c go` starts the next wave.
- `work/build-0-2/bin/answer c changes "<change>" ["skills: <change>"]...` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
