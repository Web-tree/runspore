# C18 — One grammar for the CLI: verb, then kind

Design record of the CLI review held on 2026-10-15. It decides the shape of every `spore` command from CLI 0.3 on. Today's 23 commands mix three grammars: verbs on an implied run (`signal`, `resolve`, `release`), bare nouns (`status`, `events`, `worker`) and one noun group (`step claim`). They name one idea three ways (`--data`, `--output`, `--complete <o>`), and three of them answer "what is this run doing?" differently. **From 0.3 every command is `spore <verb> <kind>/<name>`, as in kubectl. A small set of read verbs (`get`, `describe`, `explain`, `wait`) works on every kind, because each kind is registered as data: its names, columns, schema and conditions.** Kinds that plugins add later get the same read verbs with no code of their own. Actions that fit one kind (`signal`, `resolve`, `cancel`, `claim`) are verbs too. The kind registry says which kinds each verb accepts, so a wrong pair fails with the list of kinds that do accept it. Today's commands keep working as aliases with a warning for two minor releases.

Max asked for the review: "for me, it looks very messy … I thought about kubectl-like design, where we have actions first, like get, describe, etc. as a first-level citizen of the cli, and other apis already go next … and I think in this way our wasm plugins will be able to extend this api as well, as kubernetes do." Four research notes back it: an inventory of today's CLI, kubectl's grammar and extension model, noun-first and workflow-engine CLIs, and CLI design for people and scripts. The full notes are not committed; this record keeps their findings and sources.

## Terms

- **Kind** — A type of thing spore can list and describe: `run`, `step`, `signal`, `event`, `workflow`, `artifact`, `worker`, `quarantine`, `code`, `kind`. Each kind has one registry entry (L3). *Avoid:* resource type, noun, object type.
- **Verb** — The first word of a command. **Read verbs** (`get`, `describe`, `explain`, `wait`) work on any kind whose entry lists them. **Action verbs** (`start`, `signal`, `resolve`, `cancel`, `claim` and the rest) change state and accept only the kinds their entry names. *Avoid:* subcommand, operation.
- **Target** — What a verb acts on, written `kind/name` (`run/9f3a1c`, `step/9f3a1c:implement`) or as the two words `kind name`. *Avoid:* resource, ref.
- **Kind registry** — The table of kind entries the CLI dispatches on. Core kinds fill it in 0.3; plugin kinds join it with M6 (L7). *Avoid:* API discovery, schema registry.

## Why

What the inventory found (CLI 0.2.0, `crates/runspore-cli/src/cli.rs:46-248`):

1. **Three grammars.** `signal`, `resolve`, `release` and `continue` are verbs on an implied run; `status`, `events`, `capabilities` and `worker` are nouns used as commands; `step` is the only noun group, and inside it "step" means three things: the node a run is at (`show`), a claimed attempt reached by token (`complete`), and a held async attempt reached by run ID (`abandon`). `run` is both the main noun and a verb.
2. **One idea, several names.** Outcome data is `signal --data` but `resolve --output` and `step complete --output`. The outcome is `--outcome` twice and `--complete <o>` once. The run is a positional argument in most commands and `--run` in `worker` and `inspect`. The repeat-safety key is `--key` on `start` and `--id` on `signal`.
3. **Views disagree.** After `resolve --retry` with no worker running, `list` says `needs-intervention`, `list --unfinished` says `working`, and `status` still offers the three resolve commands. A second `resolve` printed `resolution applied` and was dropped as `resolve.not-applicable`. `status --json` has no `needs` field.
4. **Five ways to drive a run** (`run`, `worker`, `worker --run --until-parked`, `continue`, and the planned `continue --wait`) and **four ways to read one** (`status`, `list`, `list --unfinished`, `step show`).
5. **Flags where they do nothing.** Six worker tuning flags, five without help text, are global and show on `init`, `explain` and `build`. A misplaced flag gets clap's tip `to pass '--follow' as a value, use '-- --follow'`. Runs are named only by the full 36-character ID, though progress lines name them by start key.
6. **Nothing can be extended.** WebAssembly runs the kernel and holds the workflow artifact; action kinds are compiled-in Rust; the clap tree is a closed enum. A plugin cannot add a kind, a verb or a column today.

