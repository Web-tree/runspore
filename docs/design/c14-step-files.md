# C14 — How a step gets its instructions and assets at dispatch

Design record of the grill held on 2026-10-10 for the map ticket **C14 — How a step gets its instructions and assets at dispatch** (Web-tree/runspore#20, map #5). It settles how the files a workflow seals into its artifact (C02) reach the program, person or outside actor doing a step.

**Runspore delivers them, the same way for every kind of step.** It copies a step's declared files out of the run's pinned artifact into a read-only **step files** folder next to the database. It checks them against their digests before every use, and gives their location in two environment variables. A person or outside actor gets the same paths from `spore step claim`, or from a new look-only `spore step show`. Each visit of a step also gets a writable **work folder** kept with the run. That folder is the "runtime dir" that C07, C12 and C13 named but never defined.

**The grill also moved the Claude half of a step.** A step is not told how it may end. A Claude node is a program with **node parameters** that the workflow sets on the node, as part of its input: output format, schema, model, tools, skill. The schema parameter points at the node's own `outputSchemas` (C12), so the format is written once. Runspore checks results against it, and Claude Code enforces it through `--json-schema`. The instructions file becomes plain prose with no frontmatter. That reverses C07 R2 and R6.

There is no visual.

## Terms

- **Step** — A node of a workflow that does something (runs an action, or waits for a signal), seen while a run is at it. *Avoid:* task, stage, job.
- **Step files** — The read-only folder Runspore makes for one node of one artifact, holding only the files that node declares: its instructions file and its action's assets, bundled probe and watch included. *Avoid:* asset dir, bundle, workspace.
- **Work folder** — A writable folder Runspore gives each visit of a step (one invocation), shared by all its tries and kept with the run. *Avoid:* runtime dir, temp dir, scratch.
- **Step description** — What `spore step claim` and `spore step show` print: the step, how it may end, the output schema of each ending, and where its files are. It is printed to actors that speak Runspore's protocol and never handed to programs. *Avoid:* manifest (that is the artifact's), context, step info.
- **Node parameters** — The values a workflow sets on a node for the program its action runs, delivered as part of the node's input. For a Claude node: `format`, `schema`, `model`, `tools`, `skill`. *Avoid:* frontmatter, flags, options.

## Why

C02 seals a step's `instructions` (on a node) and `assets` (on an action) into the immutable workflow artifact, but nothing delivers them when the run reaches the step (C02 Risks: "carried, pinned and inspectable but not delivered"). C03 made `step claim` return *references* to instruction files and left the bytes to this ticket. C07's Claude programs need the instructions and the step's Claude settings when they start. C12's programs also need the node's output schemas. On 0.1 the C04 stopgap passes paths in `argv`, relative to whatever directory `spore` was started from. The ticket's constraints: a resumed run sees exactly the bytes it started with, read from the stored artifact and never from the working tree; Runspore stays generic; nothing is read from the source folder after the build.

On where a Claude node's answer format comes from, Max (Q8):

> "as I see it: we have a node "claude code". it has its input parameters. schema can be one of param, same as format itself. schema makes sense only when the output is json. the workflow implementation which uses the claude code node should set up the parameters of the node."

And before that, on whether a node should know how it may end: "is it a normal situation when the nodes know about anything, about errors or anything? I think not. it should not know anything about that." And, on the answer format: "Claude Code CLI has its own format for that, right? `--output-format json --json-schema` …, and we can define this format, so why not use it?"

## Locked decisions

### L1. A Claude node's settings are node parameters (Q8, Max's text answer)

**Decision.** A Claude node is a node whose action runs the plugin's program (`claude-spore run` or `start`, C07). The program takes **node parameters** that the workflow sets on the node: `format`, `schema` (meaningful only when `format` is `json`), and the settings of L3. They reach it through the node's `input` mapping (kernel §4) as `$literal` values, beside any data the node maps from earlier steps. The program reads them from stdin like any other input. Runspore hands it nothing else, and no program learns anything about the graph: not where an ending leads, not what happens on an error.

