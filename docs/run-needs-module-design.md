# What a run needs: one host module

Today two separate pieces of code decide what an unfinished run needs. The CLI has its own copy of the rule, and the host has another for `follow` and `run_until_parked`. This change moves the rule into one host module, a pure function over facts the engine gathers. `status`, `list`, `continue`, `step show`, `follow` and `run_until_parked` all ask that module. In the same pull request, the status and list output is rebuilt from one typed run view that has two adapters, JSON and text. The JSON is tidied and its format version moves from `runspore.cli/0.2` to `runspore.cli/0.3`. The plugin refuses a format it does not know. The design came out of the architecture review of 2026-10-17, where it was the top candidate, and a grilling session held on 2026-10-17 and 2026-10-18.

## Terms

- **What a run needs**: the one answer for an unfinished run, which is one of `working`, `driver`, `signal`, `child`, `stuck`, `external-ready`, `held`, `capability-missing` or `quarantined` (spec/cli.md §What a run needs). It is `None` once the run has completed, failed or been cancelled. *Avoid:* run standing, state, status.
- **Held step**: a step whose work is done outside the worker by an actor or a started job, and which has not reported yet. Its holder may report a message while it waits. *Avoid:* background step, running step, waiting for a person.
- **Run view**: the typed model of one run that `status`, `list`, `continue` and `step show` render. It is built from one read, and its JSON and text adapters render it. It is an internal name and does not go in the glossary. *Avoid:* status object (that is what the JSON adapter prints).

No new glossary term goes into CONTEXT.md. "What a run needs" already appears there under **Unfinished run**.

## Why

The architecture review found the following:
- The rule is assembled in five places across two crates.
- The pending check `applied_sequence + 1 < next_sequence` is written 7 times across the CLI, the host and the store.
- The need is a `&'static str` that is matched against string literals on 38 lines.
- The status text re-reads the JSON it just built, using string keys.

The bug fixes cluster in this rule:
- 902b31a: `until` took other runs' steps.
- 2c35814: `until` ignored child runs.
- fb6425e: the `driver` need had to be added in three files.
- c070a3c: quarantine was fixed in three renderers.
- 7ef95cd: FC-160, `scheduled`.

22 of the 26 commits that touch `view.rs` also change `host/src`. Today the truth table can be tested only by spawning `spore`, killing processes and sleeping.

## Locked decisions

### Scope: the rule and the output rework, in one pull request
- **Decision.** Move the rule into the host. Rebuild `status`, `list`, `continue` and `step show` output from one typed run view, with JSON and text as two adapters. All of it lands in a single pull request.
- **Rejected: the rule only, output unchanged.** This was the first answer. It was reopened once the JSON tidy was chosen, because a tidy with no rework has nothing to act on.
- **Rejected: two pull requests in a row.** Easier to review and to trace, but the maintainer chose one change.

