# Claude Code mods: what the official docs say

Checked on 2026-10-06 against code.claude.com (docs reference page "as of v2.1.290"; local CLI was 2.1.292). Sources are Anthropic's docs, Anthropic's changelog, and the TypeScript declarations the docs call "the complete reference". Anything marked **(inferred)** is my reading, not something a page says.

## Summary

1. Both pages cited by `docs/13` exist: [Mods overview][ov] and [Mods reference][ref] return HTTP 200 with those titles, no redirect. Mods shipped in Claude Code 2.1.287 on 2026-10-01 ("Added Claude Mods") and are on by default ([changelog][cl], [ov][ov-onoff]).
2. A mod is a plugin whose `hooks/hooks.json` lists one JS/TS ES module that exports `register(on, options)`. Its hooks run inside Claude Code's own process (installed mods share one worker thread) and reach the outside world only through the `$` mods API ([ov][ov-how], [ref][ref-files], [ts][ts-worker]).
3. The API has events for tools, prompts, turns, sessions, subagents, the interface, other mods, telemetry and every settings-hook event. Its namespaces cover UI, commands, tools, agents, model calls, prompt submission, files, processes, HTTP, MCP, timers, env and a 4 MiB store that all sessions share ([ref][ref-events], [ref][ref-api]).
4. Mods are not sandboxed. A process a mod starts runs outside the Bash sandbox, and deny rules don't cover a mod's own `$.fs` or `$.process` calls. A hook's own execution time is capped at 10 s, and only `$.store` and files outlive a session ([ov][ov-reach], [admin][ad-default], [ref][ref-limits], [ui][ui-state]).
5. A mod can make Claude do work. `$.prompt.submit` starts a turn when the session is idle, `$.agent.spawn` starts a background subagent, and a `turn.complete` hook sees each end, so a mod can hand off a step and learn when it finished. Every factual claim in docs/13's mods section checks out, but the note leaves out this capability ([api][api-submit], [ref][ref-api], [d.ts][dts]).

Source caveat: the guides document `$.agent.spawn` only as a name in a table. Its behaviour comes from the GitHub `claude-code.d.ts`, which says it was "Written by Claude Code 2.1.277" and carries the banner "EARLY ACCESS: this surface may change between releases without notice". The docs say the GitHub copy can be older than your build and to trust the copy Claude Code writes for your version ([ref][ref], [create][cr-types]). I didn't generate the 2.1.292 copy, because that needs a Claude Code session to load a mod.

## 1. Definition and packaging

- **What it is.** "A mod is a plugin that changes how Claude Code looks and behaves. It's made of JavaScript or TypeScript event handlers: Claude Code calls one when an event happens … and the handler can watch the event, change it, or take it over." [ov][ov]
- **Relation to plugins.** A mod is a plugin with a hooks module. The plugins overview lists "A hooks module … A plugin that has one is called a mod" as one plugin component. [plugins][plugins] A plugin "can hold all of them, so a mod can ship in the same plugin as a skill and an MCP server." [ov][ov-compare]
- **Naming.** On these pages, "hook" means a mod's handler. The shell, HTTP or prompt hooks configured in settings are "settings hooks". [ov][ov]
- **File layout** [ref][ref-files]:
  - `.claude-plugin/plugin.json` (required): the normal plugin manifest. "Mods add no required fields."
  - `hooks/hooks.json` (required): `"modules": ["./register.js"]`, an array with one path. It can also hold settings hooks under `hooks`. A missing or misspelled `modules` key means no mod. [ts][ts]
  - Hooks module (required): exports `register(on, options)`. Allowed extensions are `.js .mjs .cjs .jsx .ts .mts .cts .tsx`, and the file must be an ES module. `options` holds the manifest's `userConfig` values with defaults filled in.
  - `types/index.d.ts`, named by the manifest's `types` field: needed when the mod uses `$.state` or adds a namespace to the API. [manifest][manifest]
  - `*.test.ts` / `*.test.tsx`: tests that `claude plugin test` runs.
  - A mod runs before the mods it lists under `dependencies`, and gets their typed namespaces. [ev][ev-order], [create][cr-types]
