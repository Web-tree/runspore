# C17 prototype: a live view of a run in the mod (throwaway)

For [C17 — A live view of a run in the mod](https://github.com/Web-tree/runspore/issues/40). Never merged:
this branch is the primary source.

A Claude Code mod that shows one scripted run of the C11 coding loop (`workflow/`, copied from
`docs/design/c11-reference-loop/coding-loop`) moving through 14 moments: a cut-off Claude try, three reds,
the loop running out, keep-going, green, approval. Each moment is what `spore status --json` would say,
including the fields C17 proposes adding (`position.from`, `invocation.maxAttempts`, `streak`).
Graphviz (`dot` on `PATH`) lays out `workflow.dot`; nothing else is needed.

Run it: `claude --plugin-dir prototype/c17-live-run-view`, then `/c17`.

| Command | What it does |
| --- | --- |
| `/c17` | opens the view |
| `/c17 1` | text drawing in the pane: Graphviz layout drawn in characters, files beside or below |
| `/c17 2` | Graphviz picture in the pane (kitty/Ghostty only; elsewhere its fallback line) |
| `/c17 3` | a local browser page (`/private/tmp/c17-live-run-view/view.html`, reloads every second); the pane keeps one line |
| `/c17 4` | one line above the prompt with a `view` button; no pane |
| `/c17 next`, `prev`, `play`, `pause` | step through the moments (it plays every 3.5 s by default) |

Inside the pane (after ctrl+x tab or a click): `h`/`l` step, `p` play/pause, `1`–`4` switch.
`bun hooks/try-text.ts [cols] [moment]` prints the text drawing in any terminal.

Measured on Claude Code 2.1.296 in herdr inside kitty: variant 2 shows its fallback line, not the picture.