**Rejected.**
- *A — a description file in the step's folder listing the node's endings and their schemas (my recommendation).* Lost because it has Runspore hand a program facts about the node that the workflow did not choose to pass. Max's model is the opposite: the workflow configures the program, as with any program's arguments.
- *B — the program asks Runspore for the node's endings with a read command.* Lost for the same reason, and it makes every Claude program find and query the database.
- *C — no format: the instructions text describes the answers in words, and Runspore checks afterwards.* Lost because it gives up Claude Code's own enforcement (`--json-schema`), so a badly shaped answer would stop the node for a person instead of being repaired in the conversation (C12).

**What this keeps.** C07 R7 holds in a sharper form. Runspore never adds outcome names to a program's input or environment. A Claude program sees its own possible answers only because the workflow author put them in its parameters.

### L2. The answer format is written once, on the node (Q9 → B)

**Decision.** The output schemas stay where C12 put them, in the node's `outputSchemas`, and Runspore checks every result against them as C12 decided. The Claude node's `schema` parameter points at them instead of repeating them: `"schema": {"$outputSchemas": {}}` in the node's `input`. The build replaces that marker with a `$literal` copy of the same node's `outputSchemas` (R8), so the package holds a machine-made copy that cannot drift. The program turns the per-outcome map into the one schema that `claude -p --json-schema` takes (C12's procedure). The map's keys are the outcomes Claude chooses between.

**Rejected.**
- *A — the schema only as the Claude node's parameter; Runspore does not check Claude nodes.* Lost because it makes Claude nodes the one kind Runspore does not check, while C12 judges a person's answer and a program's alike. Interactive Claude steps also have no Claude Code flag to lean on; they rely on C12's report command.
- *C — written twice, with the plugin's check warning when the copies differ.* Lost because two copies can disagree, and an answer could pass Claude Code's check and then fail Runspore's.

### L3. Model, tools and skill are node parameters too (Q10 → A)

**Decision.** Every Claude setting C07 put in the instructions file's frontmatter is a node parameter instead: `model`, `tools`, `skill`, and later keys (C07's candidates `permissionMode`, `allow`). The instructions file is plain prose that any reader understands. A node's setup lives in one place, `settings.json`. Runspore carries these values as the node's input data and never reads them, so they are workflow data, not Runspore vocabulary.

**This reverses C07 R2 and R6.** R2 rejected "a `$literal` in the node's input" because it puts a Claude word into the step's data. R6 kept the instructions file as the step's whole Claude half, so that `settings.json` held only Runspore words. Max chose the parameter model over both.

**Rejected.** *B — only the format is a parameter, and model, tools and skill stay in the instructions header.* Lost because one node's setup would be split across two files.

## Routine choices

Q1–Q7 and Q11 were settled from standard practice and shown as answered on the grill page; Max left them standing.

