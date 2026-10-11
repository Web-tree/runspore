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
| `/c17 3` | a local browser page (`http://127.0.0.1:47917/view.html`, served by `python3 -m http.server`; it polls `state.json` and updates in place); the pane keeps one line |
| `/c17 4` | one line above the prompt with a `view` button; no pane |
| `/c17 5` | finer text drawing: Graphviz's curves in Braille dots, boxes and labels as real text |
| `/c17 6` | the chosen look: picture where Claude Code draws one, Braille dots elsewhere (decided by a `$.ui.blit` probe) |
| `/c17 next`, `prev`, `play`, `pause` | step through the moments (it plays every 3.5 s by default) |

Inside the pane (after ctrl+x tab or a click): `h`/`l` step, `p` play/pause, `1`–`4` switch.
`bun hooks/try-text.ts [cols] [moment]` prints the text drawing (`B=1` for the Braille one) in any terminal.

Measured on Claude Code 2.1.296 in herdr 0.9.1 inside kitty 0.48.0:

- Variant 2 shows its fallback line. herdr draws kitty graphics (on by default since 0.9.0), but Claude Code
  only draws an `Image` for a terminal that names itself kitty (>= 0.28) or ghostty, and herdr names itself
  `libghostty`. Debug log: `kittyGraphics=no (probe: graphics reply OK, terminal libghostty)`.
- `CLAUDE_CODE_FORCE_TERMINAL_IMAGES=1` (found in the binary, not documented) makes it draw: screenshot by the
  probe agent, log `kittyGraphics=yes (env: CLAUDE_CODE_FORCE_TERMINAL_IMAGES)`. Plain kitty draws it as is.
- Variant 6's probe: `$.ui.blit` on the keyed Image answers
  `{"deny":"the Image draws its alt here: the terminal draws no placeholder images (probe: graphics reply OK, terminal libghostty)"}`
  in herdr, and `{}` with the variable set, so the mod can pick picture or dots without guessing.