Scored against a 20-point checklist for CLIs used by people and scripts (§Sources), 0.2.0 gets about 17 of 40. It is strong on JSON output, repeat-safe writes and having no prompts. It has no field selection, no `wait`, no stream mode, no dry run, no single verb set and no generated help.

## Locked decisions

### L1. Verb first, kind second, for every command (Max)

**Decision.** Every command is `spore <verb> [<target>…] [flags]`. The verb comes first and the target is `kind/name` or `kind name`. Kinds accept their singular, plural and short names (`run`, `runs`, `r`). No command is a bare noun and no noun groups hold verbs.

Max chose this over the hybrid the research recommended: run verbs at the top level with the run implied, plus noun groups (`spore workflow build`, `spore step claim`) for the rest. That is the shape of Argo's CLI and close to spore 0.2. No workflow engine CLI the research found puts the verb first for run operations (Temporal, Argo, Restate, Prefect and Airflow are all noun-first or run-implied). The research named two costs of verb-first; L4 and L5 answer them:

- A verb such as `resolve` makes sense for one kind only, so the grammar allows pairs that mean nothing (`resolve worker/x`). L4 makes the registry reject them with the kinds that fit.
- `spore run` is a verb today and `run` would be a kind. L5 renames the verb.

Rejected:
- *Hybrid, run implied (research recommendation).* Shorter for the most common calls, but it keeps two grammars, and a plugin kind could not reuse `get` and `describe`.
- *Noun-first (`spore run list`, `spore run cancel`), as Temporal does.* Plugins could add nouns, but every noun group would repeat its own `list` and `describe` with its own output.

### L2. The verbs (standard practice)

**Decision.** These verbs make up the whole CLI. The kinds column is what the core registry allows; plugins may add kinds to the read verbs, never new meanings to a core verb (L7).

The table shows each verb's targets and the flags this design renames or adds. Every other 0.2 argument and flag moves unchanged to the verb that replaces its command, and keeps its meaning: `--input`, `--input-file` and `--base-dir` on `start` and `execute`; `--retryable` on `fail`; `--short`, `--strict` and `--fix` on `validate`; `--composed` and `--strict` on `build`. One clash is renamed: `build -o <path>` becomes `build --out <path>`, since `-o` is the output format everywhere (L8). `build` is a 0.2 command and a 0.3 verb under one name, so through 0.4 its `-o` keeps the 0.2 meaning, a path (L9), and `build` takes its output format only as `--json`; `-o` is the format on `build` from 0.5.