- **R1. Runspore delivers the files (Q1).** Delivery is part of `spec/host.md` 0.2 and works the same for every kind of step and every host, not as a profile feature. Only Runspore can open the artifact, and a person doing a step by hand needs the files as much as a program does. Build tools and CI runners hand steps their own files. Rejected: each add-on unpacking the artifact itself.
- **R2. From the stored artifact, read-only, checked before every use (Q2).** Step files are copied from the run's pinned artifact (`blob(digest)`, C02 L8), never from the author's folder. Before every use the host checks each file's digest against the manifest. If a file is missing, changed or extra, it removes the folder and makes it again. Files are read-only (`0444`, or `0555` when executable; folders `0555`). That catches a hand-edited copy between tries. It cannot stop a program that changes permissions mid-try, and the next check repairs it. Nix and Bazel work this way. Rejected: reading from the author's folder, which breaks the resume guarantee on the first edit.
- **R3. A step sees only its own files (Q3).** A node's step files hold its `instructions` entry and the entries owned by its action, bundled probe and watch included (C07 R8). Another node's files are not there, so a step cannot quietly depend on a file it never declared. Build tools give each target only what it declares. Rejected: one folder with every file of the workflow.
- **R4. Next to the database, kept while needed (Q4).** Step files live at `<files dir>/steps/<artifact digest>/<node id>/`. They mirror the folder-relative paths of the manifest (`instructions/implement.md`, `assets/check.py`). `<files dir>` defaults to `<database path>.files`, so `spore --db x.db` uses `x.db.files/`, and an ignore rule for `runspore.db*` covers it. A step's folder is made on first use, built in a sibling temporary folder and renamed into place. A process that loses that race checks the winner's folder and uses it. The worker's sweep removes `steps/<artifact digest>/` once no unfinished run pins that artifact. Rejected: a system temporary folder, which a restart can wipe mid-step.
- **R5. Programs find their files through the environment; a bundled program is named `./…` (Q5).** Every program the host runs for a step gets two new variables: the command, an async start command, a probe and a watch (C03). `RUNSPORE_STEP_FILES` is the absolute path of the node's step files, always set; the folder may be empty. `RUNSPORE_INSTRUCTIONS` is the absolute path of the node's instructions file, unset when there is none. An `argv[0]` of the form `./<path>`, where `<path>` is one of the action's declared `assets`, runs the step files' copy. Runspore replaces it with the absolute path. Any other `./…` keeps 0.1's meaning, relative to `cwd`, so `./test.sh` in the project still runs the project's script. Probes and watches keep C07 R8's form, where a `./` reference is bundled without being listed. The manifest records whether a file is executable (R9). Rejected: placeholders in `argv`, a new mini-language left for later (Deferred).
- **R6. People and outside actors get the same paths; a look-only command shows a step (Q6).** `spore step claim` (C03 R2) adds `files` and `instructions` (absolute paths, `instructions` null when there is none) to its JSON. In text mode it prints the instructions text after the claim's summary. A new `spore step show [<runId>]` prints the run's current step as a **step description** without claiming, probing or changing anything: run, node, node kind, action (for an activity), signal (for an await-signal node), outcomes, `outputSchemas`, `files`, `instructions`. In text mode it prints the instructions text. It serves a person deciding at an `await-signal` node (whose instructions nobody claims) and the mod's menu (C08). Rejected: delivery only on claim, which leaves decision steps without their instructions.
- **R7. Each visit gets a writable work folder (Q7).** `RUNSPORE_WORK_DIR` is `<files dir>/work/<run id>/<invocation id>/`. The host creates it before the first try, and every program run for that invocation gets it: start, probe, watch and every try. It is kept with the run, and removing old runs and their folders is a later prune (Deferred). It replaces the undefined `<runtime dir>` of C07, C12 and C13 (§Changes). A finished answer written just before a crash survives until the next try reads it, and the menu can find a step's files. CI runners give each job a work folder. Rejected: each add-on picking its own place.
- **R8. The schema reference is resolved by the build (Q11).** `{"$outputSchemas": {}}` is a build-only mapping form, allowed anywhere in a node's `input` in `settings.json`. The build replaces it with `{"$literal": <that node's outputSchemas>}` and fails when the node declares none. A package that still contains it fails validation. This is one more reference of the kind the build already resolves (C02 R4), so the kernel and its certified component stay unchanged. Rejected: the engine looking it up each time the node starts, which would be a kernel mapping change.
- **R9. The manifest records executable files.** A manifest entry gains `"executable": true` when the source file had any execute bit, as in git's two file modes; the field is omitted otherwise. Without it, a bundled script could not run as `argv[0]`.
- **R10. An unreadable artifact quarantines the run.** If the host cannot produce a step's files, it does not start the try. That happens when the stored artifact is missing, an entry's blob is absent, or its digest does not match. The run is quarantined with reason `artifact.unreadable` and `release` retries after repair. It is a broken store, not a business failure, and the host never falls back to the source folder.

## Spec

Normative wording to carry into `spec/host.md` 0.2, `spec/cli.md` 0.2, `spec/artifact.md` (C02), and the profile spec. Names marked provisional are settled with C10.

### Host 0.2: step files and work folders

`EngineConfig` gains `files_dir` (default: `<database path>.files` for the CLI; required for embedders that run actions with files).

For a run at node `N` of artifact `A`:

1. **Step files** `S = <files_dir>/steps/<hex of A's digest>/<N>/` contain every manifest entry with `owner.node = N` (kind `instructions`) and every entry owned by `N`'s action (kind `asset`), at its folder-relative `path`.
2. **Making S.** Read each entry with `blob(digest)`, verify its digest, and write it into a fresh sibling folder with mode `0444`, or `0555` when `executable`. Set folders to `0555` and rename to `S`. If `S` already exists, the loser of the race uses it after step 3.
3. **Checking S, before every use.** Before each dispatch, start, probe, watch, `step claim` and `step show` that touches `N`, the set of files under `S` must equal the entries of step 1 and each file must match its digest. Otherwise remove `S` and make it again (step 2).
4. **Failure.** If step 2 cannot read or verify an entry, quarantine the run with reason `artifact.unreadable`, and start nothing.
5. **Work folder** `W = <files_dir>/work/<runId>/<invocationId>/`. Created (mode `0700`) before the invocation's first try; never removed by the host in this version.
6. **Sweep.** The worker's sweep removes `<files_dir>/steps/<artifact>/` when no unfinished run pins that artifact.

