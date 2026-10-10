# C10 — Where the plugin lives and how it gets spore

This is the design record of the grill held on 2026-10-11 for the map ticket **C10 — Where the plugin lives and how it gets spore** (Web-tree/runspore#15, map #5). It settles which repository ships the Claude Code plugin, how a person ends up with a `spore` that works with it, the version check between the two, and the names earlier tickets left provisional.

**Plugin side.** The plugin is `ml-spore` in Max's `max-skills` marketplace (`Max-Levitskiy/skills`), next to his other plugins. Its helper program `claude-spore` is a Bun script in the plugin's `bin/`. The plugin's own spec, the Claude part of this map's destination, lives beside it in `max-skills`, so this repository names no Claude at all.

**Runspore side.** The person installs `spore` themselves. Each runspore release publishes ready-built binaries with checksums, plus a one-line installer; `cargo install` also works. When `spore` or Bun is missing or too old, the plugin says so, shows the install or update line, and runs it only if the person says yes. The plugin uses the person's own `spore` whenever it is new enough.

There is no visual.

## Terms

- **Plugin** — The Claude Code package that ships the menu, the step runner and the authoring interview. It is installed from a marketplace. *Avoid:* extension, addon; mod (a mod is one part of it, the hooks module).
- **Helper** — The plugin's own program, `claude-spore`, which runs Claude steps, checks workflows and hands in answers. `spore` never knows about it. *Avoid:* wrapper, shim.
- **Oldest supported spore** — The lowest `spore` version the plugin works with, stated in the plugin. *Avoid:* pinned version, required version.

## Why

The ticket asked:
- which repository ships the plugin: this one, or the `max-skills` marketplace;
- how a person ends up with a `spore` binary that matches the plugin: bundled in the plugin, downloaded on first use, `cargo install`, or a package manager;
- what the version handshake between the plugin and the binary is.

Earlier tickets also left names here:
- **C07:** `claude-spore run|start|check|probe|watch`, the actor label `person`, the result file path, and the frontmatter key list.
- **C08:** `/spore`, `claude-spore stop`, `spore init` and exit code 7.
- **C09:** the skill name (`spore-author`), the command forms `new` and `change <name>`, a `references/` file, the default model and the tool sets.
- **C12:** `claude-spore report`, `outputSchemas`, the `.answers` and `.facts.json` files, and `output.missing`.
- **C14:** the node parameter names, `step show`, and the closed key set.

## Locked decisions

### L1. The plugin lives in `max-skills` (Q1 → B, Max)

**Decision.** The plugin is a folder in `Max-Levitskiy/skills` (`plugins/ml-spore`), listed in its `max-skills` marketplace with a relative `./plugins/ml-spore` source like the other six plugins. People install it with `/plugin install ml-spore@max-skills`. Plugin releases follow that repository's own release flow and version numbers. They are not tied to runspore's.

**Rejected.**
- *A — in this repo, which also serves as its own marketplace.* My recommendation. One pull request would change a format in `spore` and in the plugin at once, tested together under one version. Max chose to keep his plugins in one collection. The coupling is handled by the version check (R3) instead.
- *C — code here, and `max-skills` also lists it pinned to a release (`git-subdir` source).* Lost with A.

**Consequence.** The plugin and `spore` release separately. That is why the version check exists (R3), and why the helper (L3) and the profile spec (R4) move to `max-skills` too.

### L2. The person installs `spore`; the plugin offers the install line (Q2 → text, Max)

Max's answer: "plugin say runspore is missed and suggest to install."

**Decision.** The plugin carries no `spore` and downloads none by itself.
- **Missing.** When `spore` is not on the path, the plugin says so in one line, shows the one-line install command, and runs that command only if the person says yes.
- **Too old.** The same thing happens with the update command when the person's `spore` is older than the oldest supported spore (R3).

**Rejected.**
- *A — the plugin downloads the exact matching `spore` on first use, checked against a fingerprint it ships.* My recommendation. It means one install step and versions that always match. Max chose an explicit install the person agrees to.
- *B — the plugin carries `spore` for every platform.* Lost because every plugin update would fetch tens of MB of programs and bloat the repository.

### L3. The helper is a Bun script in the plugin (Q4 → A)

**Decision.** `claude-spore` is a `#!/usr/bin/env bun` script in the plugin's top-level `bin/`, with its code in the plugin's `src/`. This is the same pattern as `ml-agent-config`'s `bin/agent-config`. Bun is a second prerequisite, checked like `spore`: when it is missing or too old, the plugin shows a message with the install line and runs it only on a yes.

**Rejected.**
- *B — a Rust program built in the runspore repository and released with `spore`.* It would need no Bun, but it would split the plugin across two repositories again, which undoes L1.

**Consequence.** Every `claude-spore` reference in C07, C08, C09, C12 and C14 means this script. The worker runs it as a plain `command` argv (`["claude-spore", "run"]`), so it has to be on the worker's path (R5).

## Routine choices

- **R1. Which `spore` the plugin uses (Q3 → B, Max).** The plugin uses the person's own `spore`, the first one on the path, whenever it is at least the oldest supported spore. There is no second copy. *My recommendation was A, the plugin's own copy, which L2 has since made moot.*
- **R2. Install channels (settled as standard practice).**
  - Each runspore release on GitHub publishes `spore` for macOS (arm64, x86_64) and Linux (x86_64, arm64) as archives, with a `SHA256SUMS` file.
  - A one-line installer (`install.sh`) downloads the right archive, checks it against `SHA256SUMS`, and puts `spore` in `~/.local/bin`.
  - `cargo install --git https://github.com/Web-tree/runspore runspore-cli` also works.
  - A Homebrew tap is deferred.
  - The plugin's message shows the `install.sh` line.
- **R3. The version check (settled as standard practice).**
  - The plugin states its oldest supported spore, for example `0.2.0`.
  - At session start, and before every `spore start` the menu runs, the plugin runs `spore capabilities --json` (C03). That output gains `version` (the semver of the binary) and `contracts` (the spec versions it speaks, e.g. `{"cli": "0.2", "host": "0.2", "kernel": "0.2"}`).
  - Older than the oldest supported spore, or a major contract version the plugin doesn't know, means the L2 message with the update line. Nothing starts.
  - Newer minor versions are accepted.
  - The session-start check is silent when everything fits.
  - Two different `spore` versions on one database are spore's own concern. A `spore` refuses a database whose store schema is newer than it understands (`store.md` 0.2, R6).
- **R4. Where the Claude part of the spec lives (Q5 → A).** The profile spec lives next to the plugin, as `plugins/ml-spore/docs/spec.md` in `max-skills` (name routine). Runspore's `spec/` holds only generic contracts and names no Claude. This replaces the map's planned `spec/claude-code.md` in this repository.
- **R5. The helper outside a Claude session (settled).**
  - Claude Code puts a plugin's `bin/` on the Bash tool's path only. A worker started from the menu, through the Bash tool, inherits that path and finds `claude-spore`.
  - For a worker started in a plain terminal, the plugin offers once to install a launcher at `~/.local/bin/claude-spore`. The launcher finds the newest installed `ml-spore` under `~/.claude/plugins/cache/max-skills/ml-spore/` and runs its `bin/claude-spore`.
  - A mod calls the helper by absolute path (`$.plugin.root + "/bin/claude-spore"`), because `$.process.run` does not see the plugin's `bin/`.
- **R6. Store schema guard (settled, generic).** `store.md` 0.2 records the store schema version in the database. A `spore` that finds a newer one refuses with exit 10 (store error) and says which version wrote it. This is plain Runspore and needs no Claude.
- **R7. Names (settled, replacing every "provisional, C10" mark).**

  | Was provisional | Settled |
  |---|---|
  | plugin name | `ml-spore` (the collection's `ml-` prefix) |
  | `/spore` (menu) | `/spore`, registered by the mod with `$.command.register`; `/spore new`, `/spore change <name>` hand off to the interview |
  | `spore-author` (C09) | skill `author`, shown as `/ml-spore:author`; its guide ships as `skills/author/references/authoring.md` |
  | `claude-spore run\|start\|check\|probe\|watch\|stop\|report` | kept as is |
  | `spore init`, `spore step show`, exit code 7 (cancelled) | kept as is (generic, `cli.md` 0.2) |
  | actor label for a person | `person` |
  | result files `.answers`, `.facts.json`; code `output.missing` | kept as is |
  | `outputSchemas` | kept as is (generic, `kernel.md` 0.2) |
  | node parameters | closed set: `format`, `schema`, `outcome`, `model`, `tools`, `skill`, `context`. Any other key is an error from `claude-spore check`. Only `context` carries data to Claude; the rest are settings (closes C14's open thread) |

- **R8. Default model (settled).** A Claude step with no `model` runs with whatever model the person's Claude Code uses: the helper passes no `--model`. The interview writes `model` only when the person names one (C09).
- **R9. Tool sets (settled).** The interview picks one of two sets for a Claude step's `tools`:
  - **edits:** `Read`, `Glob`, `Grep`, `Edit`, `Write`, `Bash`, for a step that changes files;
  - **reads:** `Read`, `Glob`, `Grep`, for a step that reviews or judges.

  A workflow may list any other tools by hand. `claude-spore check` accepts any Claude Code tool name.
- **R10. Surface (settled).** Terminal Claude Code only. claude.ai and Cowork refuse a plugin with a top-level `bin/`, which matches the map's out-of-scope line for the Desktop app.
- **R11. Permissions (settled).** A plugin cannot ship standing allow rules: its `settings.json` drops `permissions`.
  - The `author` skill and the menu's skill pre-approve `Bash(spore *)` and `Bash(claude-spore *)` with `allowed-tools` for the turn they run in.
  - The plugin's README shows the two rules to add to a project's `.claude/settings.json` for standing approval.

## Verified facts

Checked on 2026-10-11 against Claude Code 2.1.296, code.claude.com and the Claude Code changelog.

- **`bin/` on the path.** A plugin's top-level `bin/` goes on the Bash tool's path, after the user's own entries. It was added in 2.1.91 (docs: plugins/components#executables). It is not on the path of a mod's `$.process.run`, and `${CLAUDE_PLUGIN_ROOT}` is not set in the Bash tool.
- **No install scripts.** A plugin install runs no install script. There is no `postinstall`, and npm sources are fetched with `--ignore-scripts`.
- **No prerequisite field.** `plugin.json` has no field for a required external program or a minimum Claude Code version. `dependencies` lists other plugins only.
- **Versions and data.** `plugin.json` `version` wins over the marketplace entry's (plugins/loading). `${CLAUDE_PLUGIN_DATA}` survives plugin updates. Old cached versions are deleted 14 days after they are orphaned.
- **Permissions from a plugin.** A plugin's `settings.json` applies only `agent` and `subagentStatusLine`. Skill `allowed-tools` pre-approves only for the invoking turn.
- **claude.ai and Cowork** refuse plugins with a top-level `bin/`.
- **`max-skills`.** All six plugins use relative `./plugins/<name>` sources, and `ml-agent-config` ships `bin/agent-config` as a Bun script.
- **Mod commands.** Mods register slash commands with `$.command.register({ name, … })`. Built-in names are refused (docs/research/claude-code-mods.md). The docs don't say whether a mod command's name gets the plugin prefix.

## Changes outside C10

- **Map #5:**
  - The Destination now says the profile spec lives with the plugin in `max-skills`.
  - In the Notes, `docs/13` paragraph 1 ("keep the projects separate") partly stands again: the plugin is a separate project, and only `spore` lives here.
  - The spec-split fog line replaces "a new `spec/claude-code.md`" with the plugin's own spec in `max-skills`, and adds this ticket's `cli.md` 0.2 `capabilities` fields and `store.md` 0.2 schema guard.
- **C09:** `spore-author` becomes skill `author`; "the plugin's default model" is R8; the tool sets are R9.
- **C11 (#16):** the reference loop's test setup installs `spore` (R2) and `ml-spore`, and needs Bun.
- **`docs/13`:** the integration section should point at `max-skills` for the plugin. Revise it when the spec lands, together with the paragraphs the map already marks as superseded.

## Risks

- **Two installs before the first run.** A new person needs `spore`, Bun and the plugin. The plugin's offered install lines keep this to two yes answers, but a person without network access or `~/.local/bin` on their path gets stuck at the first one.
- **Drift between two repositories.** A runspore change that breaks the plugin shows up only when the plugin's tests run against the new `spore`. The plugin's CI should test against the newest runspore release and against its oldest supported spore.
- **`/spore` may be prefixed.** If a mod command's name gets the plugin prefix, the menu becomes `/ml-spore:spore`. The first build must check this, and fall back to a short skill name if it is.
- **The launcher reads Claude Code's cache layout** (`~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/`), which is not a documented contract.

## Deferred

- **A Homebrew tap for `spore`.** Reopen when someone asks for it, or when a release has users outside Max's machines.
- **Listing `ml-spore` in a Web-tree marketplace too.** Reopen if runspore gets its own marketplace.

## Open threads

None.
