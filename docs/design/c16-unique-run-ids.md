# C16 — Effect keys repeat across databases

Design record of the grill held on 2026-10-10 for the map ticket **C16 — Effect keys repeat across databases** (Web-tree/runspore#26, map #5). It decides how a step's identity stays unique beyond one database. In 0.1 a run ID is a hash of the tenant and the start key, so two databases that start a run with the same `--key` produce identical run IDs, effect keys and Claude session ids. C04 hit this: a second scratch database resumed the first database's Claude conversation. From host 0.2 on, every new run gets a **random run ID**. The start key only finds an existing run in the same database, so starting twice with one key still returns one run. Effect keys, invocation IDs and the Claude session id from C13 are all built from the run ID, so all of them become unique. The kernel is unchanged, and runs that already exist keep their IDs.

Both questions were settled from standard practice and shown on the grill page as answered, open to reopening. This follows Max's standing request to be asked only what brings value. Max read them and pressed Finish without reopening either.

## Terms

- **Start key** — The name a caller gives a run so that starting it twice gives back the same run (`spore start --key`). It names a run only inside one database: the same start key in another database starts a different run. *Avoid:* run ID, workflow ID.
- **Run ID** — A run's identity: random, made once when the run is created, and unique across databases. Every ID the run hands out (invocations, effect keys, Claude session ids) is built from it. Steps see it as `RUNSPORE_RUN_ID` and mappings as `$get ["run", "id"]`. *Avoid:* start key, execution ID.
- **Effect key** — The key a step hands to services outside Runspore so that a repeated try of the step is not carried out twice (`RUNSPORE_EFFECT_KEY`). It equals the invocation ID: the same on every try of one visit, different on every other visit (kernel §1, C13), and after C16 different in every other database. *Avoid:* idempotency token, request ID.

## Why

The ticket's question: "What makes a step's identity unique beyond one database?" Its context, from C04 (findings, §Session ids): "a second scratch database reused the key `c04-happy`. Its `implement` step computed the same session UUID, found the first database's transcript ("already in use"), and resumed a conversation about a different directory." The same hazard reaches any outside service that receives `RUNSPORE_EFFECT_KEY` as an idempotency key, which host 0.1 recommends ("an idempotent command passes it to whatever it calls"). C13 moved the Claude session id from the effect key to `RUNSPORE_RUN_ID` + `RUNSPORE_NODE_ID`, which does not help, since the run ID itself repeats. C13 left that fix to this ticket.

Spore's generated start keys are random (`key_<hex>`), so only explicit keys collide. Explicit keys are the documented way to make `start` idempotent, and a script that starts the same workflow against a fresh database will reuse them.

Max's words: on the scope of grill questions (C13, 2026-10-10, standing for this map): "can we just think about the questions we're asking as about something which really brings the value, not something that really is a standard practice in the workflow engineering". Both questions here were settled that way; Max pressed Finish.

## Locked decisions

### L1. The same start key in two databases gives two separate runs (Q1, standard practice)

**Decision.** A start key names a run only inside one database. Starting a workflow with key `K` in database A and with `K` in database B gives two runs that share nothing they hand out: different run IDs, invocation IDs, effect keys and Claude session ids. Within one database nothing changes: the same key returns the same run, and the same key with a different workflow or input is `run.exists`, as in 0.1.

This is what engines that hand out idempotency keys do. Temporal gives every run a random run ID and keys activity idempotency on the run ID plus the activity ID. Step Functions names an execution with a random UUID unless the caller names it, and its ARN carries the account and region. Stripe asks for idempotency keys with enough randomness (V4 UUIDs) that two clients never pick the same one. A workflow that wants outside services to deduplicate across databases puts its own business key in the run input and passes that on. Runspore's key covers only Runspore's retries.

Rejected:
- *B — the start key is a promise of global uniqueness, and the same key in two databases is one logical run to outside services (the stance of engines that key steps on a caller-chosen workflow ID).* A reused key then silently merges two unrelated pieces of work. That is exactly the C04 failure. The upside, deduplication after a lost database is recreated, is available under L1 by passing a business key in the input.

### L2. Each new run gets a random run ID; the start key finds existing runs (Q2, standard practice)

**Decision.** Host 0.2 drops `ids::run_from_start_key`. `start` first asks the store whether the start key already names a run in this tenant. If it does, it uses that run's ID, and the rest is 0.1's duplicate path. If not, it makes a fresh random run ID. All other derived IDs keep their 0.1 formulas, which already start from the run ID: invocation, command, attempt, effect key, and the C13 session id. So all of them become unique with no further change. The kernel is untouched: it takes the run ID as given in `run.started` (kernel §7 rule 7).

This is the Temporal model: a caller-chosen ID deduplicates starts and a random run ID identifies the run. It costs one store read in `start`, and nothing migrates. Runs created by 0.1 keep their stored, hash-derived IDs, and the lookup finds them by key like any other run.

Rejected:
- *B — a random store ID, made when the database is created and mixed into the run ID hash.* This is as small a change and keeps run IDs computable from the key. It loses on copies: a copied database file keeps its store ID, so new runs in the copy that reuse a key collide again. A per-run random ID has no such case for new runs.
- *C — only Claude steps change: the plugin stores a random session id per step instead of deriving it.* This fixes Claude conversations and leaves every outside service that receives `RUNSPORE_EFFECT_KEY` exposed. It also gives the plugin state of its own to keep.
- *A store-unique tenant (the ticket's second candidate).* The tenant is a namespace a person chooses (`EngineConfig.tenant`, default `default`). Filling it with a random value hides identity in a field meant for people, and a copied file keeps it, as in B.

## Routine choices

- **R1. Run ID format.** 128 bits from the operating system's random source, written `run_<32 lowercase hex>`. That is the same shape as 0.1's IDs and fits the kernel's `[A-Za-z0-9_.-]{1,128}`.
- **R2. A race between two starts with one key.** Both lookups miss and both make an ID. The store's existing predicate lets one `create_run` win and answers the other with `conflict` / `run.start-key-mismatch`. On that code, `start` repeats the lookup once and continues on the duplicate path.
- **R3. No migration.** Existing runs keep their IDs. A 0.1 run's effect keys therefore stay as they were, which keeps in-flight steps consistent across the upgrade.
- **R4. No new value for commands.** There is no store ID and no new environment variable. `RUNSPORE_RUN_ID`, `RUNSPORE_INVOCATION_ID` and `RUNSPORE_EFFECT_KEY` become unique on their own.
- **R5. Tests inject run IDs.** `EngineConfig` gains a run ID source, the OS random source by default. Tests give it a fixed sequence. H6 ("final state equals the uninterrupted run's") gives both runs the same ID.
- **R6. Nobody computes a run ID from a key.** `spore start` and `spore run` print the run ID that `start` returned instead of hashing the key, as the CLI does today. The plugin and the menu (C08) read run IDs from spore's output or from the store.
- **R7. Generated start keys stay as they are.** They only need to be unique within one database now.

## Spec

### Host 0.2, §2 `start(workflow_json, input, start_key)`

1. `run = find_run(tenant, start_key)`.
2. The run ID is `run`'s ID if found, else a fresh random `run_<32 hex>` (R1).
3. Build the `run.started` evidence with that run ID; canonicalize, validate and dry-run the workflow as in 0.1.
4. `create_run` with request ID `start/<start_key>`, as in 0.1:
   - `applied`: a new run, `created = true`.
   - `duplicate`: the same run, `created = false`.
   - `conflict` / `run.exists`: the key names a run with another workflow or input. Returned as in 0.1.
   - `conflict` / `run.start-key-mismatch`: another start with this key won between steps 1 and 4. Go back to step 1, once (R2).
5. Return the run key and `created`.

Replaces: "The run ID is `ids::run_from_start_key(tenant, start_key)`."

### Store 0.2

- New read: `find_run(tenant, start_key) -> Option<RunKey>`. It is read-through, with no receipt, like `get_run`. The SQLite adapter already has the `(tenant, start_key)` lookup inside `create_run`.
- `create_run` is unchanged. Its `run.start-key-mismatch` predicate is now reachable in normal use (R2), where in 0.1 it took a hash collision.
- Store conformance: a case where two `create_run` calls carry one start key and two run IDs. One wins, the other gets `run.start-key-mismatch`, and `find_run` returns the winner.

### CLI 0.2

- `start`/`run`: "the same key always names the same run" reads "the same key always names the same run in this database".
- Output reports the run ID returned by `start` (R6).

### The C04 case, replayed

| | Database A | Database B |
| --- | --- | --- |
| `spore run --key c04-happy` | `run_3f…` (random) | `run_a9…` (random) |
| `implement` effect key | `inv_…` from `run_3f…` | `inv_…` from `run_a9…`: different |
| Claude session id (C13: run + node) | from `run_3f…` | from `run_a9…`: different, so B starts a fresh conversation |
| `spore run --key c04-happy` again in A | same run `run_3f…`, `created = false` | — |

## Verified facts

- `ids::run_from_start_key(tenant, start_key)` = `run_` + the first 32 hex of `hash("run", [tenant, startKey])`. Invocation and command IDs hash `[tenant, runId, activationId(, ordinal)]`, and the effect key equals the invocation ID (`crates/runspore-types/src/digest.rs`). Nothing outside the run's own ID varies between databases.
- `run_from_start_key` is called only by the host's `start` (`crates/runspore-host/src/engine.rs`) and by the CLI's `key_run`, which prints the run ID (`crates/runspore-cli/src/cli.rs`).
- `RunStarted` carries `tenant`, `run_id` and `input` (`crates/runspore-types/src/model.rs`). The started digest therefore depends on the run ID, so a retried `start` must reuse the existing run's ID to hit `create_run`'s duplicate path. Step 1 does this.
- SQLite `create_run` already looks up `SELECT run_id FROM runs WHERE tenant = ? AND start_key = ?` and answers `run.start-key-mismatch` when the key names another run.
- Generated start keys are `key_<16 hex>` from a randomly seeded hasher over time and PID (`generated` in `cli.rs`).
- The kernel validates only the run ID's characters and length (kernel §7 rule 7) and never derives it.
- Temporal's and Step Functions' identity models, and Stripe's key guidance, are as summarized in L1. They are stated from their public documentation, not re-checked in this session.

## Changes outside C16

- `spec/host.md` 0.2 §2: the `start` procedure above. `spec/store.md` 0.2: `find_run`, and the conformance case. `spec/cli.md` 0.2: the wording in CLI 0.2.
- Map, fog "how the spec is split": C16 adds host 0.2 random run IDs, store 0.2 `find_run`.
- C13's record ([c13-bounded-loops.md](c13-bounded-loops.md), Risks) names C16 as the owner of the session-id collision. This record is that fix; the C13 derivation (run + node) stands unchanged.
- `CONTEXT.md`: **Start key** and **Effect key** added.

## Risks

- **A copied database copies its runs.** Continuing one run in both the original and a copy runs its steps twice, under the same effect keys and the same Claude conversation. A copy of a run is the same run, and no identity scheme separates them. New runs started in a copy are separate (L2).
- **Run IDs are no longer computable from keys.** Any tool that hashed a key to find a run breaks. Only the CLI does this today (R6). A plugin written against 0.1 must use the IDs spore prints.
- **An outside service that deduplicated by effect key across databases loses that.** This is intended (L1). A workflow that relied on it moves to a business key in its input.

## Deferred

None.

## Open threads

None.