| Verb | Kinds | Replaces |
|---|---|---|
| `get <kind> [name…] [--unfinished]` | every kind | `list`, `list --unfinished`, `events`, `explain` (no code) |
| `describe <target>` | every kind | `status`, `step show`, `explain <code>`, `inspect <artifact>` |
| `explain <kind>[.field…]` | every kind | — (new: field docs from the kind's schema) |
| `wait <target> --for=<condition> [--timeout <duration>]` | run, step | — (new; replaces polling `status`) |
| `start workflow/<name> \| <path>` | workflow, artifact | `start` |
| `execute workflow/<name> \| <path>` | workflow, artifact | `run` (L5) |
| `continue [run/<id>] [--wait]` | run | `continue`, `worker --run --until-parked` |
| `signal run/<id> <name> [--outcome] [--data] [--key]` | run | `signal` |
| `resolve run/<id> \| invocation/<run>:<inv-id> --retry \| --outcome <o> \| --error <msg> [--data] [--key]` | run, invocation | `resolve` |
| `cancel run/<id> [--reason] [--key]` | run | — (c08 §spore cancel, #74) |
| `release quarantine/<run-id>` | quarantine | `release` |
| `claim step [--run <id>] \| step/<run>:<node> [--actor] [--holder]` | step | `step claim` |
| `renew`, `complete`, `fail` `step --token <t>` | step | `step renew`, `step complete`, `step fail` |
| `abandon step/<run>:<node>` | step | `step abandon` |
| `serve [--slots] [--lease-ms] …` | — | `worker` |
| `validate workflow/<name> \| <path>` | workflow, artifact | `validate` |
| `build workflow/<name> \| <path>` | workflow | `build` |
| `cat run/<id> <path>` | run | `inspect --run --file` |
| `init` | — | `init` |
| `version` | — | `capabilities` |

`get` prints a table, one row per item, with columns from the kind's entry (L3); `-o wide` adds the low-priority columns. `describe` prints everything a person needs about one item: for a run, its status, position, the arrow into it, what it needs, the outstanding invocation, the commands that answer it, and whether a worker is alive. It is the one call that answers "what is this run doing and what do I do?" (`describe run/X -o json` gives the same as an object, which kubectl lacks.) `describe` changes nothing and shows nothing stale (#268). At a held async step whose action has a probe, it runs the probe. On a `dead` answer it records nothing: it shows the attempt as cut off with reason `probe-dead` and what happens next, the same way `status` already shows a worker's try that was cut off. The expiry is written by the verbs that write, as today: `serve`'s sweep, `claim` and `continue`. `status --no-probe` does not carry over, since `describe` writes nothing; the alias accepts it and ignores it. `wait` reads the same live view. `continue run/X` works the run until it parks, as `worker --run --until-parked` does: it runs watches, and exits 6 at a held step with no watch, or one whose watch says the holder waits for a person. At a step another worker runs, it waits for that worker's result instead of returning exit 0, so it replaces `worker --run --until-parked` exactly (#268). `get events --run X` replaces `events`, as `get` with a parent (L6). `get runs --unfinished` keeps today's `list --unfinished`: only unfinished runs, newest first, filtered in the store rather than by the caller. `get runs` keeps `list`'s tree in text output: a child run is indented under its parent, in sibling order, with the parent node named. `start` and `execute` take the same targets `start` and `run` take today: `workflow/<name>` for a workflow in the project folder, or a path to a workflow folder, its `workflow.dot` or a built artifact. As today, an existing path wins: a target that names an existing file or folder is that path, and only otherwise is it a workflow name in the project folder. `workflow/<name>` always means the name. When two folders share a workflow name, which a lookup by name refuses with `workflow.ambiguous`, `get workflows -o name` prints each of them by its path, which every workflow verb takes.

Why settled: these are kubectl's read verbs (`get`, `describe`, `explain`, `wait`) and Temporal's run verbs (`start`, `execute`, `signal`, `cancel`), with spore's own `resolve`, `continue`, `claim` and the step-worker verbs kept as they are. `serve` is the usual name for a long-running process that does work for others; `worker` stays the kind it serves as (`get workers`). `version` follows `kubectl version`. `version -o json` keeps the whole handshake `capabilities` gives today, `actionKinds`, `asyncKinds` and `resultChannels` included, so a caller such as `ml-workflow` checks the host in one call before it starts anything. Action kinds (`command`, `external`, `native`) are host capabilities, not kinds a verb targets, so they stay in that handshake and are never rows of `get kinds`.

### L3. Each kind is one registry entry (standard practice)

**Decision.** The CLI reads one entry per kind and dispatches every read verb on it. Core kinds are entries like any other.

```yaml
kind: Run
singular: run
plural: runs
shortNames: [r]
categories: [all]          # `get all` lists every kind in the category; core kinds all join it
owner: core                # or plugin:<id>
parent: null               # step, signal, event and invocation name `run` here
schema: <JSON Schema with descriptions>     # explain, -o json shape, condition names
columns:                   # NAME is implicit
  - {name: STATUS,   jsonPath: .status,           type: string}
  - {name: NODE,     jsonPath: .position.nodeId,  type: string}
  - {name: NEEDS,    jsonPath: .needs,            type: string}
  - {name: AGE,      jsonPath: .sinceMs,          type: date}
  - {name: WORKFLOW, jsonPath: .workflow,         type: string, priority: 1}
  - {name: KEY,      jsonPath: .startKey,         type: string, priority: 1}
verbs: [get, describe, explain, wait, continue, signal, resolve, cancel, cat]
conditions: [Finished, Waiting, NeedsIntervention, Quarantined, Parked]
source: {list: <SQL view or plugin export>, get: <SQL view or plugin export>}
```

`get kinds -o wide` prints every entry with its short names, owner, verbs and conditions, so a person or a script finds which verbs fit a kind in one call. kubectl cannot do this: its discovery lists only REST verbs, never `rollout` or `logs`.

Why settled: this is the CRD model reduced to what a local tool needs (kubectl's `additionalPrinterColumns`, `shortNames`, `categories` and `kubectl explain`). Left out as overkill: API groups and versions, discovery endpoints and caching, namespaces and contexts, `apply`/`edit` (runs are history, not desired state), the full label-selector language and go-templates. `get all` lists every core kind, never a subset; kubectl's `get all` leaves kinds out and is a known trap (kubernetes/kubectl#151).

### L4. A verb with a kind it does not fit fails with the kinds that fit (standard practice)

**Decision.** Before any database read, the CLI checks the verb against the target kind's `verbs`. A mismatch is exit 2 with `verb.kind-mismatch`: `resolve does not apply to worker; it applies to: run`. A flag that belongs to another verb gets the same treatment instead of clap's tip: `spore continue run/X --retry` says `--retry belongs to resolve: spore resolve run/X --retry`. This answers the main cost of verb-first that the research named, and closes the misplaced-flag gap from checkpoint D.

Why settled: Cloudflare's `cf`, gh and Arcjet's guidance all reject a bad pair locally with the fix; kubectl's weakest point here is that you learn a verb does not fit only from a server error.

### L5. `spore run` becomes `spore execute` (standard practice)

**Decision.** `execute workflow/<name>` starts a run and works it until it ends or parks, as `run` does today. `run` is a kind only. The old `spore run <workflow>` keeps working as an alias (L9); with no workflow argument after it, `spore run X` is read as the alias, never as a kind.

Why settled: Temporal names the same pair `start` and `execute` (`temporal workflow execute` starts and waits for the result).

### L6. One address form for every target (standard practice)

**Decision.**
- A run is `run/<id>`, where `<id>` is the full run ID, any unique prefix of at least 6 hex digits, or its start key. Start keys and ID prefixes are matched together: a name that matches more than one run, in either form or across both, fails with the matching runs listed, and spore never picks one. `run/key:<k>` names a run by start key only, for scripts that must not hit an ID prefix.
- A target with a parent names it before a colon: `step/<run>:<node>`, `signal/<run>:<message-id>`, `event/<run>:<sequence>`, `invocation/<run>:<inv-id>`. `--run <id>` scopes a whole command instead (`get steps --run 9f3a1c`), as kubectl's `-n`.
- `-o name` prints targets in this form, and every command takes them back, so the output of one command is input to the next.
- IDs keep their kind prefixes (`run_`, `inv_`, `msg_`, `req_`). A prefixed ID of the wrong kind fails before any work.
- `continue`, `describe run` and `describe step` without a name still pick the only unfinished run, and fail listing the candidates when there are several; `describe step` then describes the step that run is at, as `step show` does. `claim step` without `--run` keeps today's queue behaviour: it claims the oldest ready external step that matches, across every run in the database. `claim step/<run>:<node>` claims that step, so a name from `get steps -o name` is claimed as it is, and fails with `step.not-ready` when it is not the run's ready step.
- A step target may name one visit of its node by activation: `step/<run>:<activation>`, as in `step/9f3a1c:approve/1`. `-o name` prints steps in this form. `claim` and `abandon` given an activation act only while it is the run's current one, and otherwise fail with `step.not-ready`, so a stale name never acts on a later visit of a node a bounded loop comes back to. `step/<run>:<node>` means the current visit.
- `resolve invocation/<run>:<inv-id>` answers that invocation only. If the run has moved on to another invocation, it fails with `resolve.not-applicable` and changes nothing, as `resolve <run> <inv>` does today. `resolve run/<id>` answers the outstanding one.
- `renew`, `complete` and `fail` name the attempt by `--token` alone, as in 0.2: the token already carries the run, the invocation and the attempt, so a reporter that kept only `RUNSPORE_TOKEN` can still report. They are the one exception to the `-o name` round trip: `-o name` prints the step as `step/<run>:<activation>`, which `describe` takes, but only the token reports on an attempt, since the token also carries the attempt and its fence when a run comes back to the same node.

Why settled: `TYPE/NAME` with plural, singular and short names is kubectl's form; prefix matching that refuses ambiguity is git's and Restate's. Start keys are unique per database (`UNIQUE (tenant, start_key)`, C16), so they are safe names.

### L7. Plugins add kinds, and verbs only where no core verb fits (standard practice)

**Decision.** When the M6 plugin work lands (docs/17), a WebAssembly plugin ships a manifest with kind entries (L3) and exports `list` and `get` functions for them. The core then gives each plugin kind `get`, `describe`, `explain`, `wait`, `-o json|name|wide` and completion, with no output code in the plugin. `wait --for=<condition>` reads conditions the Kubernetes way: an object reports `status.conditions`, a list of `{type, status: "True"|"False"|"Unknown", reason, message}`, and a condition the kind declares holds when its entry has `status: "True"`. Core kinds report their conditions the same way. A plugin may declare an action verb for its own kinds, with its arguments as a schema, and exports `invoke`: the core calls it with the verb, the target and the parsed arguments, and it returns the L8 result or error object (stable code, state, `retryable`, next commands). The core parses the arguments, prints the result and shows the verb in `get kinds -o wide`. The exact ABI of `list`, `get` and `invoke` is fixed with M6. A plugin can never redefine a core verb or kind, or add a core kind to its verb; a clash fails when the plugin loads. Plugin kinds and verbs show their owner in `get kinds`.

0.3 builds the registry and puts the core kinds in it. Nothing loads plugins until M6. The clap tree becomes one built at start from the registry instead of a derive enum, so core and plugin kinds take the same path from the first day.

Why settled: kubectl's lesson is that CRDs (new kinds as data) carry most of the value, and PATH plugins (new verbs as code) are opaque, rebuild their own output and cannot be listed by kind. Helm 4 runs its plugins as WebAssembly. Every extension model the research found except Azure's stops plugins from overriding built-ins.

### L8. One flag vocabulary and one output contract (standard practice)

**Decision.**
- **Flags.** `--data` is every JSON payload (`signal`, `resolve`, `complete`), `--outcome` every outcome, `--key` every repeat-safety key (`start`, `signal`, `resolve`, `cancel`), `--fail`/`--error` both become `--error`, `--run` every run scope. Worker tuning flags (`--lease-ms`, `--heartbeat-ms`, `--poll-ms`, `--grace-ms`, `--stop-grace-ms`, `--max-watches`) belong to `serve`, `execute` and `continue` only; `--db` stays global; `--native-kernel` becomes the `RUNSPORE_NATIVE_KERNEL` variable. Every flag and argument has help text and every verb has examples.
- **Output.** `-o text|wide|json|jsonl|name`; `--json` stays as `-o json`. `--fields a,b` selects fields in JSON. `-o jsonl` streams one object per line and ends with `{"type":"done"}`; it is the form of `wait` and `continue --wait`. Results go to stdout, progress and warnings to stderr.
- **Durations.** `wait --timeout` takes a number with a unit (`500ms`, `30s`, `5m`, `1h`, or a compound such as `1m30s`), as kubectl does. A bare number is refused with exit 2 and an error that shows the fix: `--timeout 30 has no unit; give one, e.g. 30s or 5m (units: ms, s, m, h)`. Leaving out `--timeout` waits at most 30s, and `0s` checks once and exits. No value waits forever (#268).
- **Errors.** Under `-o json` every error is JSON, including usage errors, and names the argument. Each error has a stable code, the current state, `retryable`, and the exact next commands. Writes print `kind/name verbed` and the next command.
- **Exit codes.** The 0.2 table stays, plus `7` cancelled (c08), `8` timeout (`wait`), and `9` not found from 0.5. Until then anything not found, a run, a workflow or a code, stays exit 2 as in 0.2, since `start`, `validate` and other 0.2 command names are unchanged and L9 keeps them working.
- **Dry run.** `--dry-run` on `signal`, `resolve`, `cancel` and `release` prints what the command would change and the resulting status, writing nothing: the event `signal`, `resolve` and `cancel` would record, and for `release`, which records no event, the quarantine it would clear.
- **One source.** `--help`, `help -o json`, shell completion and the CLI reference come from the registry and the clap tree. The reference is its own generated file, `spec/cli-reference.md`, with each verb's synopsis, flags, kinds and examples. CI fails when the committed file is out of date. `spec/cli.md` stays hand-written, keeps the prose contracts and links to the reference (#268).

Why settled: gh (`--json fields`), kubectl (`-o`, `wait --for`), terraform and cargo (JSON streams with a final event), Restate's 2026 CLI revamp (typed exit codes, next steps, dry run), Cloudflare's `cf` (generated from one schema) and Arcjet's guidance all converge on these. They fix the 0.2 gaps listed in §Why.

### L9. Old commands stay for two minor releases (standard practice)

**Decision.** Every 0.2 command keeps working in 0.3 and 0.4 as a hidden alias that prints one warning line on stderr with its new form (`spore status is now: spore describe run/9f3a1c`) and adds a `deprecations` array to JSON. The 0.2 flags do too: `--native-kernel` and the six worker tuning flags stay accepted in their 0.2 places, `--fail` and `--complete <o>` on `resolve`, `--output` on `resolve` and `step complete`, `-o` and `--output` as the artifact path on `build`, `--id` on `signal`, `resolve` and `cancel`, each with the same warning. 0.5 removes them. The `ml-workflow` plugin and the build scripts under `work/build-0-2/` move to the new forms in 0.3.

Why settled: Kubernetes' deprecation policy keeps CLI elements for at least two releases; docker's flat `docker ps` still works nine years after `docker container ls` came in.

## Delivery

Tickets in build order, each one release of the CLI that works end to end:

1. **Registry and read verbs for runs.** The runtime clap tree, the kind registry with `run`, `get runs`, `describe run/X` (with `needs`, worker liveness and next commands), `get kinds`, `-o`, run prefixes and start keys, `verb.kind-mismatch`, misplaced-flag hints. About 1 day.
2. **The rest of the core kinds.** `step`, `signal`, `event`, `invocation`, `workflow`, `artifact`, `worker`, `quarantine`, `code`; `explain`; `get all`; `-o name` round trips. About 1 day.
3. **Action verbs and aliases.** `start`, `execute`, `continue`, `signal`, `resolve`, `release`, step-worker verbs, `serve`, `validate`, `build`, `cat`, `version`; the unified flags; every 0.2 command as an alias. About 1 day.
4. **`wait`, `cancel`, `-o jsonl`, dry run, exit codes 7–9, JSON usage errors.** About 1 day; `cancel` is #74.
5. **Generated reference and completion.** `help -o json`, completion scripts, `spec/cli-reference.md` generated, with a CI check that it is current. Half a day.

The `ml-workflow` plugin and `work/build-0-2/` move to the new forms with ticket 3. Plugin loading is M6 and not part of this list.

## Sources

- Kubernetes: [kubectl reference](https://kubernetes.io/docs/reference/kubectl/), [CRD printer columns](https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definitions/#additional-printer-columns), [kubectl conventions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-cli/kubectl-conventions.md), [KEP-1440 kubectl events](https://github.com/kubernetes/enhancements/blob/master/keps/sig-cli/1440-kubectl-events/README.md), [kubectl#151](https://github.com/kubernetes/kubectl/issues/151), [deprecation policy](https://kubernetes.io/docs/reference/using-api/deprecation-policy/).
- Workflow engines: [Temporal CLI](https://docs.temporal.io/cli/workflow), [Argo CLI](https://argo-workflows.readthedocs.io/en/latest/cli/argo/), [Restate invocations](https://docs.restate.dev/services/invocation/managing-invocations), [Restate CLI revamp](https://github.com/restatedev/restate/pull/5419), [Helm 4 plugins](https://helm.sh/community/hips/hip-0026).
- CLI design: [clig.dev](https://clig.dev/), Anthropic, [Writing effective tools](https://www.anthropic.com/engineering/writing-tools-for-agents) (2025-09-11), Arcjet, [Designing a CLI](https://arcjet.com/blog/designing-a-cli-for-ai-agents) (2026-06-02), Cloudflare, [Introducing cf](https://blog.cloudflare.com/cloudflare-cf-cli-launch/) (2026-09-28).