### A holder's reported wait is a detail of `held`, not a new need
- **Decision.** From runspore's point of view there is no person. There are only signals, actor labels and holders, as in Temporal or n8n. The need stays `held`. Whatever the holder reported while it waits (today, a message from its watch) is carried as a detail of the answer. Who reads that detail is the caller's business.
- **Rejected: a new need value, "waiting for a person".** It puts a person into a generic orchestrator, and it conflicts with the convention backlog issue #272 already set: the need stays coarse, and a field says who is waited on.
- **Decision.** The watch line that carries the detail names no one either. A watch prints `{"waits": "<text>"}` instead of `{"needs": "person", "message": "<text>"}`, and the text lands in `step.waits` unchanged. The old line is no longer recognised: the plugin, its only writer, ships in step with this release.
- **Decision.** No output says "a person". A stuck step reads `needs resolving`, and `continue --wait` follows a run until it needs something from outside the worker or has ended.
- **Precedent.** In Temporal, a pending activity stays pending, and its heartbeat details say what it is doing.
- **Fits the neutral engine.** [docs/design/neutral-engine.md](design/neutral-engine.md) rejects a new state on a step (#234 closed as not planned, and one holder status per attempt). This change adds none: `waits` is the watch's existing report under a neutral name, and `Need` has the same values as the spec's table.
- **Related backlog.** #272 (a wait node can carry a prompt) and #273 (notify when a run parks) plug into this module rather than adding a second rule, if they pass the neutral engine's test (its L2). #299 decides whether a watch's report is stored or stays process-local.

### The JSON may change shape, and its format version moves to `runspore.cli/0.3`
- **Decision.** Tidy the status and list JSON while rebuilding it from the typed run view. Every object carries `"format": "runspore.cli/0.3"`.
- **Decision.** The plugin (`ml-workflow`, in the `max-skills` repository) is updated to the new shape. It refuses a format it does not know and says to update.
- **Rejected: keep today's JSON byte for byte.** That would keep the shape's quirks, and the maintainer chose to tidy now.
- **Rejected: print the old fields beside the new ones for one release.** It doubles the output and the tests for a reader released by the same maintainer.
- **Precedent.** A Kubernetes API version change. spore is before 1.0, and the `format` field exists for this moment.

## Routine choices

- **Where disagreements are settled.** Where today's copies of the rule disagree, spec/cli.md §What a run needs decides. Each fix lands in this pull request and is listed in its description, with no separate tickets.
- **Shape of the module.**
  - The host gets one pure function from a run's facts to its answer.
  - `Engine` gathers the facts: run view, decoded state, invocations, async step, unfinished direct children, cut-off and now.
  - The cut-off check (`host::cut_off`, which reads the OS process table) stays an input to the function, so in-process tests can supply it.
- **Engine-local stop rules stay outside the module.** `follow` and `run_until_parked` ask the module, then add one engine-local fact: whether this engine watches the held step.
  - `run_until_parked` stops at a held step this engine does not watch.
  - `follow` stops at a held step whose holder reported a wait.
  - Both keep their current meaning, now expressed over the one answer.
- **Types.**
  - The need becomes an enum whose serialized names are the spec's.
  - Status checks use the existing `RunStatus` enum, not strings.
  - Snapshot decoding happens once, in the host, with `canonical::decode`. The CLI no longer decodes snapshot bytes with `serde_json`.
- **What stays in the CLI.** The commands printed to type next (`spore signal …`, `spore resolve …`) follow the CLI grammar (C18), so they stay in the CLI.
- **The host hands over decoded facts.** Its read side returns facts the CLI needs, so the CLI no longer reads package JSON by pointer or parses event bodies:
  - the last result per node;
  - the start input (for `key_reused`);
  - a node's outcomes;
  - `maxAttempts` and loop limits from the package;
  - the full run list.

  `maxAttempts` then has one default, the build's.
- **Tests.**
  - Each row of the spec table gets a fast in-process test against the module.
  - The existing binary tests (`run_needs.rs`, `status.rs`, `coming_back.rs`) stay. They check the CLI contract, and they are updated to the new JSON shape.
  - Crash, guard and kill tests stay at binary level.
- **Spec.** spec/cli.md is changed in the same pull request: the new JSON shape, the format version, and any rule disagreements resolved.
- **The plugin.** Its update is a matching change in its own repository, `max-skills`, released together with this spore release.

## Verified facts

- **The CLI rule.**
  - `crates/runspore-cli/src/view.rs`: `needs` 503, `cut_off_of` 524, `needs_driver` 534, `needs_below` 579, `Shown::needs` 173.
  - `crates/runspore-cli/src/cli.rs:1639-1693`: `read_tree` re-composes these in its own order. It computes the cut-off only when no async step is present, which is the same as `Shown::cut_off`.
- **The host stop rules.**
  - `crates/runspore-host/src/engine.rs`: `parked` 524, `applied` 539, `waiter` 458, `until` 1316, `tree` 1362.
  - `crates/runspore-host/src/engine/steps.rs:350-376`: `waits_outside` and `needs_someone`.
- **The pending check** appears at these places:
  - `view.rs:507`
  - `engine.rs:459, 528, 539`
  - `steps.rs:351, 368`
  - `store-sqlite/src/lib.rs:1284, 1579`
- **Format version.** `crates/runspore-cli/src/cli.rs:32`, `FORMAT = "runspore.cli/0.2"`; spec/cli.md:986 says every object carries it.
- **The one known disagreement.** `continue --wait` (`follow`) stops at a held step whose holder reported a wait (`step.waits`), while `list` shows `held`. Under the held decision, the answer stays `held` with the wait carried as a detail. `follow`'s stop is the engine-local rule above. This matches spec/cli.md (`--wait` follows past held async steps until the run needs something from outside).
- **View coverage.** The only unit tests in `view.rs` cover time formatting (`utc_text`, `duration_text`).

## Risks

- A single pull request across the host and most of the CLI's status code is large. If a regression appears, it is harder to tell whether the rule or the rendering caused it. Mitigations:
  - Write the in-process table tests first.
  - Keep the binary contract tests green, apart from the intended JSON changes.
- The plugin breaks until it is updated. It must ship in step with this spore release, and refuse unknown formats with a clear message.
- Resolving disagreements in favour of the spec may change behaviour a driver relies on. Each such change is listed in the pull request.
- `cut_off` reads the live OS. Until the liveness seam (architecture review card 5) exists, the module receives the cut-off as an input, and the gathering step stays tested end to end.

## Deferred

None. The JSON-shape and pull-request questions were deferred once and answered later.

## Open threads

- **The JSON tidy list (settled during the build).** Recorded in spec/cli.md §JSON output:
  - `status` gains `needs`.
  - A `list` row's `since` becomes `updatedAtMs`, like every other time field.
  - A cancel's `at` becomes `atMs` (in `status`, `cancel` and `events`).
  - A held step's `"waiting": "person"` and `message` become one `waits`, the holder's text or null.
  - The format moves to `runspore.cli/0.3`.

  The plugin reads `since` from list rows, so it must follow; it reads none of the other renamed fields.
- **The rest of the "person" wording (follow-up).** This change removes the person from what runspore does and prints: the watch line, the status and `continue` text, and the spec passages for them. The prose elsewhere still names a person and moves to a follow-up issue:
  - CONTEXT.md (`stuck` "waits for a person's decision", `unsafe` "waits for a person");
  - host.md §2 "Telling a person" and kernel.md §1;
  - the build catalogue's messages;
  - backlog issues #272 and #273, which are reframed as signals, actors and holders.
- **The plugin change.** It needs its own ticket in `max-skills`: read `updatedAtMs`, refuse an unknown `format`, print `{"waits": …}` from its watch, and say how its oldest supported spore relates to the new format.