Every program the host runs for an invocation (a `command`, an async start command, a probe, a watch) gets these variables beside host 0.1's `RUNSPORE_*`:

| Variable | Value |
| --- | --- |
| `RUNSPORE_STEP_FILES` | absolute path of `S` |
| `RUNSPORE_INSTRUCTIONS` | absolute path of `N`'s instructions file in `S`; unset when `N` has none |
| `RUNSPORE_WORK_DIR` | absolute path of `W` |

`argv[0]` of a `command` or async start command that reads `./<p>`, where `<p>` is in the action's `assets`, is replaced by the absolute path of `<p>` in `S`. Other `argv` entries and `cwd` keep 0.1's meaning.

### Build and artifact (C02 amendments)

- A node's `input` in `settings.json` may contain `{"$outputSchemas": {}}` at any depth. The build replaces each with `{"$literal": <the node's outputSchemas>}`, and it is a build error on a node without `outputSchemas`. The kernel never sees the form.
- A manifest entry is `{path, digest, kind, owner, executable?}`; `executable: true` when the source file had any execute bit.

### CLI 0.2

| Command | Change |
| --- | --- |
| `step claim` | JSON gains `files` and `instructions` (absolute paths; `instructions` null when none). Text output prints the instructions text after the summary |
| `step show [<runId>]` (new, provisional name) | Prints the step description of the run's current step (the only unfinished run when omitted) without claiming or probing. Exit 2 when the run is not at an activity or await-signal node |

Step description, JSON:

```json
{"format": "runspore.cli/0.2", "runId": "run_…", "nodeId": "review", "nodeKind": "activity",
 "actionId": "review", "outcomes": ["approved", "needs-changes"],
 "outputSchemas": {"approved": {…}, "needs-changes": {…}},
 "files": "/…/runspore.db.files/steps/…/review/",
 "instructions": "/…/runspore.db.files/steps/…/review/instructions/review.md"}
