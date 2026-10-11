# C17 — A live view of a run in the mod

Design record of the prototype held on 2026-10-12 for the map ticket **C17 — A live view of a run in the mod** (Web-tree/runspore#40, map #5). It settles how the plugin's mod shows a run as it moves, inside a terminal Claude Code session. The view shows the workflow drawn from its DOT with the current node and the arrow just taken highlighted, beside the matching lines of `workflow.dot`, `settings.json` and the step's instructions. **The mod draws a real Graphviz picture in a pane where Claude Code can draw pictures, and the same layout in Braille dots and text everywhere else.** While a session follows a run, one line above the prompt shows where the run is, and the full view opens when the person asks. Runspore gains a few plain fields and one flag on `spore status --json`, and a way to read a source file from a run's pinned artifact. Everything else lives in the plugin.

Max asked for this after C11's browser prototype: "from the prototype, we can borrow the things with displaying of the workflow state with visuals and highlight text, and integrate something like this in our pluging for claude code as mod." The prototype is a real mod, run in Max's own setup (Claude Code 2.1.296 in herdr 0.9.1 inside kitty 0.48.0). It is on the never-merged branch [`prototype/c17-live-run-view`](https://github.com/Web-tree/runspore/tree/prototype/c17-live-run-view/prototype/c17-live-run-view). It plays one scripted run of the C11 coding loop through 14 moments (a cut-off Claude try, three reds, the loop used up, keep-going, green, approval) in six looks: a coarse text drawing, a picture, a browser page, a line above the prompt, a Braille-dot drawing, and the chosen picture-or-dots.

## Terms

- **Live view** — The plugin's pane showing one run: the drawing, the highlighted file lines and the run's facts (status, node, visit, try, loop count). It changes when the run moves. *Avoid:* dashboard, monitor, graph view.
- **Run line** — The one line above the prompt for each run the session follows, with a button that opens the live view. *Avoid:* status bar (Claude Code's own line under the prompt), banner.
- **Arrow just taken** — The arrow the run took into the node it is at: the previous node and the outcome it routed on. Kernel 0.2 calls the same thing `from` in the mapping context (C11 L1). *Avoid:* last edge, transition.
- **Dot drawing** — The live view's drawing made of characters: Graphviz's own layout, its curves plotted in Braille dots (2×4 per cell), boxes and labels as real text. *Avoid:* ASCII art, text graph.

## Why

The ticket asked four things: what draws the graph in a terminal pane (text layout, an image, or a local browser page the mod opens); where the run's position, last arrow, red count, visits and tries come from (`spore status`/`events --json`, and whether they need a field such as the last arrow); when the view opens and refreshes; and how it sits with C08's menu and C06's following session. Runspore stays generic: anything spore must add is a plain CLI field.

## Locked decisions

### L1. A real picture where Claude Code draws one, the dot drawing elsewhere (Max)

**Decision.** The live view draws the workflow with Graphviz as a picture (a mod `Image`) when the terminal can show it. Otherwise it draws the same layout as a dot drawing. The current node is filled yellow and the arrow just taken is orange, with its label, in both. The mod does not guess which one works. It draws the picture with a key, then asks Claude Code to swap that picture for itself (`$.ui.blit`). Claude Code refuses with a reason when the pane shows the picture's fallback text, and the mod then switches to the dot drawing for the rest of the session.

What the prototype measured (§Verified facts): herdr draws kitty graphics, but Claude Code 2.1.296 only draws an `Image` for a terminal that calls itself kitty or Ghostty. Herdr calls itself `libghostty`, so in Max's usual setup the picture shows only when Claude is started with `CLAUDE_CODE_FORCE_TERMINAL_IMAGES=1`. That variable is undocumented. Hence the fallback.

Max first answered the look question with "do you think we can render the graph to be able to be displayed in terminal? can you check that? I think it's possible." The check found herdr's picture support and Claude Code's opt-out, and it produced the dot drawing. Max then chose "picture, dots fallback".

Rejected:
- *Coarse text drawing (look 1): boxes and `│╱╲─` lines.* It works everywhere, but its lines crowd and break where curves bend. The dot drawing keeps Graphviz's curves at the same size.
- *A browser page (look 3).* It has the best picture, but it lives outside the terminal, and it needs a local server so the page can update without reloading. A first version reloaded every second and flashed (Max: "in the browser view I see the page flashing every second"). Nothing stops a later button from opening one; it isn't part of this design.
- *Picture only, with a one-line fallback.* That leaves herdr users, Max included, with no drawing.

### L2. One run line above the prompt; the live view on request (Q2 → A, Max)

**Decision.** While the session follows a run (C06 L3), the mod shows one run line above the prompt: workflow, node and visit, the arrow just taken, try `n/m`, the loop count (`red 2/3 in a row`), and "needs you" in orange when the run waits for a person. A `view` button (hotkey `v`) opens the live view. The view also opens from a **View** action on every row of C08's menu, for any unfinished run, followed or not. Nothing opens a pane by itself, so the person's screen changes only when they ask. Claude Code would also hold back an unasked pane below 144 columns.

Rejected: *the view opens by itself when a run starts or continues* (a pane appearing on its own, and only on wide terminals); *nothing until asked* (no sign of progress while following).

### L3. The facts come from `spore status --json`, with new generic fields (standard practice)

**Decision.** The view reads its facts from `spore status <runId> --json` and its files from the run's pinned artifact. CLI 0.2 adds these, all plain run facts with no Claude in them:

- `position.from`: `{"nodeId", "outcome"}` of the arrow just taken, or `null` at the start node. Spore derives it from the run's events (the result routed into the current activation), so the kernel's state is unchanged. It is the same value kernel 0.2 maps as `from` (C11 L1).
- `invocation.maxAttempts`: the action's automatic retry limit, beside the existing `attempt`. A person's `resolve --retry` can go past it (C07), so the plugin shows `try n/m` while `n ≤ m` and `try n (past the limit of m)` after.
- `streaks`: kernel 0.2's streaks (C13) with the package's limit added, `{"<nodeId>": {"outcome", "count", "limit"}}`. An entry is present only while a streak runs.
- `end`: `{"nodeId", "from"}` for a `completed` or `failed` run: the complete or fail node it ended at and the arrow into it. The kernel clears `position` at those nodes (kernel §state, *Enter*), so without `end` the view could not light the last node and arrow. Spore derives it from the events, like `position.from`.
- `artifactDigest`: the digest of the artifact the run pinned (C02 records it on the run).
- `status --no-probe`: a read with no side effects. Plain `status` probes every held async step (C03 §2), and a *dead* answer expires the attempt; a display that refreshes every second must not do that. The worker's sweep and `continue` keep probing as before.
- `spore inspect --run <runId> --file <path>`: prints one source file (`workflow.dot`, `settings.json`, an instructions file) from the run's pinned artifact (C02: `blob(digest)` holds the sources). The view draws and quotes these, never the workflow folder, which may have been edited, renamed or deleted since the run started, or never existed for a run started from a prebuilt artifact.

Visits (`position.visit`), tries (`invocation.attempt`) and status already exist. `spore events --json` is not needed for the view; event history stays in the map's fog.

Why settled, not asked: an engine's status call carrying the current state plus the transition that led there is common practice (Step Functions' execution history, Temporal's pending activity with `attempt` and `maximumAttempts`, Argo's node status).

### L4. Refresh by polling status while something shows, redrawing only on change (standard practice)

**Decision.** While a run line or the live view is visible, the mod runs `spore status <runId> --json --no-probe` once a second from a timer (`$.clock.every`). It redraws only when the fields it shows change. `revision` alone is not enough: it moves only on kernel turns, and store-level changes such as `cancel`, quarantine and a held attempt do not touch it (store §commit). With nothing visible it polls nothing. A run that reaches `completed`, `failed` or `cancelled` keeps its final picture in an open view and drops its run line.

Why settled: Kubernetes watches and status pages refresh this way, by comparing what they show. C05 measured `spore list` at 257–667 ms. A follow-up with a push channel (`spore status --wait-change`) can replace the poll later without changing the view.

### L5. The plugin's helper draws, with no system Graphviz (standard practice)

**Decision.** `claude-spore` (the plugin's Bun helper, C10) carries Graphviz compiled to WebAssembly (`@hpcc-js/wasm-graphviz`, as in C11's prototype) and an SVG-to-PNG renderer in WebAssembly (`@resvg/resvg-wasm`). It writes the picture and the layout (`plain` format) that the dot drawing uses, from the DOT text the run pinned (L3). The person installs nothing beyond the plugin. The prototype used the system `dot` only for speed. The display tweaks are the plugin's and render-only (C02: `label` is render-only): each arrow shows its `on`, an explicit label wins, end nodes get a double outline, and the highlight is added.

## Routine choices

- **Beside the drawing.** `workflow.dot` in full, with the current node's line highlighted yellow and the arrow just taken orange. The current node's block in `settings.json`, with its action's block above it. The first three lines of the step's instructions, with the full text on request (as C09). Side by side when the pane is 150 columns or wider, stacked otherwise; the pane scrolls.
- **Pictures are cached per moment.** The picture depends only on the run's artifact digest, the node and the arrow, so the helper writes it once per (artifact digest, node, arrow) under the plugin's data folder and the `Image` reads the file (`source: {file, format: 'png'}`).
- **The picture check runs once per session**, on the first picture drawn. Its answer holds until the session ends.
- **Several followed runs** each get their own run line, newest first.
- **How the mod knows a run is followed.** It sees Claude's Bash call `spore continue <runId> --wait` (C06's following command) in a `tool.call` hook. The run line goes when the run ends or the person cancels it.

## Specification

### CLI 0.2 — `status --json`

```json
{"format": "runspore.cli/0.2", "runId": "run_…", "status": "running", "revision": "9",
 "artifactDigest": "sha256:…",
 "position": {"nodeId": "implement", "activationId": "implement/3", "visit": 3,
              "from": {"nodeId": "test", "outcome": "red"}},
 "invocation": {"invocationId": "inv_…", "attempt": 1, "maxAttempts": 3, "state": "scheduled"},
 "streaks": {"test": {"outcome": "red", "count": 2, "limit": 3}},
 "end": null, "result": null, "quarantine": null}
```

`position.from` is `null` when the run is at its start node's first activation. `invocation.state` keeps its kernel values (`scheduled`, `unknown`, and C07's `stuck`). `invocation.maxAttempts` is the action's `retry.maxAttempts` (1 without retry). `streaks` is `{}` when no streak runs. `end` is non-null only for `completed` and `failed`, for example `{"nodeId": "done", "from": {"nodeId": "approve", "outcome": "approved"}}`.

