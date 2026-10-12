# C21: the records, cross-checked

Findings for the map ticket [C21 — Cross-check the records: one change list per spec file](https://github.com/Web-tree/runspore/issues/47) (map #5). Read on 2026-10-12 at `main` 17d9715: `CONTEXT.md`, `spec/*.md` (0.1), every record in `docs/design/` (C02, C03, C06–C17) and `docs/research/` (C04, C05, `claude-code-mods.md`, `cli-name.md`), the [C01 findings][c01] on branch `research/workflow-map-carryover`, the other files in `docs/`, `work/`, and the two READMEs. The spec-addition inventory in the ticket's comment was a starting point; §6 lists where it differs from the records.

Facts only. Nothing here decides; [C22](https://github.com/Web-tree/runspore/issues/48) does. Rule IDs are the records' own (C13 R2 = rule R2 of the C13 record; L = locked decision, R = routine choice). `file:line` is at 17d9715.

## Summary

- **Change items:** `kernel.md` 18, `host.md` 37, `store.md` 13, `cli.md` 25, new `dot.md` 12, new `artifact.md` 8, the schema subset 8 (its own `schema.md` or a `kernel.md` section; placement is open), the plugin's spec in `max-skills` 25, and `spec/README.md` 4. That makes 150 in total.
- **Contradictions with no later decision:** 9 (U1–U9 in §2.2). Another 5 gaps were never decided (U10–U14), and 12 routine items are left for C22 (§2.3).
- **Stale statements with a later decision:** 53 places in the records (§2.1) and 38 in docs outside `spec/` (§4).
- **Terms used two ways:** 20 (§3). `CONTEXT.md` is also missing 27 terms the records define.
- **Concurrency:** on one machine, with one OS user and one database per checkout, any mix of drivers is safe. Several machines, two repositories sharing one database on purpose, another OS user, who is *allowed* to claim, and how later steps run inside a worktree a step made are all undecided (§5).

## 1. Change lists, one per spec file

### 1.1 `spec/kernel.md` 0.2 (18)

| ID | Change | Source |
|---|---|---|
| k.1 | Semantics `0.2`; package format `runspore.workflow/0.2` accepted beside `0.1` (W01) | [C13][c13] R2, §Package W01 |
| k.2 | A 0.2 kernel runs 0.1 packages "as 0.1 does" (`loopLimits`, `exhausted` reservation) | [C13][c13] R2 (see U2) |
| k.3 | Stuck step: *fail the invocation* with no `failed` route sets `inv.state = stuck`, status `needs-intervention`, and emits diagnostic `invocation.stuck` | [C07][c07] L3, §Kernel 0.2 |
| k.4 | `invocation.resolved` accepts `stuck`: `retry` ignores `maxAttempts`, `complete` as 0.1, `fail` is terminal with the kept error | [C07][c07] §Kernel 0.2 |
| k.5 | Limit and mapping errors stay terminal | [C07][c07] §Kernel 0.2; [C13][c13] L1 |
| k.6 | Golden traces for stuck after a non-retryable failure, after exhausted retries and after exhausted expiries; resolve from stuck; a drawn `failed` route still taken | [C07][c07] §Kernel 0.2 |
| k.7 | Node field `loopLimits` on `activity` and `await-signal`, values 1..=999 | [C13][c13] L2, R1, R3, R6 |
| k.8 | Reserved outcome `exhausted` in 0.2; W07, W08 and W09 amended; *consume* treats a signal with outcome `exhausted` as unrouted | [C13][c13] §Package, §Routing |
| k.9 | W12: limit + 1 ≤ `maxVisitsPerNode`; a node with `loopLimits` routes `exhausted` | [C13][c13] R5, §Package |
| k.10 | State gains `streaks`; a bad streak is `state.invalid` | [C13][c13] §State |
| k.11 | The *Route* procedure; diagnostic `loop.exhausted`; the recorded result stays the real one | [C13][c13] §Routing, R4 |
| k.12 | Property K10 and its traces | [C13][c13] §Routing |
| k.13 | Node field `outputSchemas` on `activity` and `await-signal`, 0.2 only, never read in a transition; W13 | [C12][c12] L2, R6, R9, §Package |
| k.14 | Top-level package field `inputSchema` (0.2), covered by the digest | [C08][c08] R6, §inputSchema |
| k.15 | W14 (every node is reachable from `start`) and W15 (an end is reachable from every node), 0.2 only | [C09][c09] R7, §validate |
| k.16 | Mapping context `from` (`{node, outcome, output}` or `null`); W10 admits `from` as a path's first step | [C11][c11] L1, §Kernel 0.2 |
| k.17 | `$get` with `default`; W10 allows exactly the keys `$get` and `default`; one more operation | [C11][c11] R11 |
| k.18 | Golden traces for `from` and `default` | [C11][c11] §Kernel 0.2 |

`{"$outputSchemas": {}}` left in a package is already rejected by 0.1's W10 (`$` keys); [C14][c14] R8 relies on that, so the kernel needs no change for it.

### 1.2 `spec/host.md` 0.2 (37)

| ID | Change | Source |
|---|---|---|
| h.1 | Load an artifact: instantiate it, read `manifest()`, check `format` and `dialect`, read `blob(main)` and check its digest, store the package and the artifact; the run records both digests | [C02][c02] §Host load sequence, R6 |
| h.2 | An artifact is untrusted code: sandbox it like the kernel | [C02][c02] Risks, L8 |
| h.3 | Reject unsafe manifest paths on load and again before each write (`artifact.invalid-path`) | [C14][c14] R11 |
| h.4 | `start`: `find_run` first, else a random `run_<32 hex>`; on `run.start-key-mismatch` look up once more; `ids::run_from_start_key` is dropped | [C16][c16] L2, R1, R2 |
| h.5 | `EngineConfig` gets a run-ID source (fixed in tests) | [C16][c16] R5 |
| h.6 | `start` checks input against `inputSchema` (`input.invalid`) | [C08][c08] R6 |
| h.7 | `base_dir` is the folder holding `.spore/` when the database was found by walk-up; it is recorded on the run and every dispatch uses it | [C11][c11] R1 |
| h.8 | Handshake: a capability set; `validate` and `start` reject mismatches and name the fix; one result channel is picked and recorded; checked again before dispatch, claim, probe and watch; `capability-missing` replaces 0.1's non-retryable `host.runner-missing` | [C03][c03] L4, §Handshake, Open threads |
| h.9 | `files_dir` config; `start` rejects a workflow with instructions or asset entries when it is unset | [C14][c14] §Host |
| h.10 | `validate_workflow` checks schemas against the subset (`workflow.invalid-schema`), `inputSchema` included | [C12][c12] §Host; [C08][c08] §inputSchema |
| h.11 | Action kind `external` (fields `actor`, `async`, `effect`, `outcomes`, `retry`); the worker never claims or runs it | [C03][c03] L1, §Action fields |
| h.12 | The `async` object: `timeoutMs` (1 h), `leaseMs`, `probe`, `watch`, `channels` (`["store"]`), and `stop` | [C03][c03] R6, §Action fields; [C08][c08] L4 |
| h.13 | Tagged references `{"command": {…}}` or `{"component": {…}}` for probe, watch and stop; `component` is refused until a host advertises it | [C07][c07] R4; [C08][c08] §spore cancel |
| h.14 | A `./` probe or watch is bundled as an action asset; a bare name is looked up on the host | [C07][c07] R8 |
| h.15 | Async start: input on stdin, `RUNSPORE_TOKEN`, `RUNSPORE_RESULT_CHANNEL`; on exit 0, stdout (≤ 4 KiB canonical JSON) is the holder handle; other results map through the 0.1 table | [C03][c03] §The async attempt |
| h.16 | The start command of a repeatable async action must be repeatable by its effect key | [C15][c15] L5a |
| h.17 | Probe contract (`{"holder": alive\|dead\|unknown}`, 10 s timeout, only `dead` frees); probes on claim, `status`, `continue`, and in the sweep at most every 30 s | [C03][c03] L3, §2, §Probe and watch |
| h.18 | Watch contract; one watch per held attempt under its own limit; watches stop on shutdown | [C03][c03] R3, §2 |
| h.19 | A watch may print a non-final `{"needs": "person", "message"}` line | [C06][c06] §CLI 0.2, Changes |
| h.20 | `command` result mode `output: "result"` | [C07][c07] R3 |
| h.21 | `command.undeclared-outcome` (not retryable) for every command | [C07][c07] R7 |
| h.22 | Text output `{exitCode, stdout, stderr, truncated}`, keeping each stream's end (stderr ≤ 16 KiB); never `command.output-too-large` in text mode | [C11][c11] L2 |
| h.23 | Output-schema check: sync and watch results become `output.invalid` (not retryable); `signal`, `resolve --complete` and step completion are refused and append nothing; error details format | [C12][c12] L1, R1, R2, R9, R10 |
| h.24 | Step completion refuses an undeclared outcome and needs canonical output ≤ 64 KiB | [C03][c03] R9 |
| h.25 | `RUNSPORE_VISIT` for every command | [C13][c13] R8 |
| h.26 | `RUNSPORE_STEP_FILES`, `RUNSPORE_INSTRUCTIONS`, `RUNSPORE_WORK_DIR` for the command, async start, probe and watch | [C14][c14] R5, R7 |
| h.27 | `RUNSPORE_DB` for every program the host starts, `async.stop` included | [C08][c08] §The project folder |
| h.28 | Step files: layout, made by rename, checked before every use, file modes, quarantine with `artifact.unreadable` | [C14][c14] R2–R4, R10, §Host |
| h.29 | A work folder per invocation, mode 0700, never removed in this version | [C14][c14] R7, §Host step 5 |
| h.30 | The sweep removes step files under a per-artifact lock | [C14][c14] R4, §Host step 6 |
| h.31 | An `argv[0]` of `./<asset>` (and a probe's or watch's `./…`) is rewritten to the step-files copy | [C14][c14] R5 |
| h.32 | A guard in front of every command, async start, probe and watch | [C15][c15] L2, §Host |
| h.33 | Every stop is SIGTERM, then `stop_grace_ms` (5000), then SIGKILL; this replaces 0.1's immediate SIGKILL | [C15][c15] L4 |
| h.34 | `set_process` before the program runs; cleared when the command ends and by `hold_attempt` | [C15][c15] §Host |
| h.35 | Before a lease expiry, stop a live recorded group on this host and boot; another host or EPERM expires with `process.unchecked` | [C15][c15] L3, L6 |
| h.36 | After `cancel_run`, run `async.stop` once (holder handle on stdin, C14 environment); its output is late evidence and its failure is ignored | [C08][c08] §spore cancel |
| h.37 | Required evidence H6–H11, H7b included (the IDs clash with 0.1's H6, see U9) | [C15][c15] §Evidence |

### 1.3 `spec/store.md` 0.2 (13)

| ID | Change | Source |
|---|---|---|
| s.1 | A content-addressed artifact table; the run's `artifact_digest` | [C02][c02] R6 |
| s.2 | An attempt's `holder` (≤ 4 KiB or null) and `deadlineMs`; `claim_attempt` takes both; an external step's owner is a fresh ID per claim | [C03][c03] §Store 0.2 |
| s.3 | `hold_attempt` | [C03][c03] §Store 0.2 |
| s.4 | `heartbeat` serves as renew, capped at `deadlineMs` | [C03][c03] §Store 0.2 |
| s.5 | `expire_attempt` takes a reason: `lease`, `probe-dead`, `abandoned` or `cancelled`. Only `lease` needs `now ≥ leaseUntilMs`. The reason, and `process.unchecked`, go in the audit record | [C03][c03] §Store 0.2; [C08][c08] §spore cancel; [C15][c15] L6 |
| s.6 | `scan_due` also expires attempts past `deadlineMs` | [C03][c03] §Store 0.2 |
| s.7 | `InvocationView` shows the running attempt's full reference (`attemptId`, `owner`, `fence`) | [C03][c03] §Store 0.2 |
| s.8 | The run keeps the result channel picked for each async action | [C03][c03] L4, §Store 0.2 |
| s.9 | `cancel_run(key, reason, requestId)`, status `cancelled`, the refusals (`run.cancelled`, `lease.lost`) and seven conformance rows | [C08][c08] L4, §spore cancel |
| s.10 | `find_run(tenant, start_key)` and a conformance case for a start-key race | [C16][c16] §Store 0.2 |
| s.11 | An attempt's `process` and `set_process`; shown in `InvocationView` and in `scan_due` lease items | [C15][c15] §Store 0.2 |
| s.12 | The run's `baseDir` | [C11][c11] R1, §Store 0.2 |
| s.13 | The schema guard (0.1 `schema.too-new`) says which version wrote the database; exit 10 | [C10][c10] R6 |

### 1.4 `spec/cli.md` 0.2 (25)

| ID | Change | Source |
|---|---|---|
| c.1 | `validate`, `start` and `run` take a folder, `workflow.dot` or an artifact; JSON files are refused | [C02][c02] R2, §CLI 0.2 |
| c.2 | `build <folder> [-o file] [<composed flag>]`, writing `<folder>.wasm` beside the folder by default | [C02][c02] R3, R8 |
| c.3 | `inspect <artifact>` for debugging, not a stable interface | [C02][c02] R2 |
| c.4 | `inspect --run <runId> --file <path>`; exit 2 when the file is not in the artifact | [C17][c17] L3 |
| c.5 | Build and validate errors name the file and line; `workflow.invalid-schema` | [C02][c02] §CLI 0.2; [C12][c12] §CLI |
| c.6 | Database resolution: `--db`, `$RUNSPORE_DB`, walk-up to `.spore/runspore.db`, `./runspore.db`; `db` in `status --json` and `list --json` | [C08][c08] R4, §The project folder |
| c.7 | `spore init` | [C08][c08] §The project folder |
| c.8 | Writing commands keep the ignore file up to date; only writing commands create a database | [C08][c08] §The project folder |
| c.9 | A start key names one run "in this database"; `start` and `run` print the returned run ID | [C16][c16] §CLI 0.2, R6 |
| c.10 | `--input` is checked against `inputSchema` (exit 2, `input.invalid`) | [C08][c08] R6 |
| c.11 | `step claim [<runId>] [--actor] [--holder]` and its JSON, incl. `resultChannel`, `files` and `instructions`; text mode prints the instructions | [C03][c03] R1, R2, §CLI 0.2; [C14][c14] R6 |
| c.12 | `step renew`, `step complete`, `step fail`, `step abandon` | [C03][c03] R4, R5, R7, §CLI 0.2 |
| c.13 | `step show [<runId>]` and the step description; exit 2 off an activity or await-signal node; exit 10 on `artifact.unreadable` | [C14][c14] R6, §CLI 0.2 |
| c.14 | `capabilities --json`: action kinds, async kinds, channels, `version`, `contracts` | [C03][c03] §Handshake; [C10][c10] R3 |
| c.15 | The `status --json` 0.2 shape: the `step` object ([C03][c03] R2), the kept error at a stuck step ([C07][c07] §Kernel 0.2), `db` ([C08][c08]), `process` ([C15][c15] §CLI), `position.from`, `invocation.maxAttempts`, `streaks`, `end`, `artifactDigest` ([C17][c17] L3), status `cancelled` ([C08][c08]) | as listed |
| c.16 | `status --no-probe` | [C17][c17] L3 |
| c.17 | `continue [<runId>]`: none, one or several unfinished runs; the held-step steps; what it prints | [C08][c08] R2, §spore continue |
| c.18 | `continue --wait` | [C06][c06] §CLI 0.2 |
| c.19 | `cancel <runId> [--reason] [--id]` | [C08][c08] §spore cancel |
| c.20 | `list --unfinished`; rows gain `workflow`, `nodeId`, `needs`, `since` and `actor`, newest first | [C08][c08] §spore list |
| c.21 | `needs: working` when a matching signal is already stored | [C11][c11] R5 |
| c.22 | `run` and `worker --until-parked` return at an async step | [C03][c03] R2 |
| c.23 | Exit codes: 6 ([C03][c03] R2, given more meanings by [C06][c06]); 7 ([C08][c08]); 2 for `output.invalid`, `input.invalid` and an undeclared outcome ([C12][c12] R2, [C08][c08] R6, [C03][c03] R9); 4 at a stuck step ([C07][c07]); 10 for a newer store schema and for `step show` artifact errors ([C10][c10] R6, [C14][c14] R6) | as listed |
| c.24 | `--stop-grace-ms` for `worker`, `run` and `continue` | [C15][c15] §CLI 0.2 |
| c.25 | JSON format `runspore.cli/0.2` | [C14][c14] §CLI; [C17][c17] §CLI |

### 1.5 `spec/dot.md`, new (12)

| ID | Change | Source |
|---|---|---|
| d.1 | Strict grammar: what is accepted, what is rejected (with a line number); an unknown attribute is an error; the `dot` binary is never required | [C02][c02] R1 |
| d.2 | The render-attribute allowlist (an initial list; whether it is final is open) | [C02][c02] §`workflow.dot`, Risks |
| d.3 | Attributes with meaning: graph `dialect` (required, checked first) and `start`; node `kind` (default `activity`); edge `on` (default `ok`); `label` is render-only | [C02][c02] L7, R5 |
| d.4 | Dialects `runspore/0.1` and `runspore/0.2`; the dialect must agree with the settings `format` | [C02][c02] R5; [C13][c13] R2 |
| d.5 | ID and outcome patterns; at most 256 nodes; the graph id is the workflow name | [C02][c02] §`workflow.dot`, R4 |
| d.6 | `on="exhausted"` (0.2); `on="failed"` routes a failure | [C13][c13] §Package; [C02][c02] §`settings.json` |
| d.7 | Workflow folder: fixed file names; entry is the folder or `workflow.dot`; only declared files are built in; references stay inside the folder | [C02][c02] R4 |
| d.8 | Settings shape (the package minus topology), plus `instructions` on nodes and `assets` on actions, which the build strips | [C02][c02] L6, R4 |
| d.9 | Settings 0.2 fields: `loopLimits`, `outputSchemas` (inline or a file path), `inputSchema` | [C13][c13] R1; [C12][c12] R6; [C08][c08] R6 |
| d.10 | Build-only forms: the `$outputSchemas` marker, and `$defs`/`$ref` in schemas | [C14][c14] R8; [C12][c12] §Schema subset |
| d.11 | Build errors: C02's list, a `$outputSchemas` marker on a node with no schemas, a bad schema | [C02][c02] R7; [C14][c14] R8; [C12][c12] R10 |
| d.12 | Build steps: merge, strip references, inline schema files and `$ref`s, resolve the marker, bundle `./` probes and watches | [C02][c02] R4; [C12][c12] R6; [C14][c14] R8; [C07][c07] R8 |

### 1.6 `spec/artifact.md`, new (8)

| ID | Change | Source |
|---|---|---|
| a.1 | WIT world `runspore:artifact@0.1.0`, interface `reader` (`manifest`, `blob`); imports nothing | [C02][c02] L5, L8 |
| a.2 | Manifest `runspore.artifact/0.1`: `dialect`, `name`, `main`, `entries[{path, digest, kind, owner}]` | [C02][c02] R9 |
| a.3 | `executable: true` on an entry | [C14][c14] R9 |
| a.4 | `workflow.dot` and `settings.json` ride along as `source` entries; `inspect --run --file` reads them | [C02][c02] R9; [C17][c17] L3 |
| a.5 | The composed artifact exports both worlds unchanged (a fixed WAC composition holding the certified kernel bytes) | [C02][c02] L9, R3 |
| a.6 | The package digest is identity; the artifact digest is provenance | [C02][c02] R6 |
| a.7 | Entry-path rules (`artifact.invalid-path`) | [C14][c14] R11 |
| a.8 | `./` probe and watch files are action-owned `asset` entries, even though no `assets` list names them | [C07][c07] R8; [C14][c14] R5 |

### 1.7 The schema subset: `spec/schema.md` or a `kernel.md` section (8)

[C12][c12] §Spec names both homes; the choice is open (C22).

| ID | Change | Source |
|---|---|---|
| sc.1 | The keyword table (JSON Schema 2020-12 meanings) | [C12][c12] R6, §Schema subset |
| sc.2 | Integers only; `number` rejected | [C12][c12] §Schema subset; [C04][c04] recipe |
| sc.3 | Depth counted from the package root | [C12][c12] §Schema subset |
| sc.4 | The minimum-serialised-size rule (≤ 65536 bytes) | [C12][c12] §Schema subset |
| sc.5 | `$defs`/`$ref` in the authoring form only, inlined by the build; remote references rejected | [C12][c12] §Schema subset |
| sc.6 | Error code and format: `output.invalid`, `{outcome, errors[{path, message}]}`, JSON Pointer paths, at most 16 errors, `(root)` in text | [C12][c12] R10 |
| sc.7 | An `inputSchema` root must be `type: "object"` | [C08][c08] §inputSchema |
| sc.8 | `workflow.invalid-schema` for a schema outside the subset | [C12][c12] R10; [C08][c08] §inputSchema |

### 1.8 The plugin's spec, `plugins/ml-workflow/docs/spec.md` in `max-skills` (25)

This repo's spec must name no Claude ([C10][c10] R4). Everything below goes to `max-skills`.

| ID | Change | Source |
|---|---|---|
| p.1 | Home and packaging: `ml-workflow` in `max-skills`; the Bun helper `claude-spore` in `bin/`, its code in `src/`; terminal only | [C10][c10] L1, L3, R10 |
| p.2 | Minimum versions: Claude Code 2.1.290 and Bun 1.2.0, and where each is checked | [C10][c10] L3, R12 |
| p.3 | Installing and updating `spore`: offer the line and run it only on a yes; the helper checks the signature and the checksum | [C10][c10] L2, R2 |
| p.4 | Version check against `spore capabilities --json`: the oldest supported spore, a range per contract | [C10][c10] R3 |
| p.5 | The helper outside Claude: the launcher; the mod calls the helper by `$.plugin.root`; on a helper/mod version mismatch, `/reload-plugins` | [C10][c10] R5; [C05][c05] |
| p.6 | Permissions: `allowed-tools` per skill, plus the README's standing rules | [C10][c10] R11 |
| p.7 | Entry points: sub-skills `menu`, `start`, `status`, `continue`, `cancel`, `author`; the pop-up opens from `skill.prompt` or the `open_menu` tool | [C06][c06] L4, routine |
| p.8 | The menu widget: a pane with a `Select` | [C05][c05] Summary 1 |
| p.9 | A one-line notice at session start | [C08][c08] R1, named per [C06][c06] L4 |
| p.10 | Following a run: a background `continue --wait --json` and what to do on each exit code | [C06][c06] L3, §Plugin |
| p.11 | Menu rows: Continue on `held`, Decide with `--id <activationId>` after showing the latest results, View, "change a workflow" | [C08][c08] §Plugin; [C11][c11] R3, R4, R12; [C17][c17] L2; [C09][c09] R2 |
| p.12 | Start flow: `claude-spore check`, then an input draft per `inputSchema` property | [C08][c08] R5, R9 |
| p.13 | Profile kinds and how each maps to actions | [C07][c07] L1, §The mapping |
| p.14 | Node parameters: the closed set `format`, `schema`, `outcome`, `model`, `tools`, `skill`, `context` | [C10][c10] R7; [C14][c14] L1–L3 |
| p.15 | What `claude-spore check` checks | [C07][c07] R10; [C12][c12] R5; [C14][c14] §Plugin |
| p.16 | Background step (`claude-spore run`), including `--tools`/`--allowedTools` with `--permission-mode default`, integer-only output, and denials reported but not failed | [C07][c07]; [C12][c12] §Plugin; [C13][c13] §Plugin; [C14][c14] §Plugin; [C11][c11] R7; [C04][c04] recipe |
| p.17 | Interactive step: `start`, `probe`, `watch` (with the `needs` line), `report` and `stop`; the holder handle `{herdrAgent, session, pid, started, boot, machine}` | [C07][c07] R15; [C11][c11] R6; [C12][c12] R7; [C06][c06]; [C08][c08] L4 |
| p.18 | Session ID from the run and the node, and which message to send | [C13][c13] L3, R7, §Plugin |
| p.19 | Repair budget (2), `MAX_STRUCTURED_OUTPUT_RETRIES=3`, `output.missing` | [C12][c12] R3, §Plugin |
| p.20 | Files in the work folder: `<attempt>.result.json`, `.answers`, `.facts.json`, `params.json` | [C14][c14] §Plugin; [C12][c12] R7, R8 |
| p.21 | Defaults the interview writes: `idempotent`, 3 tries, 1 h, 4 h or 7 d limits, 3 tries on repeatable commands, no `model`, the edits and reads tool sets, the `summary` schema | [C07][c07] R11–R13; [C11][c11] R2; [C10][c10] R8, R9; [C12][c12] R5 |
| p.22 | The authoring interview, `/ml-workflow:author [<workflow>]`, `references/authoring.md`, `from` and `lastTest` in `context` | [C09][c09]; [C06][c06] L4; [C10][c10] R7; [C11][c11] L1, R11 |
| p.23 | Actor label `person` | [C10][c10] R7 |
| p.24 | Run line, live view, `claude-spore draw`, the picture check, a 1 s `status --no-probe` poll | [C17][c17] L1–L5 |
| p.25 | The reference loop is the profile's check | [C11][c11] |

### 1.9 `spec/README.md` (4)

| ID | Change | Source |
|---|---|---|
| r.1 | "A user writes a workflow as one JSON document" (line 10) | [C02][c02] R2 |
| r.2 | The out-of-scope list (lines 34–38) loses cancellation, output (and run-input) schema checks, and the DOT frontend | [C08][c08] L4; [C12][c12]; [C08][c08] R6; [C02][c02] |
| r.3 | Decisions M01 (one document) and M12 (no cancellation) are superseded | [C02][c02]; [C08][c08] L4 |
| r.4 | The file list gains `dot.md`, `artifact.md` and maybe `schema.md` | [C02][c02] §Spec; [C12][c12] §Spec |

## 2. Contradictions and stale statements between records

### 2.1 Stale, with a later decision (53)

| Where | Says | Superseded by |
|---|---|---|
| c02:40 (L1) | A prose file exists only for an external step | [C07][c07] §The mapping; [C14][c14] R1 (instructions for every kind) |
| c02:267, 280 | Nothing delivers instructions and assets at dispatch | [C14][c14] |
| c02:125, 238 | `inspect <artifact>` is the only inspect form | [C17][c17] L3 adds `--run --file` |
| c03:3, 41 | The kernel stays at semantics 0.1 | [C07][c07] L3, [C13][c13], [C12][c12], [C09][c09], [C11][c11] (kernel 0.2) |
| c03:12 | Term "in-session step" | [C07][c07] L2 (interactive step) |
| c03:51, 60, 171–175 | The reference `implement` is `external`, `unsafe`, claimed by actor `coding-session` | [C07][c07] R11, Changes (3); [C11][c11] (background `command`, `idempotent`) |
| c03:110, 117–118, 125 | `"probe": {"argv": …}` | [C07][c07] R4 (tagged form) |
| c03:131, 212 | A watch runs only while a worker runs | [C06][c06] (`continue --wait` runs watches) |
| c03:142 | The watch contract has no `needs` line | [C06][c06] |
| c03:89, 213 | Instruction delivery is open | [C14][c14] R6 |
| c03:194, 211 | The in-session probe and handle are open | [C07][c07] R9; [C11][c11] R6 |
| c03:214 | Output schemas are open | [C12][c12] |
| c06:112 | "cli.md §Signals" | `spec/cli.md` calls the section "Interrupts" (naming only) |
| c07:3, 19, 43–45, 90, 94, 98, 147–159, 184–185 | Claude settings live in instruction frontmatter | [C14][c14] L3 (node parameters) |
| c07:102, 249 | Session UUID from the effect key | [C13][c13] L3; [C16][c16] |
| c07:240 | "A loop back is a new conversation" | [C13][c13] L3 |
| c07:103, 192 | Claude writes `<runtime dir>/<session uuid>.result.json` | [C12][c12] R7 (report command); [C14][c14] R7 (work folder) |
| c07:108 | Profile spec `spec/claude-code.md` | [C10][c10] R4 |
| c07:190, 244 | Q20 (interactive holder handle) deferred | [C11][c11] R6 |
| c07:235 | Watches need a running worker | [C06][c06] |
| c07:251, 252 | Labels, names and the key list are provisional | [C10][c10] R7; [C14][c14] L3 |
| c08:100, 218, 270, 290 | The menu is `/spore` | [C06][c06] L4 (`/ml-workflow:menu`) |
| c08:171, 234 | `continue` prints `spore step claim <runId>`; a person claims with plain `spore step claim` | [C03][c03] R1 (no `--actor` takes only unlabelled steps) with [C10][c10] R7 (label `person`); [C06][c06] §Plugin adds `--actor <label>` |
| c08:173, 235 | Foreground or background is C06's | [C06][c06] L3 |
| c08:218, 289 | The widget is open (`$.ui.ask`, a pane…) | [C05][c05] (a pane with `Select`) |
| c08:230 | Decide sends a signal with no `--id` | [C11][c11] R4 |
| c08:232 | "Open its pane" waits for Q20 | [C11][c11] R6 |
| c08:233 | A `held` row has no Continue | [C11][c11] R3 |
| c08:226–237 | Rows have no View | [C17][c17] L2 |
| c08:239–245 | No "change a workflow" | [C09][c09] R2 |
| c08:265 | "ADR M12 (docs/02)" | M12 is in `spec/README.md:55`, not `docs/02` (pointer only) |
| c09:68, 69 | Skill `spore-author`; command forms `new`, `change <name>` | [C10][c10] R7; [C06][c06] L4 (`/ml-workflow:author [<workflow>]`) |
| c09:70 | Hand-off by `$.prompt.submit` | [C05][c05] (`$.command.run`) |
| c09:80 | Draft line "sees: task, test output, rejection reason" | [C11][c11] Changes ("why it is back", `from`) |
| c09:167 | `spec/claude-code.md` describes the interview | [C10][c10] R4 |
| c10:109 | The mod registers a command `spore` | [C06][c06] L4 |
| c10:162 | Risk: the menu's typed name is unknown | [C06][c06] (closed) |
| c12:57, 191 | Files under `<runtime dir>` | [C14][c14] R7 |
| c12:54 | Every outcome of a Claude step needs a schema | [C14][c14] Changes (only for `format: "json"`) |
| c12:60, 246 | Run-input schemas are deferred | [C08][c08] R6 (`inputSchema`) |
| c12:225 | `spec/README`: inputs stay out of scope | [C08][c08] R6 |
| c12:166 | Schemas reach the wrapper "through the channel C14 settles" | [C14][c14] L1, L2 (`schema` parameter) |
| c12:12, 253 | Names are provisional | [C10][c10] R7 |
| c13:158 | One result file per visit | [C12][c12] R7 (per try); [C14][c14] |
| c13:176, 197 | Run ID is a hash of the start key; collision risk | [C16][c16] |
| c13:206 | A frontmatter switch for a fresh conversation | [C14][c14] L3 (no frontmatter) |
| c14:152, 216 | Parameter names and the role of `context` are provisional | [C10][c10] R7 |
| c04:55, 89 | `permissionMode` and `allow` are frontmatter keys | [C14][c14] L3; [C10][c10] R7 closed set (neither key is in it); [C11][c11] R7 (`--permission-mode default`) |
| c04:57, 73–75 | Session ID from the effect key; needs a store-unique part | [C13][c13] L3; [C16][c16] |
| c04:54 | `cwd` relative to where `spore` started | [C11][c11] R1 |
| c04:81 | `<runtime dir>/<session uuid>.envelope.json` | [C14][c14] R7 (work folder; the file name stays plugin-internal) |
| c05:40, 48 | The mod's command is `/spore` | [C06][c06] L4 |
| C01 rows L05, W08 | The in-session step is locked; run ID comes from the start key | [C07][c07] L2; [C16][c16] |

The other changes C06 and C17 list for earlier records ("Changes to earlier records") were never written into those files. This is why c08 and c10 still show `/spore`, no View, and no Continue on `held`.

### 2.2 Contradictions with no later decision (U1–U9: questions for Max or C22)

| ID | Contradiction | Where |
|---|---|---|
| U1 | **Is a cancel an event?** `spore cancel` "appends a cancel event", while `cancel_run` "records `cancelled {reason, at}` on the run", the kernel is unchanged, and pending events "stay unconsumed". No event kind for it exists in kernel 0.1 §5. | c08:184 vs c08:86, 186–192 |
| U2 | **Kernel 0.2 on 0.1 workflows and runs.** C07 L3 makes the stuck pause "the default for every step whose node has no `failed` path" and says L3 "changes every workflow". C13 R2 says a 0.2 kernel runs 0.1 packages "as 0.1 does" (stated for `loopLimits` and `exhausted` only). Kernel 0.1 checks 1 and 5 reject any other semantics or state version. No record says whether 0.1 packages get stuck-step semantics, how a run started under 0.1 resumes under 0.2, or how a 0.1 store migrates (C10 R6 only refuses a *newer* schema; C08 `init` only prints file moves). | c07:76, 237; c13:69; kernel.md §7.1 |
| U3 | **Worktree isolation vs the recorded base dir.** C08 L3 and C09 make isolation a workflow step that creates a worktree, and say "its steps work in a worktree". C11 R1 runs every dispatch from the run's recorded base dir. `cwd` is a static action field, and the closed parameter set has no working directory (C10 R7). No record says how later steps, Claude steps included, run inside a worktree an earlier step made. | c08:73–77; c09:115; c11:128; host.md §3 |
| U4 | **What a session death records.** C11's table says "your session dying records it as stopped at once". C15 says a dead worker's try records nothing and its lease runs out, and that a closed terminal (SIGHUP) kills the worker "without a graceful shutdown". Handling SIGHUP like SIGTERM is left as an optional refinement. | c11:88, 93 vs c15:25, 71 |
| U5 | **`async.stop` and the guard.** C08 runs `stop` with "the C14 environment" and `RUNSPORE_DB`. C14's variable list names only the command, async start, probe and watch. C15's guard and stop sequence cover the same four, not `stop`. Whether `stop` is guarded, and what its timeout is, is not stated. | c08:151, 193; c14:91; c15:79 |
| U6 | **File references in `settings.json`.** C02 R4's strip-before-validate rule (`instructions` on nodes, `assets` on actions) is "my resolution; Max did not rule on it" (the alternative was a top-level `files` section). Every later record (C07, C11, C12, C14) writes the R4 form. | c02:128, 278 |
| U7 | **Who may act on a step.** In the profile an approval is a plain `spore signal` (C07 R1, C11), and any caller may claim any label, `abandon`, `resolve` or `cancel`. `docs/11` says "a plain graph signal saying approved is not sufficient" and that an approval binds an authenticated actor. `docs/14` says `abandon` needs "stronger permissions and an expected revision". No record addresses either. (C18 and C20 are open.) | docs/11-security.md:30; docs/14-api-and-operations.md:5; c03:88, 92 |
| U8 | **Which run when `<runId>` is left out.** `step claim` takes "the first ready one" in the whole database. `step show` takes "the only unfinished run". `continue` lists runs and exits 2 when there are several. | c03:160; c14:111; c08:161 |
| U9 | **Evidence IDs.** C15 numbers its evidence H6–H11 (with H7b). Host 0.1 already has H6 ("crash at every failpoint"), which C16 R5 and `work/tasks.md:30` cite. | c15:95–105; host.md:213; c16:49 |

### 2.3 Gaps nobody decided

Questions (U10–U14):

| ID | Gap | Where |
|---|---|---|
| U10 | The base dir when the database comes from `--db` or `RUNSPORE_DB` (for example a `spore start` run inside a step). R1 keeps 0.1's current directory; "a flag may be wanted". | c11:202 |
| U11 | What `status --json` shows for a `cancelled` run. `end` is defined only for `completed` and `failed`. | c17:46, 87 |
| U12 | An interactive step's permission mode. C11 R7 covers only `claude-spore run`, and C04's `permissionMode`/`allow` are outside C10's closed set (C20 is open). | c11:134; c10:116 |
| U13 | Several machines, two repositories sharing a database on purpose, and another OS user. See §5. | c15:61 |
| U14 | Whether a `spore` 0.2 build still accepts `dialect="runspore/0.1"` folders. C02 R5 defines 0.1, and C13 R2 says a 0.2 kernel accepts 0.1 packages, but JSON packages are no longer an input (C02 R2). | c02:129; c13:69 |

Routine, for C22 from standard practice (12):
- The render allowlist (c02:160).
- The composed-build flag and file name (c02:282).
- `inspect` spelling and output (c02:283; c17:91).
- `step show` field names, and whether it probes (c14:215).
- `start` with a bare workflow name (c08:291).
- The limit on concurrent watches (c03:131).
- Failpoints for the new store operations (`cancel_run`, `hold_attempt`, `set_process`; no record names them).
- Where `list`'s `since` comes from (c08:178; no store field).
- Whether C12's minimum-size rule also counts the `$literal` schema copy in a node's input (c14:201).
- Whether schema files get manifest entries (c12:116; C02 R9 lists only `source`, `instructions` and `asset`).
- `pattern` in the subset (c12:250).
- Where release signing and `install.sh` are specified (C10 R2; no `spec/` file owns releases).

## 3. Terms used two ways (20)

| Term | The two (or more) uses |
|---|---|
| runtime dir / work folder | `<runtime dir>` in C04:81, C07:192, C12:57, 191, 229 and C13:158; `RUNSPORE_WORK_DIR` and "work folder" in C14 R7 and `CONTEXT.md:111` (which lists "runtime dir" under *Avoid*) |
| bundle | C02 Terms avoid it for folder and artifact; `CONTEXT.md:109` avoids it for step files; jco JavaScript output in docs/01:23 and docs/04:5 ("generated target bundles"); "verified kernel bundle" in work/tasks.md:57; a verb for building into the artifact (C07 R8, C14 R3) and for inlining `$ref`s (docs/06:39) |
| artifact | the kernel component ("certified artifact", spec/README.md:50 M08; "kernel artifact", docs/04:5); C02's workflow and composed artifacts; a logical store record with publisher and locator (docs/07:22); Git snapshots as "authoritative artifacts" (docs/13:19). Spelled `artifact_digest` (C02 R6) and `artifactDigest` (C17) |
| package | the canonical JSON document (kernel.md §2, C02 Terms); a container holding a manifest, IR, schemas, assets, a lockfile and a source map (docs/04:5, docs/01:5); `register_package` and `inspect-package` (docs/14) |
| frontmatter / node parameters | Claude settings in instruction frontmatter (C07 R2, R6; C04:55; C13:206) vs node parameters in `input` (C14 L1–L3, C10 R7, `CONTEXT.md:115`) |
| in-session / interactive; headless / background | C03 Terms and file name `c03-in-session-hand-off.md`, C01, C12:18 vs C07 Terms; C04's title and file name `c04-headless-agent-step.md`, C12:18 vs "background step" |
| `steps/` | the workflow folder's instructions subfolder (C09:132, C11:32) vs the step-files tree `<files_dir>/steps/` (C14 R4). C11's files therefore land at `…/steps/<digest>/implement/steps/implement.md`. C02, C07, C12 and C14 examples use `instructions/` instead |
| integration / profile / plugin / mod / helper | "the Claude Code integration" (C07, C13, C15 L5); "profile" (the map, C01); plugin, mod and helper (C10, `CONTEXT.md`). `CONTEXT.md` defines no "integration" or "profile" |
| cancel | a store-level terminate with no clean-up (C08 L4, `CONTEXT.md:49`) vs a durable request with a dispatch generation and compensation (docs/06:33) |
| read-only | an effect class; "read-only commands" that never create a database (c08:150), a list that includes `status`, which may still expire an attempt through a probe (C03 §2, C17 L3); read-only step files (C14 R2). C06 L2's read-only list leaves `status` out |
| heartbeat | `heartbeat` (store.md §2) vs `heartbeat_attempt` (c08:190, docs/09:5) vs `step renew` (C03) |
| watch | an action's wait on a holder handle (C03, `CONTEXT.md:99`) vs "following a run" (C06 avoids "watching") vs "where to watch it" (c08:121) and "starts, watches and resumes runs" (`CONTEXT.md:122`) |
| what a run needs | `step.state` `ready\|held\|capability-missing` (C03 §CLI); `needs` `external-ready\|held\|…` (C08); `needs: "held"` with `waiting: "person"` (C06). Exit 6 has three meanings (C06 table) |
| session | a Claude session; a herdr-managed session; an OS session (the C15 guard's "session and group"); "two sessions sharing one database" in the map's concurrency question |
| decision | a person's answer at an `await-signal` node (C08 Decide, C11) vs the kernel's transition result (kernel.md, host.md §2) |
| holder handle / process record | kept apart in C15 Terms, but C11 R6 puts PID, start time and boot into a holder handle, "C15's process record, kept by the action" |
| graph-embedded component | "pack command … standalone workflow.wasm" (docs/01:7) and "graph-embedded component" (docs/05:5) vs C02's composed artifact (C02 Terms avoid "workflow.wasm") |
| owner | the store's attempt owner vs "who owns the loop" (C06 avoids "owner"; `CONTEXT.md:89` avoids it for Holder) |
| try, visit / attempt, invocation, activation | the records use the plain words, the specs use the spec words; `CONTEXT.md` defines neither set |
| ADR | A-decisions in docs/02 vs M-decisions in spec/README; c08:265 places M12 in docs/02 |

Terms the records define that `CONTEXT.md` lacks (27):
- C02: Package, DOT dialect, Workflow folder, Settings file, Workflow artifact, Composed artifact, Kernel component.
- C03: Token.
- C06: Following a run, Sub-skill.
- C07: Result mode.
- C10: Oldest supported spore.
- C11: Reference loop, Arrived from (`from`), Coming back.
- C12: Report command, Run facts.
- C13: Retry, Exhausted arrow, Streak, Safety net.
- C14: Step description.
- C15: Guard, Process record.
- C16: Run ID.
- C17: Live view, Run line. (Dot drawing is plugin-only.)

## 4. Docs outside `spec/` that still say the old thing (38)

| File:line | Says | Now |
|---|---|---|
| CONTEXT.md:68 | An external step's actor may be "a Claude session" | Generic and still true, but after C07 L2 the profile uses external steps only for a person's manual work (C07 Terms) |
| CONTEXT.md:76 | The actor label is "on an external step" | It is on the action (C03 R1) |
| CONTEXT.md:121–123 | The mod lets a person "watch" runs | C06 avoids "watching" for following |
| CONTEXT.md:133–135 | The entry menu: runs, workflows, "writing a new one" | It also offers "change a workflow" (C09 R2) and View (C17) |
| docs/01:7 | "optional pack command … standalone workflow.wasm" | The composed artifact via `spore build` (C02 L9, R3) |
| docs/01:23 | "trusted generated JS bundles" | A term clash (§3) |
| docs/01:30 | Pin the graph, kernel, schemas, bindings… | A run pins the package and artifact digests, not the kernel (C02 R6; see U2) |
| docs/03:21 | `adapter-claude` in this repo's TypeScript tree | The plugin lives in `max-skills` (C10 L1) |
| docs/04:5 | A package holds a manifest, assets, a lockfile…, and optionally a self-contained component | Package = canonical JSON; the manifest and files live in the artifact (C02 L5, R9) |
| docs/04:7 | A run pins activity implementation digests | Bare-name probes and the helper are not pinned (C07 R8, C10 L3) |
| docs/05:5 | "a graph-embedded component is the alternative" | The composed artifact (C02 L9) |
| docs/06:5 | JSON is the first language; DOT is import-only | The DOT folder is the authoring input, and JSON is not public (C02 R2) |
| docs/06:33 | Cancellation as a durable request with a generation and compensation | A store-level close, Temporal's *terminate* (C08 L4) |
| docs/06:41 | Validate mapped input before dispatch | Deferred (C12 R11) |
| docs/06:41 | Repair as a new activation | Repair inside the try (C12 R3) |
| docs/06:41 | Commands get no repair "by default" | Never (C12 R4) |
| docs/08:14 | "Agent or skill" binding kind | The kinds are words; Claude steps are `command`s (C07 L1) |
| docs/08:15 | Human: "durable request and authenticated response" | `await-signal` or `external`, unauthenticated (C07 R1; U7) |
| docs/11:30 | A plain signal saying approved is not enough | The profile's approval is a plain signal (U7) |
| docs/13:5 | "Keep the projects separate" | Partly stands again (C10; map Notes) |
| docs/13:7 | Preserve cmd/skill/agent/human binding kinds | Words only (C07 L1) |
| docs/13:7 | The importer normalizes bounded retry edges | `loopLimits` (C13) |
| docs/13:9 | A zero-dependency plugin; the Bun planner can stay agent-driven | Superseded (map Notes; C01 L05) |
| docs/13:9 | A controlled mode with `get_ready`/`claim_action`/… | `spore step claim\|complete\|fail` (C03 L1) |
| docs/13:15 | A supervised or deliberately launched worker owns the loop | Nobody owns it (C06 L1, L2) |
| docs/13:21–30 | The eight-step delivery flow | Out of scope (map) |
| docs/13:32 | A stopped session leaves the run waiting or progressing "according to its selected mode" | Closing Claude pauses the run (C06 L3); a crash stops leftovers (C15 L1) |
| docs/14:5 | `abandon` needs stronger permissions | Any caller can run `step abandon` (C03 R5; U7) |
| docs/14:9 | CLI `compile`, `inspect-package` | `build` and `inspect` (C02) |
| docs/17:13 | M4: an MCP bridge and a coding pipeline with "no duplicate PR" | The reference loop has no PR (map); the plugin is in `max-skills` (C10) |
| docs/17:19 | P13 "implements the skills and Claude profile" here | The profile spec is in `max-skills` (C10 R4) |
| docs/sources.md | Nothing superseded | Sources the records cite are not listed (Attractor spec, herdr docs, Claude Code plugin docs and changelog, Temporal/Step Functions/Stripe pages, W02 notes) |
| work/tasks.md:30 | "evidence H6" | The ID clashes with C15 (U9) |
| work/tasks.md:55 | "Cancellation (spec decision M12)" left for later | Decided for 0.2 (C08 L4) |
| work/tasks.md:57 | "verified kernel bundle" | A term clash (§3) |
| README.md:5, 20–26; examples/README.md:15–25 | JSON workflows as CLI input; cancellation and schemas "not implemented" | Accurate for 0.1; becomes stale with CLI 0.2 (C02 R2) |
| docs/research/claude-code-mods.md:231–239 | The mod submits steps to the person's session or `$.agent.spawn`, and reports through a `complete_step` tool | Inferred in 2026-10-06; superseded by C07 L2 and C05 |
| docs/research/cli-name.md:57 | `spore run x.json` | Illustrative; JSON input goes away (C02 R2) |

C02's open thread (c02:284) worried that uncommitted "bundle" edits to `CONTEXT.md`, docs/01, 04, 05, 06, 17, `sources.md` and `work/tasks.md` would land. None did: those files have no C02-era "bundle" text. The hits above are older senses of the word.

## 5. Concurrency: two sessions or repositories sharing one database, and who may claim what

### What the records answer

- **One database, many processes on one host.** One SQLite file; WAL; `synchronous = FULL`; a busy timeout of at least 5 s; every mutation in `BEGIN IMMEDIATE`; `store.busy` is retried identically. The capabilities say `multiworkerClaims = true` for the same host. Receipts, revision compare-and-swap and fences make competing coordinators collapse into one result (store.md §1, §3; host.md H1, H2).
- **Any mix of drivers on one run is safe.** Claims, leases and fences decide who does each try. When two watches race, the store keeps the first result and the other becomes late evidence. There is no owner per run ([C06][c06] L1, which rejected a stored owner).
- **On one machine, one try runs at a time.** The guard stops a dead worker's command group, and the sweep stops a recorded live group before expiring its attempt ([C15][c15] L1–L4). Step files are made by rename and swept under a per-artifact lock ([C14][c14] R4).
- **Where the database is.** One `.spore/` per checkout, so two checkouts never share runs ([C08][c08] L1). Several unfinished runs may share a checkout, and keeping them apart is the workflow's job (L2, L3). Steps inherit `RUNSPORE_DB`, so a step in a worktree reaches its own run's database (§The project folder).
- **Run identity.** A start key names a run only within one database. A copied database copies its runs, and continuing one run in both copies runs its steps twice (accepted risk) ([C16][c16] L1, Risks).
- **Which external steps a claimer takes.** `--actor <label>` takes only steps with that label; no `--actor` takes only unlabelled steps (Max did not rule on this default). The claim token is the attempt, so only its bearer can renew, complete or fail it ([C03][c03] R1, Terms, Open threads).
- **A process record from another machine or OS user.** It cannot be checked; the attempt expires by lease, with `process.unchecked` ([C15][c15] L6).

### What they leave undecided

1. **Several machines on one database.** C15 L6 explicitly "left open; joins the map's open question". Step-file paths are local, and delivery to another machine is deferred ([C14][c14] Risks, Deferred). A probe from another machine answers `unknown` ([C11][c11] R6). A Claude step whose conversation is on another machine fails without retry ([C13][c13] §Plugin). store.md §3 says "never on a network filesystem (documented, not detected)", and docs/01:9 adds PostgreSQL "before supporting multiple worker machines". The map's Out of scope does not list it.
2. **Two repositories sharing one database on purpose** (`--db`, `RUNSPORE_DB`). Every CLI run uses tenant `default`, so the repositories share one start-key namespace: the same key returns the other repository's run, or `run.exists` ([C16][c16] L1). `worker`, and `step claim` with no `<runId>`, act on every run in the database (U8), and the menu would list the other repository's runs. The base dir for such runs is an open thread (U10). No record allows or forbids this.
3. **Another OS user on the same machine.** Work folders are mode 0700 for whoever made them ([C14][c14] §Host step 5), and process records answer EPERM, so the attempt expires as `process.unchecked` ([C15][c15] L6). Nothing says whether this is supported.
4. **Who is allowed to claim.** The label is a filter, not an authorization. Anyone who can open the file can claim any label, `signal`, `resolve`, `abandon` or `cancel`. A claim records only a fresh owner ID and an opaque holder handle, so no actor identity is kept (U7; C18 and C20 are open).
5. **Two runs in one checkout.** The isolation C08 L3 points to (a worktree step) has no mechanism that sends later steps into the worktree (U3).
6. **Mixed `spore` versions on one database.** A newer schema is refused ([C10][c10] R6). Upgrading a 0.1 database and resuming 0.1 runs under 0.2 are unspecified (U2).
7. **Unmeasured:** write contention from many pollers (1 s `status --no-probe` per visible run in C17; 200 ms reloads in `continue --wait` in C06) under the 5 s busy timeout; a locked database overrunning the 10 s `session.start` hook budget ([C08][c08] Risks).

**Verdict:** the records fully answer two drivers or sessions sharing one checkout's database on one machine under one OS user. They do not answer items 1–6. Item 4 (who may claim what) is only a filter today.

## 6. The ticket's inventory comment against the records

- C15: the comment says "evidence H6–H10". The record has H6–H11 with H7b, and adds L5a (repeatable async start).
- C14: the comment puts the "build-only `$outputSchemas` form" in `artifact.md`. The record puts it in the build and settings (C14 §Build and artifact). The manifest only gains `executable`.
- C10: release signing, `install.sh` and the minimum Claude Code and Bun versions belong to no `spec/` file. The minimums go to the plugin's spec; release signing has no named home.
- Items the comment does not list, all included above:
  - C02 host load sequence and artifact sandboxing (h.1, h.2).
  - C03 `RUNSPORE_TOKEN`/`RUNSPORE_RESULT_CHANNEL`, the full attempt reference in `InvocationView`, and the result channel kept per run (h.15, s.7, s.8).
  - C07 the `invocation.stuck` diagnostic and the stuck golden traces (k.3, k.6).
  - C08 the ignore-file rules, `db` in JSON, and exit 2 for several runs (c.6, c.8, c.17).
  - C12 the W01 restriction and the subset check in `validate_workflow` (k.13, h.10).
  - C13 the W07–W09 amendments and streak checks (k.8, k.10).
  - C14 `artifact.invalid-path`, `files_dir` required for embedders, and the per-artifact lock (h.3, h.9, h.30).
  - C16 the run-ID source (h.5).
  - C17 `invocation.state` showing `stuck` (c.15).

[c01]: https://github.com/Web-tree/runspore/blob/research/workflow-map-carryover/docs/research/workflow-map-carryover.md
[c02]: ../design/c02-dot-format.md
[c03]: ../design/c03-in-session-hand-off.md
[c04]: c04-headless-agent-step.md
[c05]: c05-spike-mod.md
[c06]: ../design/c06-loop-owner.md
[c07]: ../design/c07-four-kinds.md
[c08]: ../design/c08-entry-menu.md
[c09]: ../design/c09-authoring.md
[c10]: ../design/c10-plugin-home.md
[c11]: ../design/c11-reference-loop.md
[c12]: ../design/c12-output-schemas.md
[c13]: ../design/c13-bounded-loops.md
[c14]: ../design/c14-step-files.md
[c15]: ../design/c15-leftover-command.md
[c16]: ../design/c16-unique-run-ids.md
[c17]: ../design/c17-live-run-view.md