```

An await-signal node shows `"nodeKind": "await-signal"`, `"signal"`, and no `actionId`.

### A Claude node, written down

```json
"actions": {
  "review": { "kind": "command", "argv": ["claude-spore", "run"], "output": "result",
              "outcomes": ["approved", "needs-changes"],
              "effect": "idempotent", "retry": {"maxAttempts": 3}, "timeoutMs": "3600000" }
},
"nodes": {
  "review": {
    "action": "review",
    "instructions": "instructions/review.md",
    "outputSchemas": {
      "approved":      { "type": "object", "properties": {"summary": {"type": "string"}}, "required": ["summary"] },
      "needs-changes": { "type": "object", "properties": {"changes": {"type": "array", "items": {"type": "string"}, "minItems": 1}}, "required": ["changes"] }
    },
    "input": {
      "format": {"$literal": "json"},
      "schema": {"$outputSchemas": {}},
      "model":  {"$literal": "opus"},
      "tools":  {"$literal": ["Read", "Grep", "Glob"]},
      "context": {"tests": {"$get": ["nodes", "test", "output", "stdout"]}}
    }
  }
}
```

The parameter names (`format`, `schema`, `model`, `tools`, `skill`, `context`) belong to the plugin and are provisional (C10). `context` carries the data the author maps from earlier steps. `instructions/review.md` is plain prose and explains what each ending means (C07 R7).

### Plugin side (amends C07 and C12 procedures)

- **Background (`claude-spore run`).**
  1. Reads its parameters from stdin, and the prose from `RUNSPORE_INSTRUCTIONS`.
  2. With `format: "json"`, builds the one `--json-schema` value from the `schema` map, as C12 describes.
  3. Runs `claude -p --output-format json` with the model, tools and skill from its parameters, and `--add-dir "$RUNSPORE_STEP_FILES"` so Claude can read the step's files.
  4. Sends the prose as the first message, telling Claude where the step files are.
  5. Writes run facts to `$RUNSPORE_WORK_DIR/<RUNSPORE_ATTEMPT>.facts.json`.
- **Interactive (`claude-spore start`).**
  1. Reads its parameters from stdin and saves them to `$RUNSPORE_WORK_DIR/params.json`, so the report command and later tries can read them.
  2. Starts Claude in the herdr pane with the model, tools and skill from its parameters and `--add-dir "$RUNSPORE_STEP_FILES"`.
  3. Passes `RUNSPORE_WORK_DIR` into the pane's environment.
  4. The report command (C12) checks answers against the `schema` in `params.json` and writes `$RUNSPORE_WORK_DIR/<attempt>.result.json` and `<attempt>.answers`. The watch reads them from the same folder.
- **`format: "text"`.** Allowed only on a node with exactly one outcome. The output is `{"text": "<Claude's reply>"}`, and no `--json-schema` is passed. Runspore still checks it against that outcome's schema when one is declared.
- **`claude-spore check`** (C07 R10) validates these parameters in `settings.json` (closed key set, `schema` present when `format` is `json`) instead of frontmatter keys.

## Verified facts

- Host 0.1 §3: a `command` gets the canonical input on stdin and `RUNSPORE_RUN_ID`, `RUNSPORE_NODE_ID`, `RUNSPORE_INVOCATION_ID`, `RUNSPORE_ATTEMPT`, `RUNSPORE_EFFECT_KEY` in its environment. `cwd` resolves against `base_dir` (default: the current directory). `argv` is never run through a shell.
- CLI 0.1: the database is `--db`, else `$RUNSPORE_DB`, else `./runspore.db`.
- C02 R9 / §The manifest: entries are `{path, digest, kind, owner}` with no file mode. `instructions` and `assets` are stripped from the package at build. The C02 Risks section says nothing delivers them yet.
- C03 R2: `step claim` returns references to the step's instruction files and leaves the bytes to C14. `status --json` deliberately omits input and instructions.
- C07 R8: a `./` probe or watch reference is bundled as an action asset; C07 R2 and R6 put `skill`, `model` and `tools` in instructions frontmatter.
- C07, C12 and C13 write result, answer and facts files under `<runtime dir>`, which no record defines.
- C04's example runs `["./test.sh"]` with `cwd: "project"`, a project script, not a workflow file. A `./` rule that bundled every `argv[0]` would change its meaning, hence R5's "listed in `assets`" condition.
- `claude --help` (2.1.x): `--json-schema <schema>` takes the schema as a JSON string (its example is inline JSON), and `--add-dir <directories...>` grants tool access to extra directories.
- Kernel 0.1 W03/W04: node and action IDs match `[A-Za-z0-9_-]{1,64}`, so they are safe as path segments.

## Changes outside C14

- **C02.** The manifest entry gains `executable` (R9). `settings.json` node `input` admits the build-only `$outputSchemas` form (R8). The Risk "nothing reads them at dispatch" is answered here.
- **C03.** R2's claim adds `files` and `instructions`. Its open thread "Instruction delivery" is answered. The CLI gains `step show`.
- **C07.**
  - R2 and R6 are reversed by L3: `skill`, `model`, `tools` and later keys are node parameters set as `$literal` values in the node's `input`, and the instructions file has no frontmatter.
  - R10's check reads `settings.json`.
  - R7 is restated by L1: Runspore never adds outcome names, but an author may pass a Claude node its own schemas.
  - Its plugin procedures read parameters from stdin and files from `RUNSPORE_STEP_FILES` / `RUNSPORE_INSTRUCTIONS`.
  - "`<runtime dir>/<session uuid>.result.json`" (R15) is superseded by C12/C13's per-try files, now under `RUNSPORE_WORK_DIR`.
- **C12.** Item 1 of the background procedure ("reads the node's outcomes and output schemas, through the same channel C14 settles") becomes "reads its `schema` parameter". `<runtime dir>/<invocation>/` becomes `$RUNSPORE_WORK_DIR/`, so files are `<attempt>.result.json`, `<attempt>.answers` and `<attempt>.facts.json`. "Every outcome of a Claude step needs a schema" applies to `format: "json"`.
- **C13.** The interactive result file moves under the work folder, as in C12.
- **C04.** The `argv`-relative-path stopgap is replaced: instructions come from `RUNSPORE_INSTRUCTIONS`, and the schema from the node's parameters.
- **C10.** Parameter names, `step show`'s name, and the closed parameter key set for `claude-spore check`.
- **C11.** The reference loop writes its Claude nodes as in §A Claude node.

## Risks

- **Claude words now sit in `settings.json`.** They are data Runspore never reads, so the specs stay generic, but a workflow file is no longer Claude-free (the cost C07 R2 avoided).
- **Parameters are stored with every invocation.** A node's evaluated input, schema copy included, is part of each invocation's command and snapshot. A schema near C12's 64 KiB minimum-value cap adds that much per visit and counts against `limits.maxStateBytes` and `limits.expressionOperations`. C12's build-time size rule should count the `$literal` copy too.
- **Disk use.** One copy of the step files per artifact version and node, kept while any unfinished run pins the version. Large assets across many versions add up until the sweep runs.
- **Read-only is not protection.** A program that changes permissions can edit its step files during its own try. The check before the next use repairs them, but not the try that did it.
- **Work folders grow without a prune.** Nothing removes them in this version.
- **Paths are local.** An actor that claims a step from another machine would get paths it cannot open. SQLite and `spore` are single-machine today, so this waits for the concurrency fog.

## Deferred

- **Placeholders in `argv`** (`{"$file": "assets/x.py"}` or similar) for passing a bundled file as an argument to an interpreter. Reopens when a workflow needs a non-executable bundled script, or one that is not `argv[0]`.
- **Prune of finished runs and their work folders.** Reopens with the progress/evidence fog item, since run facts live in work folders.
- **Delivering step files to an actor on another machine** (bytes over the claim instead of paths). Reopens with the concurrency fog item.

## Open threads

- The step description's exact field names and whether `step show` also probes a held step (C03 probes on `status`, `continue` and `claim`). Settled with `cli.md` 0.2.
- Whether the `context` parameter should be the Claude program's only data key, or whether any non-parameter key is data. That is a plugin convention, settled with C10.