- **Language and runtime.** "Claude Code loads `.js` and `.ts` files directly", so no Node.js, bundler or build step is needed. [create][cr] The module "has no Node.js APIs, no timer globals such as `setTimeout`, and no network or file access of its own". Standard JS and web APIs (`URL`, `TextEncoder`, `AbortController`, `crypto.subtle`) are available. [api][api-reach] Installed mods share one worker thread, the "hooks worker". [ts][ts-worker] The docs don't name the JavaScript engine (not documented).
- **Static-analysis rules.** Each `$` call must be written out in full (`$.fs.read(...)`), event names must be string literals, imports must be relative files inside the plugin (plus `claude-code`), and dynamic `import()` and `require` are not allowed. Claude Code "refuses to load a mod that uses the mods API in a way this command can't read." [create][cr-validate], [admin][ad-review]
- **Install and enable.**
  - From a marketplace: `/plugin install name@marketplace` or `claude plugin install name@marketplace`, then `/reload-plugins` if a session is open. [ov][ov-install]
  - For one session: `claude --plugin-dir <dir>`, which hot-reloads on save. The env var `CLAUDE_CODE_PLUGIN_DIRS` does the same for apps you can't pass a flag to. [ref][ref-settings]
  - Claude can write a mod into `~/.claude/dev-mods/<session-id>/`. It loads only after you approve hot reloading for that session. [create][cr-ask]
  - In the Agent SDK, plugins load through `plugins: [{ type: "local", path }]`. That page doesn't mention mods. [sdk-plugins][sdkplug]
  - Mods are on by default. Turn them off per plugin in `/plugin`, for one session with `--safe-mode`, or everywhere with `disableAllHooks`. A directory must be trusted before any mod loads there. [ov][ov-onoff], [ts][ts]
  - Anyone can run `claude plugin validate <dir>` to list a mod's `hooks:` and `calls:` without running it. [ov][ov-list]

## 2. API surface

### Events

All from [ref][ref-events] unless noted. Return shapes: `next(e)` passes the event on, `next({...e, x})` passes it on rewritten, and an object answers the event itself.

| Group | Events and what a hook can return |
|---|---|
| Tools | `tool.call`: `next(e)`, `{ deny }` or `{ result }`. `tool.check`: `{ decision: allow\|ask\|deny }`. `tool.describe`: `{ description, isDeferred }` |
| Prompts | `prompt.submit` (rewrite `text`, add `context`, or `{ drop }`), `prompt.fill`, `prompt.suggest`, `prompt.edit`, `prompt.compose` (`{ sections }`), `prompt.section` (`{ text }` or `null`), `prompt.context` (`{ blocks }`), `prompt.attachment`, `prompt.mention` (2.1.290+), `skill.prompt`, `attribution.text` |
| Commands and config | `command.run`, `command.describe`, `config.set`, `config.describe` |
| Turns | `turn.start` (`e.turnId`), `turn.step` (async generator: change `model` or `effort`, or answer without calling the model), `turn.complete` (`{ text }` shows a line under the answer) |
| Session | `session.start`, `session.end` (`e.reason`: `clear\|resume\|logout\|prompt_input_exit\|other`), `session.compact` (`{ skip }`), `session.receive` / `session.send`, `session.append` (rewrite a stored row), `session.attach` / `session.detach`, `session.measure` |
| Subagents | `agent.offer` (`{ isOffered: false }`), `agent.spawn` (choose `model`, or `{ deny }`) |
| Interface | `ui.render`, `ui.resolve`, `ui.press`, `ui.input`, `ui.select`, `ui.focus`, `ui.scroll`, `ui.close`, `ui.message`, `ui.fault` |
| Other mods | `plugin.register` (`{ refuse }`), `engine.create` (add an API namespace) |
| Telemetry | `telemetry.log`, `telemetry.mark` (only with the `{ to: 'collector' }` filter) |
| Settings-hook events | `classic.<Event>`, such as `classic.Stop` or `classic.SessionStart`. `e` is the settings hook's stdin JSON |
| API calls | Every `$` method is also an event (`fs.read`, `model.complete`, …) that mods earlier in the chain can observe, rewrite or refuse |

The changelog for 2.1.292 also adds `prompt.autocomplete` ([changelog][cl]).

### Commands, tools and agents a mod registers

- **Commands.** `$.command.register({ name, description, argumentHint, immediate })`, answered by a `command.run` hook. Returned `{ text }` "prints in the transcript and Claude reads it". `immediate: true` lets the command run while Claude is working. Built-in names are refused. [api][api-cmd] The overview says such a command "runs your function at once, with no Claude turn". [ov][ov]
- **Tools for Claude.** `$.tool.register({ name, description, inputSchema })`. Claude sees the tool as `mcp__<plugin>__<name>`, and a `tool.call` hook filtered to that name serves it. [api][api-tool]
- **Agent types.** `$.agent.register` defines a type named `<plugin>:<name>`, with its own system prompt and tools. An `agent.offer` hook can hide it from the model so only the mod spawns it. [ref][ref-api], [d.ts][dts]
- **Running existing tools and commands** [d.ts][dts]:
  - `$.tool.call` runs a tool through the hooks, the permission check and its dialog.
  - `$.tool.check` asks for the permission decision without running anything.
  - `$.command.run` runs a slash command "as if the person typed" it, once the session is idle.

### UI

