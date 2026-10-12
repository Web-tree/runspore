# Checkpoint E: Run it from Claude Code

Comes after wave 8 (P2, P4, P5, R19, R20, R21, R22, R23, R24, R27).

**Try:** `/ml-workflow:start coding-loop`: your session follows the run and tells you when it needs you, and offers the decision right there. Try the interactive `implement` in a herdr pane, cancel a run, and kill your session and come back. The spore side passes C11's whole kill table.

**Feedback asked for:** Is following a run from your session the right default? Is anything missing in the decision flow?

Answer from the orchestrator folder:

- `work/build-0-2/bin/answer e go` starts the next wave.
- `work/build-0-2/bin/answer e changes "<change>" ["skills: <change>"]...` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