`--no-probe` makes `status` read the store only: no probe runs and nothing is expired. Without it, `status` behaves as C03 says.

`spore inspect --run <runId> --file <path>` writes the named source file of the run's pinned artifact to stdout, exit 2 when the artifact has no such file. The exact `inspect` spelling is settled with `cli.md` 0.2 (map fog: "`spore inspect` output").

### Plugin (in `max-skills`, C10)

- The run line: an `AbovePrompt` render, one row per followed run, with a `view` button.
- The live view: a pane (`$.ui.open({ id: 'run-<runId>' })`) opened by the run line's button or the menu's View action. It holds the picture or the dot drawing, the file lines, and the facts line.
- `claude-spore draw --run <runId> [--at <node>] [--from <node>:<outcome>] --png <file> | --plain`: reads `workflow.dot` from the run's pinned artifact (`spore inspect`) and renders the display DOT with Graphviz WASM and resvg.
- The picture check: `$.ui.blit` on the keyed `Image`. A `deny` naming the alt switches to the dot drawing.

## Verified facts

- **Herdr draws pictures; Claude Code opts out.** Herdr 0.9.1 implements the kitty graphics protocol, on by default since 0.9.0 (`terminal.kitty_graphics`, herdr.dev/llms-full.txt). `kitten icat` showed the graph in a herdr pane (screenshot). Claude Code's debug log in herdr: `kittyGraphics=no (probe: graphics reply OK, terminal libghostty)`. With `CLAUDE_CODE_FORCE_TERMINAL_IMAGES=1` it logs `kittyGraphics=yes (env: CLAUDE_CODE_FORCE_TERMINAL_IMAGES)`, and a screenshot shows the picture in the herdr pane. Setting `TERM=xterm-kitty`, `TERM_PROGRAM=ghostty` or `kitty` changed nothing. In plain kitty 0.48.0, Claude Code draws the picture as is. The variable came from the Claude Code 2.1.296 binary, not its docs.
- **The mod can tell which one it got.** In herdr, `$.ui.blit` on the keyed picture answered `{"deny":"the Image draws its alt here: the terminal draws no placeholder images (probe: graphics reply OK, terminal libghostty)"}`. With the variable set it answered `{}`. The documented contract says the same ("an Image drawing its alt there").
- **A pane's elements in the terminal:** `Box`, `Text` (colour, background, bold), `Button` (with hotkeys), `Link`, `Image` (kitty/Ghostty) and `Raster` (cell grid). `Svg` is desktop only. A pane is a sidebar in a wide fullscreen terminal and a framed region above the prompt otherwise. (Claude Code 2.1.296 types; prototype screens.)
- **The dot drawing fits the reference loop** in about 55 columns by 30 rows, and the coarse text drawing in about 45 by 25. Both were measured with `bun hooks/try-text.ts` and in the pane.
- **A browser page under `file://` cannot fetch**, so a page that updates in place needs a local server. The prototype's page reloaded and flashed until one was added.