- **Panes.** `$.ui.open({ id, title, focus, rows, columns, … })` opens a pane: a sidebar in a wide fullscreen terminal, otherwise a framed region above the prompt. A pane the mod opens without a user action waits until the terminal is at least 144 columns wide (110 after the user has opened it once). [ui][ui-open], [ref][ref-limits]
- **The band above the prompt** (`AbovePrompt`) is shared by all mods. [ui][ui-where]
- **Redrawing Claude Code's own UI.** A mod can redraw these render sites: `UserMessage`, `AssistantMessage`, `ToolUse`, `ToolResult`, `ToolGroup`, `CommandOutput`, `AskUserQuestion`, `Spinner`, `ToolProgress`, `TurnDuration`, `InfoNotice`, `SessionMode`, `PromptHint`. Some are terminal-only. [ref][ref-sites]
- **Elements**: `Box`, `Text`, `Button`, `Link`, `Code`, `Markdown`, `Input`, `Select`, `Client`, `Svg` (desktop only), `Raster` and `Image` (terminal only). [ref][ref-elements]
- **Other output** [api][api-show], [ref][ref-api]:
  - `$.ui.status(text)`: a line under the prompt that stays until you change it.
  - `$.ui.toast(text)`: about 4 s.
  - `$.ui.log(text)`: a dim transcript line that Claude doesn't read. `{ to: 'debug' }` sends it to the debug log instead.
  - `$.ui.ask(question, options)`: the question dialog.
  - Also `$.ui.notice`, `$.ui.copy`, `$.ui.selection`, `$.ui.blit`, and `$.audio.play` / `$.audio.speak`.
- **Not reachable.** The permission prompt is not a render site, so a mod "can't change what a prompt shows you". [ov][ov-reach]

### Tool-call interception

- **`tool.call`** fires for every tool call, "including calls a subagent makes and calls to MCP tools". A hook can [ev][ev-guard]:
  - observe it;
  - rewrite the arguments by passing changed arguments to `next`;
  - retry by calling `next(e)` again;
  - read the result after `await next(e)`, which includes `deny` and `isError`;
  - refuse with `{ deny }`, which Claude reads as the tool result;
  - answer with `{ result }`, in which case "no permission prompt appears and the tool doesn't run";
  - hold the call while it asks the user with `$.ui.ask`.
- **`tool.check`** runs after permission rules and `PreToolUse` hooks have decided, and can replace their `allow`/`ask`/`deny`. [ev][ev-check]
- **Order** [ev][ev-order]:
  - Managed-settings `PreToolUse` hooks run before any mod, and their block is final.
  - Other `PreToolUse` hooks run after the last mod calls `next`. A mod that answers without calling `next` stops them from running.
- **What a mod's approval overrides** [perms][perms]:
  - It can approve a call that an `ask` rule or a non-managed `PreToolUse` hook blocked.
  - In auto mode, a call the mod approves skips the classifier.
  - Deny rules hold over a mod only where the built-in guard loads (managed settings, or Team/Enterprise sign-in), and an admin can turn that off with `allowModsToOverrideDenyRules`. [admin][ad-default]
- **Auto mode and rewritten input.** If a hook changes a call's input after the classifier reviewed it, the call is denied with "a hook changed this call's input after the model wrote it". [ts][ts-autodeny]

### State and storage

[ui][ui-state], [ref][ref-limits], [api][api-reach]

| Where | How long it lasts | Limits |
|---|---|---|
| Module variable | Until the module reloads (on every save during development) | none stated |
| `$.state` (reactive, declared in `types/index.d.ts`) | Until the session ends or `/clear`, `/resume` or `/branch` runs. It survives a module reload | none stated |
| `$.store` (`get`, `set`, `delete`, `keys`) | A JSON file per plugin under `~/.claude/plugins/store/`, kept until the mod deletes it or no session uses the store for `cleanupPeriodDays` | 4 MiB of JSON in total. Every session on the machine shares it, and `get` then `set` "isn't atomic", so concurrent writes race |
| `$.fs.write` | A file | 4 MiB per file. It "isn't atomic", so another process can read a partly written file |

## 3. Reaching outside

All of these go through the mods API, "with the same permissions as the user running Claude Code". [api][api-reach]

- **Processes.**
  - `$.process.run(argv)` uses no shell. It resolves `{ exitCode, stdout, stderr }` whatever the exit code, and rejects if the program can't start or is still running at the timeout: 30 s by default, 10 min at most. [api][api-reach], [ref][ref-limits] The d.ts adds `{ cwd, env, stdin, timeoutMs }` options, and says "Git runs with repo hooks off" and "CLI only". It doesn't define "CLI only". [d.ts][dts]
  - `$.process.spawn` "streams a long-running command's output". Hooks on it are async generators. [api][api-reach], [ref][ref-events]
  - Whether a spawned child outlives the session: not documented.
  - So yes, a mod can run `spore` (inferred from the above).
