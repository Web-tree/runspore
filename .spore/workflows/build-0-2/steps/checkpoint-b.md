# Checkpoint B: The coding-loop shape runs in a terminal

Comes after wave 3 (R8, R10, R11, R12, R13).

**Try:** `spore init`, then the C11 loop with stand-in scripts: three reds take the exhausted arrow to `tests_stuck`, a failing step pauses for you (`resolve --retry`), `implement` sees why it came back, and a rejection without a reason is refused.

**Feedback asked for:** Do the stuck pause, loop limits and <code>from</code> behave the way you expect, before any Claude work builds on them?

Answer from the orchestrator folder:

- `work/build-0-2/bin/answer b go` starts the next wave.
- `work/build-0-2/bin/answer b changes "<change>" ["skills: <change>"]...` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