## Risks

- **Herdr users get dots unless they set an undocumented variable.** The lasting fix is on Claude Code's side: accept `libghostty`, or trust a graphics query answered `OK`. That is worth reporting to Anthropic. Until then the dot drawing carries it.
- **`CLAUDE_CODE_FORCE_TERMINAL_IMAGES` may change without notice.** The plugin doesn't set or document it as a feature. The picture check makes either outcome safe.
- **A big workflow** gets a dot drawing wider than the pane. The pane clips and scrolls it. The reference loop fits.
- **The helper grows** by the two WebAssembly modules (a few MB). This is accepted to avoid a system dependency.
- **A one-second poll per visible run** costs a `spore` process each second (no probes, `--no-probe`). Fine for a few runs; a push channel can replace it.

## Deferred

- A browser page as an extra on request (L1, rejected as the main look).
- Event history in the view (map fog: "Progress and evidence display in the mod beyond the menu").

## Changes to earlier records

- **C08, menu rows.** Every unfinished-run row gains **View**, which opens the live view.
- **C06 L3, following a run.** While the session follows a run, the mod shows its run line (L2). The one line Claude prints when `continue --wait` returns is unchanged.
- **Map, spec split.** C17 adds: `cli.md` 0.2 `status --json` gains `position.from`, `invocation.maxAttempts`, `streaks` with `limit`, `end` and `artifactDigest`, the flag `--no-probe`, and `spore inspect --run --file`; the plugin's spec gains the run line, the live view, `claude-spore draw` and the picture check.

## Review fixes (PR #42, Codex)

All six findings held against the specs and were fixed: redraw on any shown field, not `revision` (store-level changes don't bump it); files read from the run's pinned artifact (`artifactDigest`, `inspect --run --file`); `status --no-probe` for the poll (plain `status` probes held steps); `end` for terminal runs (the kernel clears `position`); `try n (past the limit of m)` after a manual retry; the example's invocation state is `scheduled`.