- **Files.** `$.fs.read`, `write`, `list` (one level), `exists`, `stat`, `ancestors`. Relative paths resolve against the session's working directory, and each file is capped at 4 MiB. [api][api-reach], [ref][ref-api]
- **Network.** `$.http.fetch(url, init)` over http or https resolves `{ status, ok, headers, text }`. Org network policy covers it, but "doesn't cover a program the mod starts with `$.process.run`". [api][api-reach], [admin][ad-controls] The d.ts adds a `socketPath` option for Unix sockets. [d.ts][dts]
- **MCP.**
  - `$.mcp.call` calls a tool on a connected MCP server. `$.mcp.connect(server)` connects a server that the mod's own plugin manifest lists. [ref][ref-api]
  - The two sources disagree on permissions. The admin page says the call runs "under the session's permission rules" [admin][ad-review]. The 2.1.277 d.ts says "No permission prompt: the plugin's call, seen by the hooks above it, is the grant" [d.ts][dts].
- **Model calls.** `$.model.complete` makes a one-off call with no conversation history. `$.model.fork` asks one question over the current conversation. Both bill the user's plan or API key. [api][api-model]
- **Sandbox and permissions.**
  - The note's claim is **confirmed**: "Mods aren't sandboxed. If you turn on sandboxing, the sandbox isolates the Bash commands Claude runs, and a process that a mod starts runs outside it." [ov][ov-reach] The sandboxing page says the same. [sandbox][sandbox]
  - Deny rules and managed `PreToolUse` blocks apply to Claude's tool calls. They do not apply to a mod's own `$.fs` or `$.process` calls: "with `Read(.env)` denied, a mod can still read that file with `$.fs.read`". [admin][ad-default]
  - To restrict a mod's calls, an admin can keep it from loading, or put a policy mod earlier in the chain to handle the call events. [api][api-reach], [admin][ad-policy]

## 4. Lifecycle and limits

- **Load.** Installed mods load at session start, in a trusted directory. `session.start` fires "once for each loaded mod, before the first prompt, and again after a reload of that mod. Not after `/clear`, `/resume`, or `/branch`". Claude Code waits for that hook before the first prompt. [ref][ref-session], [api][api-cmdtool]
- **Reload.**
  - `/reload-plugins` reloads all plugins. [ref][ref-cmds]
  - `--plugin-dir` reloads on save, and `CLAUDE_CODE_PLUGIN_DIR_WATCH=1` does the same in long-running non-interactive sessions. [ref][ref-settings]
  - Mods Claude writes reload at the end of each turn that changes them. [create][cr-ask]
  - An installed plugin runs a cached copy for its version, so edits to it don't apply until you reinstall. [ts][ts]
- **Unload.**
  - Disabling or uninstalling the plugin unloads the mod. [ov][ov-onoff]
  - A mod that crashes the shared worker is unloaded ("it crashed the hooks worker"). After three crashes that can't be traced to one mod, every non-built-in mod is off for the session until `/reload-plugins`. [ts][ts-worker]
  - Anthropic can turn installed mods off remotely. [ts][ts-check]
- **When a hook fails.** A hook that throws, times out or returns the wrong shape before calling `next` is skipped, and the chain continues (fail open). A `.catch` handler can answer in its place, for example `{ deny }`, to fail closed. [ev][ev-fail]
- **Time and size limits** [ref][ref-limits]:
  - A hook's own execution time: 10 s, or 50 ms for `prompt.edit`. Time inside `next` or inside a mods API call doesn't count, except `$.clock.sleep`. "Claude Code skips a hook that exceeds a time limit." A wait inside `$.ui.ask` doesn't count, but "time spent awaiting a promise of your own does count". [ev][ev-hold]
  - `.catch` handler: 1 s.
  - All `session.end` hooks together: the SessionEnd budget, 1.5 s by default (`CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS` changes it). [hooks][hooks-end]
  - `$.process.run`: 30 s by default, 10 min at most.
  - `$.model.complete`: 1024 `maxTokens` by default.
  - `$.session.messages()`: the newest 4,096 entries.
  - Redraws: 10 per second (30 for some terminal sites).
- **Background and timers.**
  - "Work that outlives one event … runs on a timer you start from `session.start`" with `$.clock.every` or `$.clock.after`. "The timer's callback runs outside any event, so it keeps running between turns and doesn't start one." [api][api-bg]
  - A throwing callback goes to the debug log, and the timer runs again at the next interval. "Timers stop when the module reloads." [api][api-bg]
  - `next.signal` aborts long work when the event is abandoned. [api][api-stop]
- **Session end, `/clear`, resume, branch, compaction** [ui][ui-reload], [ts][ts-lost]:
  - `session.end` fires on exit and on `/clear`, `/resume` and `/branch`. [ref][ref-session]
  - Those three commands reset `$.state` to defaults, and `session.start` doesn't fire again. `classic.SessionStart` does fire, with `source` set to `clear`, `resume` or `fork`, so a mod reloads saved values from `$.store` there.
  - Compaction "doesn't reset `$.state`", and `classic.SessionStart` also fires after it. A `session.compact` hook can veto compaction with `{ skip }`. [ref][ref-session]
  - Module variables are reset only by a module reload (inferred from the state table). [ui][ui-state]
  - What happens to running timers and spawned processes when the session process exits: not documented.

## 5. Availability

| Where Claude Code runs | Hooks run | Drawing appears |
|---|---|---|
| `claude` in a terminal, including editor terminals and the JetBrains plugin | Yes | Yes |
| Desktop app Code tab (not WSL) | Yes | Yes, except terminal-only elements |
| Desktop app WSL session | No ("plugins aren't available in WSL sessions") | No |
| VS Code extension chat panel | Yes | No |
| `claude -p` and the Agent SDK | Yes | No |
| Remote Control from claude.ai or mobile | Yes, on your machine | In the terminal on your machine |
| Cloud session | Yes, "for a plugin that reaches the cloud session" | No |

Sources: table from [ov][ov-where]; the WSL limit is also on [desktop-wsl][wsl]. The cloud page says plugins enabled only in a repo's `.claude/settings.json` or in user settings are not installed in cloud sessions [cloud][cloud].

- **Minimum version.** Terminal: 2.1.287. The Desktop app's bundled Claude Code: 2.1.286. [ov][ov-onoff] The admin page says "on by default in Claude Code v2.1.286 and later". [admin][ad]
- **Headless limits.**
  - A mod Claude writes can't load in `claude -p` or `dontAsk` mode, because nobody can approve it. [create][cr-skip]
  - `$.ui.ask` rejects in `claude -p`. [ev][ev-hold]
  - In `-p`, load errors go to stderr. [ts][ts]
  - Whether `$.prompt.submit` or `$.agent.spawn` work in `-p` or the SDK: not documented.
- **Stability label.**
  - The mods pages carry no beta, experimental or preview label.
  - They mention an earlier "early access" period: `CLAUDE_CODE_ENABLE_FUNCTION_HOOKS` is ignored from 2.1.287. [ov][ov-onoff]
  - They warn that "the events and methods can change between releases". [create][cr-types]
  - The GitHub d.ts (2.1.277) header still says "EARLY ACCESS: this surface may change between releases without notice". [d.ts][dts]
- **Org controls.** `allowManagedModsOnly`, `allowManagedHooksOnly`, `disableAllHooks`, `disableSideloadFlags`, and `prependPlugins` / `appendPlugins`. On managed or Team/Enterprise machines the built-in guard `sec-default@builtin` runs first. [admin][ad], [ref][ref-settings]

## 6. Can a mod make Claude do work, and learn when it finished?

Yes. The docs give a mod several ways to start work in the running session, not just react to it.

**Starting work**

- **Start a main-loop turn.** "When a background job finds something that needs Claude's attention, it can start a turn by submitting a prompt with `$.prompt.submit({ text })`. Claude reads the text after a sentence that names your mod as the sender. To send it as the user's own words … add `asUser: true`. The call waits until the session is idle and then starts a new turn. It resolves when that turn starts, so don't `await` it in a handler that runs while Claude is working." [api][api-submit], [ref][ref-api]
  - The admin page lists `$.prompt.submit` as able to "submit a prompt, and can send it as the user's own words". [admin][ad-review]
  - The overview: a mod can "submit a prompt as if you had typed it". [ov][ov-reach]
- **Start a subagent.** The reference lists `$.agent` with `register`, `spawn` and `list` [ref][ref-api]. The behaviour comes from the 2.1.277 d.ts [d.ts][dts]:
  - `$.agent.spawn({ prompt, subagentType?, description?, model?, name?, cwd? })` "Spawns a subagent … the same call the engine makes when the Agent tool starts one".
  - It runs "in the background", and resolves `{ model, agentId }` "once the subagent started (its answer is its `turn.complete`)", or `{ deny }`.
  - The d.ts example registers a hidden agent type, spawns it from a tool hook, and returns the subagent's answer as the tool's result.
  - The changelog records fixes for "a mod's start-up prompt, command or subagent being queued a second time", which confirms mods queue all three. [changelog][cl]
- **Run a slash command as if typed.** `$.command.run`, queued until idle. [d.ts][dts]
- **Message another session or a subagent.** `$.session.send({ to, text })`, "the same delivery the SendMessage tool makes". It resolves when the message is queued, with `isDelivered`. [api][api-send]
- **Cancel the running turn.** `$.turn.abort({ turnId })`. [ref][ref-api], [d.ts][dts]
- **Not session work.** `$.model.complete` and `$.model.fork` are side calls to a model, outside the conversation. [api][api-model]

**Learning that the work finished**

- **`turn.complete`** fires when a turn ends, including an interrupted one (`e.isAborted`). It carries `e.answer` (Claude's final text), `e.durationMs` and `e.usage`. "A subagent's turn fires it with `e.agentId` set." [ev][ev-turn] The d.ts adds `reason: answer|aborted|refusal|error`. [d.ts][dts]
- **`turn.start`** gives `e.turnId`, which `turn.step` and `turn.complete` carry too. [ev][ev-turn]
- **`classic.Stop`** fires when Claude finishes responding. [ev][ev-classic]
- **Matching a turn to the prompt that started it** is not documented. `turn.start` carries the text and `turnId` but no origin, and `$.prompt.submit` resolves only when the turn starts. [d.ts][dts] A mod would match by text, or by "the next `turn.start` after my submit" (inferred).
- **A registered tool as the report.** A mod can register a tool such as `mcp__<plugin>__complete_step` that Claude calls with structured input. Its `tool.call` hook can then run `spore signal` through `$.process.run`. Each piece is documented (`$.tool.register`, the `tool.call` answer, `$.process.run`); the combination is inferred. [api][api-tool], [api][api-reach]
- **Waiting without a turn.** A timer callback "keeps running between turns and doesn't start one", so it can poll `spore status <runId> --json` and show progress with `$.ui.status` (inferred use of documented calls). [api][api-bg]

**Constraints to plan around (documented)**

- A submitted prompt waits until the session is idle.
- A hook's own execution time is 10 s. Time spent inside `$.process.run` (up to 10 min) doesn't count, but polling with `$.clock.sleep` does.
- Timers die on module reload.
- `$.state` resets on `/clear`, `/resume` and `/branch`.
- `$.store` races across sessions.
- In `-p` there is nobody for `$.ui.ask`.
- All of the above from [ref][ref-limits], [api][api-bg], [ui][ui-state], [ev][ev-hold].

**How this could fit Runspore (inferred)**

A small mod could:

1. Run `spore run` with `$.process.run`, or poll `spore status` on a timer.
2. On a step that needs Claude, call `$.prompt.submit` (or `$.agent.spawn` for a background subagent) with the step's instructions.
3. Expose a `complete_step` tool, or watch `turn.complete`, and translate the result into `spore signal <runId> <name> --outcome … --data …`.

Durability stays in `spore`. The mod is a session-scoped adapter.

## 7. Claims in docs/13 "Claude Code mods" section

| # | Claim | Verdict | Source |
|---|---|---|---|
| 1 | Official docs describe mods as JavaScript or TypeScript hook functions | Confirmed | [ov][ov] |
| 2 | …with UI panes | Confirmed | [ui][ui-open] |
| 3 | …commands | Confirmed | [api][api-cmd] |
| 4 | …and tool-call interception | Confirmed | [ev][ev-guard] |
| 5 | They can coexist with skills and MCP servers | Confirmed: "a mod can ship in the same plugin as a skill and an MCP server" | [ov][ov-compare] |
| 6 | Availability varies by surface | Confirmed | [ov][ov-where] |
| 7 | The documented Desktop WSL session does not load plugins | Confirmed | [ov][ov-where], [wsl][wsl] |
| 8 | Use the mod for progress, ready actions, evidence links, approvals, cancellation, resumption | Design choice, not a doc claim. Panes, buttons, commands, `$.ui.ask` and timers make it feasible | [ui][ui], [api][api] |
| 9 | Use an MCP or CLI bridge for start, signal, status, manual actions | Design choice. A separate MCP server is optional: a mod can run the CLI itself (`$.process.run`) and expose its own tools (`$.tool.register`) | [api][api-reach], [api][api-tool] |
| 10 | A supervised or deliberately launched worker owns the durable loop | Design choice. The docs support it: timers stop on reload, hooks are time-limited, and a crashing mod is unloaded | [api][api-bg], [ref][ref-limits], [ts][ts-worker] |
| 11 | Don't keep authoritative state in mod variables, the small store, or ordinary file writes | Supported: variables reset on reload, `$.store` is 4 MiB, shared, non-atomic and expires after `cleanupPeriodDays`, and `$.fs.write` isn't atomic | [ui][ui-state], [ref][ref-limits] |
| 12 | The API and hook time budgets are not a durable worker contract | Supported: 10 s hook budget, and "events and methods can change between releases" | [ref][ref-limits], [create][cr-types] |
| 13 | Mods and hooks are not the sole authorization boundary | Supported: mods aren't sandboxed and can approve calls that `ask` rules or non-managed `PreToolUse` hooks would stop | [ov][ov-reach], [perms][perms] |
| 14 | Mod docs warn that processes a mod starts are outside Claude's Bash sandbox | Confirmed, nearly verbatim | [ov][ov-reach], [sandbox][sandbox] |
| 15 | Mod behaviour can affect approvals | Confirmed: `tool.check` can approve, ask or deny, `{ result }` skips the prompt, and auto mode skips the classifier for mod-approved calls | [ev][ev-check], [admin][ad-default] |
| 16 | Guard hooks should fail closed when policy checks fail | Doc-backed advice. The default is to skip the hook (fail open), and the docs show `.catch` returning `{ deny }` to fail closed | [ev][ev-fail] |
| 17 | Merge and deploy credentials stay in an external broker | Design, not in the docs | n/a |
| 18 | In the SDK, `allowedTools` auto-approves rather than limiting which tools exist | Confirmed: "Any other tool not listed in `allowed_tools` is still available to Claude" | [sdk-perms][sdkperm] |
| 19 | Earlier approvals can skip `canUseTool` | Confirmed: `canUseTool` is the last step, and bare `allowedTools` entries trigger the `CLAUDE_SDK_CAN_USE_TOOL_SHADOWED` warning | [sdk-perms][sdkperm] |
| 20 | `PreToolUse` checks complement broker enforcement | Design. Caveat from the docs: a mod can override a non-managed `PreToolUse` block, and a mod that answers `tool.call` stops it from running | [ev][ev-order], [perms][perms] |
| 21 | A session ID is a resume hint, not a workflow commit | Design, consistent with "Sessions persist the conversation, not the filesystem" | [sdk-sessions][sdksess] |
| 22 | SDK transcript mirroring is best-effort and can drop batches | Confirmed: "Mirror writes are best-effort … drops the batch, and continues" | [sdk-storage][sdkstore] |
| 23 | Session resume does not restore the repository | Confirmed | [sdk-sessions][sdksess] |
| 24 | File checkpoints omit shell edits and most subagent edits | Confirmed (exception: a foreground `context: fork` skill) | [sdk-ckpt][sdkckpt] |
| 25 | Require a present `structured_output` and validate locally, not exit 0 or a success envelope | Confirmed: "A result can also end with subtype `success` but no `structured_output` … Treat that case as a failure". The examples validate locally. "Exit zero" isn't discussed | [sdk-struct][sdkstruct] |
| 26 | Schema repair reshapes evidence without repeating effects, and new tool work uses the broker | Design, not in the docs | n/a |

Nothing in the section is wrong. Its gap is that it describes mods as reactive only. The documented `$.prompt.submit`, `$.agent.spawn`, `$.tool.register` and `$.process.run` let a mod drive work in the session (section 6).

[ov]: https://code.claude.com/docs/en/plugins/mods/overview
[ov-how]: https://code.claude.com/docs/en/plugins/mods/overview#how-a-mod-works
[ov-install]: https://code.claude.com/docs/en/plugins/mods/overview#install-or-update-a-mod
[ov-reach]: https://code.claude.com/docs/en/plugins/mods/overview#what-a-mod-can-reach
[ov-list]: https://code.claude.com/docs/en/plugins/mods/overview#list-what-a-mod-does-before-you-install-one
[ov-onoff]: https://code.claude.com/docs/en/plugins/mods/overview#turn-mods-on-or-off
[ov-where]: https://code.claude.com/docs/en/plugins/mods/overview#where-mods-run
[ov-compare]: https://code.claude.com/docs/en/plugins/mods/overview#compare-mods-settings-hooks-skills-and-mcp-servers
[ref]: https://code.claude.com/docs/en/plugins/mods/reference
[ref-files]: https://code.claude.com/docs/en/plugins/mods/reference#files
[ref-events]: https://code.claude.com/docs/en/plugins/mods/reference#events
[ref-session]: https://code.claude.com/docs/en/plugins/mods/reference#session
[ref-api]: https://code.claude.com/docs/en/plugins/mods/reference#mods-api-methods
[ref-sites]: https://code.claude.com/docs/en/plugins/mods/reference#render-sites
[ref-elements]: https://code.claude.com/docs/en/plugins/mods/reference#elements
[ref-limits]: https://code.claude.com/docs/en/plugins/mods/reference#limits
[ref-settings]: https://code.claude.com/docs/en/plugins/mods/reference#settings-and-environment-variables
[ref-cmds]: https://code.claude.com/docs/en/plugins/mods/reference#commands
[cr]: https://code.claude.com/docs/en/plugins/mods/create
[cr-ask]: https://code.claude.com/docs/en/plugins/mods/create#ask-claude-for-a-mod
[cr-skip]: https://code.claude.com/docs/en/plugins/mods/create#sessions-that-skip-the-approval
[cr-types]: https://code.claude.com/docs/en/plugins/mods/create#get-the-types-for-your-build
[cr-validate]: https://code.claude.com/docs/en/plugins/mods/create#check-what-claude-code-reads-from-your-mod
[ui]: https://code.claude.com/docs/en/plugins/mods/interface
[ui-where]: https://code.claude.com/docs/en/plugins/mods/interface#pick-where-to-draw
[ui-open]: https://code.claude.com/docs/en/plugins/mods/interface#open-a-pane-at-the-right-time
[ui-state]: https://code.claude.com/docs/en/plugins/mods/interface#keep-state
[ui-reload]: https://code.claude.com/docs/en/plugins/mods/interface#load-a-saved-value-again-after-clear
[ev-turn]: https://code.claude.com/docs/en/plugins/mods/events#follow-a-turn
[ev-guard]: https://code.claude.com/docs/en/plugins/mods/events#guard-or-change-a-tool-call
[ev-hold]: https://code.claude.com/docs/en/plugins/mods/events#hold-a-tool-call-until-the-user-decides
[ev-check]: https://code.claude.com/docs/en/plugins/mods/events#approve-or-refuse-a-tool-call-before-the-user-is-asked
[ev-classic]: https://code.claude.com/docs/en/plugins/mods/events#hook-the-settings-hook-events
[ev-order]: https://code.claude.com/docs/en/plugins/mods/events#the-order-mods-run-in
[ev-fail]: https://code.claude.com/docs/en/plugins/mods/events#handle-a-hook-that-fails
[api]: https://code.claude.com/docs/en/plugins/mods/api
[api-cmdtool]: https://code.claude.com/docs/en/plugins/mods/api#add-a-command-or-a-tool
[api-cmd]: https://code.claude.com/docs/en/plugins/mods/api#add-a-command
[api-tool]: https://code.claude.com/docs/en/plugins/mods/api#add-a-tool
[api-model]: https://code.claude.com/docs/en/plugins/mods/api#call-a-model
[api-bg]: https://code.claude.com/docs/en/plugins/mods/api#run-work-in-the-background
[api-show]: https://code.claude.com/docs/en/plugins/mods/api#show-something-without-starting-a-turn
[api-submit]: https://code.claude.com/docs/en/plugins/mods/api#start-a-turn-from-a-background-job
[api-stop]: https://code.claude.com/docs/en/plugins/mods/api#stop-background-work
[api-send]: https://code.claude.com/docs/en/plugins/mods/api#send-and-receive-messages-between-sessions
[api-reach]: https://code.claude.com/docs/en/plugins/mods/api#reach-files-processes-and-the-network
[ts]: https://code.claude.com/docs/en/plugins/mods/troubleshoot
[ts-check]: https://code.claude.com/docs/en/plugins/mods/troubleshoot#check-whether-mods-can-load
[ts-worker]: https://code.claude.com/docs/en/plugins/mods/troubleshoot#it-crashed-the-hooks-worker
[ts-autodeny]: https://code.claude.com/docs/en/plugins/mods/troubleshoot#a-hook-changed-this-calls-input-after-the-model-wrote-it
[ts-lost]: https://code.claude.com/docs/en/plugins/mods/troubleshoot#a-value-resets-after-/clear-/resume-or-/branch
[ad]: https://code.claude.com/docs/en/plugins/mods/admin
[ad-default]: https://code.claude.com/docs/en/plugins/mods/admin#know-what-happens-by-default
[ad-controls]: https://code.claude.com/docs/en/plugins/mods/admin#know-which-controls-still-apply
[ad-review]: https://code.claude.com/docs/en/plugins/mods/admin#review-what-a-mod-can-do
[ad-policy]: https://code.claude.com/docs/en/plugins/mods/admin#enforce-a-policy-with-a-mod-of-your-own
[dts]: https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts
[cl]: https://code.claude.com/docs/en/changelog
[plugins]: https://code.claude.com/docs/en/plugins/overview
[manifest]: https://code.claude.com/docs/en/plugins/manifest-reference
[hooks-end]: https://code.claude.com/docs/en/hooks#sessionend-input
[perms]: https://code.claude.com/docs/en/permissions#extend-permissions-with-hooks
[sandbox]: https://code.claude.com/docs/en/sandboxing
[wsl]: https://code.claude.com/docs/en/desktop-wsl
[cloud]: https://code.claude.com/docs/en/cloud-environments#what-carries-over-from-your-setup
[sdkplug]: https://code.claude.com/docs/en/agent-sdk/plugins
[sdkperm]: https://code.claude.com/docs/en/agent-sdk/permissions
[sdksess]: https://code.claude.com/docs/en/agent-sdk/sessions
[sdkstore]: https://code.claude.com/docs/en/agent-sdk/session-storage
[sdkckpt]: https://code.claude.com/docs/en/agent-sdk/file-checkpointing
[sdkstruct]: https://code.claude.com/docs/en/agent-sdk/structured-outputs
