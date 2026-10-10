# C12 — Output schemas on agent steps

Design record of the grill held on 2026-10-10 for the map ticket **C12 — Output schemas on agent steps** (Web-tree/runspore#17, map #5). It settles whether and how a step's result data is checked against a declared shape. A workflow may declare an **output schema** for each way a step can end: in `settings.json`, on the node, keyed by outcome (`"outputSchemas": {"changes": {...}}`). Spore checks every result against it the moment the result arrives, for every kind of step. A command or watch result that does not fit becomes a failure the step cannot recover from, so the step waits for a person or takes its `failed` arrow (C07 L3). A reporter who is still there (`spore step complete`, `spore resolve --complete`, `spore signal`) is refused on the spot, so they can correct the answer. On the profile side, a Claude step must declare a schema for each of its outcomes. Claude repairs a bad answer in the same conversation, told exactly what is wrong, up to two times, before spore ever sees it. A background step does this through `claude -p --json-schema`, an interactive step through a report command that checks the answer. Plain commands never get repair. The kernel changes only its package format (a node field and one validation rule); the check itself lives in the host.

Two rules held throughout. Runspore stays generic: the check, the schema language and the error code say nothing about Claude, and Claude's own enforcement and repair live in the plugin. And, per Max's standing request from C13, questions with a standard answer were settled rather than asked: Q1–Q5 were recorded on the page as settled from standard practice or from Max's own decisions on the old skills-repo map, open to reopening; Max reopened none. Q6 was Max's choice.

## Terms

- **Output schema** — The shape a step's result data must have when the step ends with a given outcome, declared by the workflow on the node. Spore checks each result against it the moment the result arrives. A result with no declared schema is not checked. *Avoid:* contract, type, output mapping (that is `output` on a `complete` node, kernel §2).
- **Outcome** — The name a step's result routes by: `ok`, `changes`, `red`, `approved` (kernel §2). Each outcome may have its own output schema (L2). On the grill page: "the way a step ends". *Avoid:* result (the outcome plus its data), status.
- **Repair** — Claude fixing its own answer in the same conversation after being told exactly why it does not fit the schema. It happens inside one try; spore sees only the final answer. *Avoid:* retry (a new try of the same visit), loop-back (a new visit), rework.
- **Report command** — The plugin command an interactive step's Claude runs as its last act to hand in its answer: it checks the answer against the step's schemas and writes the result file only when it fits (R6). Name provisional (C10). *Avoid:* result file (what it writes), `step complete` (spore's own command for external steps).
- **Stuck step** — A step that cannot recover and has no `failed` arrow; the run waits at it for a person (C07 L3). An answer that does not fit, after repair, makes a step stuck. *Avoid:* failed step.
- **Run facts** — What a Claude try reports about itself rather than about the work: cost, turns, duration, tools that were denied. Kept beside the result, not in it (R8). *Avoid:* output, metadata.

## Why

The ticket asked: "Does the profile validate an agent step's output against a declared schema, and how, given that a headless `claude -p --json-schema` enforces for real while an in-session subagent enforces nothing (W02 in the old map)? Settle whether validation happens in `spore` before the outcome is accepted, what a missing or invalid output becomes (an outcome, a retry, a failure), and whether a repair budget exists." It added: "Runspore stays generic: if `spore` validates outputs, it validates any external step's output against a declared schema; the Claude-specific enforcement (`--json-schema`, repair) is the plugin's concern."

What made it urgent: in 0.1 nothing checks a step's data. A step can hand the next one an answer missing the field it needs, and a mapping that reads a missing field (`mapping.missing-path`) ends the run as terminal `failed`, which C07 keeps terminal and nothing can catch. Since C07 L2 the in-session subagent is gone (every Claude step runs in a session the integration starts), so the remaining forms are `claude -p` (enforces) and an interactive herdr session (enforces nothing by itself).

Max's words during the grill, all on Q6 (the only open question):

- "does it mean per result or per step?" — the first wording ("one shape per result, or one per step?") was unclear; "result" meant the outcome.
- "can you write it in the simple language? Like really simple" — the question was rewritten as: "The review step ends in one of two ways: 'looks good' or 'needs changes'. Must 'needs changes' come with the list of changes?"
- Accepted A: each way a step can end has its own answer format.

## Locked decisions

### L1. Spore checks every step's result against the schema the workflow declares (Q1, standard practice)

**Decision.** Checking is a generic Runspore capability, not a plugin feature. A workflow may declare output schemas on any `activity` or `await-signal` node. The host checks each result the moment it arrives: a sync command's or native function's output, a watch's result, `spore step complete`, `spore resolve --complete` and `spore signal`. Schemas are optional in spore: a node, or an outcome, with none is not checked. Claude Code's own enforcement still runs on top, in the plugin (R3, R5).

Sources: docs/06 already states the target design ("Validate untrusted producer output at ingestion"), and typed engines such as Dagster check outputs at the step boundary. The old map's L14 ("every boundary declares a schema") and L20/W06 (missing vs invalid) point the same way (C01).

Rejected:
- *B — only Claude steps are checked, inside the plugin.* It would leave commands, people's answers and external actors unchecked, and it would put a general capability into the Claude-only half, against the map's rule that Runspore stays generic.

### L2. Each outcome has its own schema (Q6 → A, Max)

**Decision.** A node's output schemas are keyed by outcome: `"outputSchemas": {"changes": {...}, "ok": {...}}`. When a step ends with `changes`, its data must fit the `changes` schema; when it ends with `ok`, the `ok` schema. An outcome with no entry is not checked by spore. In Max's plain-words version: if the review step says "needs changes", it must also list the changes, and if it does not, spore catches it and Claude fixes its answer.

The cost is one schema per outcome to write instead of one per step. The authoring interview (C09) writes them. For Claude, the plugin joins them into one `anyOf` under a fixed root object, which stays inside what Claude Code enforces by grammar (R5).

Rejected:
- *B — one schema for the whole step, whatever the outcome.* A field that only one outcome needs, such as the list of changes, could then only be optional, so "needs changes" could arrive with nothing to work on and the next step's mapping would end the run.

## Routine choices

- **R1. A result that does not fit is a failure the step cannot recover from (Q2, standard practice).** After the existing outcome check (C07 R7), the host checks the data against the outcome's schema. A result that does not fit becomes `Failure`, code `output.invalid`, not retryable, with the errors in its details. Kernel 0.2 then does what C07 L3 says: the node's `failed` arrow if the author drew one, else the step is stuck and waits for a person. A new try would not help: a command gives the same output again, and Claude's repair happens before spore sees the answer (R3). An outcome such as `invalid` was rejected because it would put a routing decision into every graph for what is a broken step. A reporter who is still present is refused instead (R2). This matches C07 R7's `command.undeclared-outcome` and a failed type check in typed engines.
- **R2. People and actors are refused on the spot.** `spore step complete`, `spore resolve --complete` and `spore signal` check the data before writing anything. Data that does not fit exits 2 (usage or validation error) and lists the errors; nothing is recorded, the attempt stays held, the run does not move. This is C03 R9's rule for an undeclared outcome, applied to data.
- **R3. Claude repairs a bad answer, up to two times, then the step waits for a person (Q3, carried over from the old map's L21; standard practice).** The repair happens inside one try, in the same conversation, with Claude told what is wrong: the first answer plus at most two repairs. If the third answer still does not fit, the try ends with a non-retryable failure (`output.invalid`), so the step is stuck or takes its `failed` arrow. Claude Code's own `--json-schema` loop, Pydantic AI and Instructor work the same way. The old map's L21 set "budget 2; an exhausted budget becomes `failed`"; under C07 L3 "failed" now means "waits for a person" unless a `failed` arrow is drawn. Rejected: no repair (the first bad answer stops the step), and unlimited repair until the time limit (burns tokens on a model that is confused).
- **R4. Plain commands never get repair (old map L22).** docs/06 says commands receive no model-based repair "by default". In this profile it is never: a command's output is checked (R1), and that is all.
- **R5. Every outcome of a Claude step has a schema (Q4, carried over from the old map's L14).** Spore keeps schemas optional (L1). The plugin's check (`claude-spore check`, C07 R10, name provisional C10) refuses a workflow in which a Claude step has an outcome without a schema, and the step refuses it again when it starts, as a backstop. For an outcome with nothing structured to say, the authoring interview writes a default: `{"type": "object", "properties": {"summary": {"type": "string"}}, "required": ["summary"], "additionalProperties": false}`. One rule follows: a Claude step's output is always its checked answer, never its final message as free text, so the plugin has one code path and every hand-off between Claude steps is checked.
- **R6. Schemas sit on the node, in a JSON Schema subset (Q5, standard practice).** On the node, not the action, so that switching a Claude step between background and interactive changes only the action (C07 R6), and so that `await-signal` nodes, which have no action, use the same field. The subset is docs/06's list plus what Claude Code can enforce by grammar (W02), restricted to spore's integer-only data (Spec, "Schema subset"). A schema is written inline in `settings.json` or as a path to a JSON file in the workflow folder; the build inlines files and local `$ref`s, so the package, and therefore the run's identity (package digest), always holds the full schema. A schema change is a new package; a running run keeps the schemas it started with.
- **R7. How an interactive step reports (amends C07 R15).** Claude no longer writes the result file directly. Its last act is to run the report command with its outcome and answer. The command checks the answer against the step's schemas. If it fits, it writes the result file (named after the invocation, C13) and says so. If not, it prints each error (path and message) and exits non-zero, so Claude sees the errors as tool output and fixes its answer in the same turn. After the third answer that does not fit (R3), it writes a failure result instead and tells Claude to stop. This is the same pattern as Claude Code's own `StructuredOutput` tool, whose errors come back as the tool's result (W02). R15's watch is unchanged: quiet plus a result file. A session that goes quiet without one is still read as waiting for the person.
- **R8. Run facts stay out of the output (my default).** C04's wrapper printed `session`, `result`, `turns`, `durationMs`, `costMicroUsd` and `deniedTools` as the step's output. Now a Claude step's output is exactly its checked answer, so mappings read `nodes.review.output.requests`, and run facts cannot break a schema. The background wrapper writes the run facts of each try to `<runtime dir>/<RUNSPORE_INVOCATION_ID>.<RUNSPORE_ATTEMPT>.facts.json`. How the mod shows them, including denied tools, which C04 says must be reported and not failed, belongs to the map's fog item on progress and evidence display. A generic "report beside the result" field in spore was considered and left for that fog item: run facts are Claude-specific today.
- **R9. Signals use the same schemas.** An `await-signal` node may declare `outputSchemas` for the outcomes it routes. A signal's data becomes that node's output (kernel §7.4), so it is checked like any other result. Signals are addressed to a run by name and may arrive before the run reaches the node, so `spore signal` checks against every `await-signal` node of the run's package that waits for that name. W13 requires those nodes to declare the same schema for each outcome they share, so the check has one answer.
- **R10. Error codes and error text.** The host's code is `output.invalid`, with details `{"outcome", "errors": [{"path", "message"}]}`, at most 16 errors, all collected rather than stopping at the first. A path is written `$.requests[2].file`; a message names what was expected and what arrived, such as `expected type "string", got integer (42)`, the format of the old map's W02 validator. The plugin adds `output.missing` for "Claude ended without giving an answer at all" (W02's null-payload case), which W06 asked to keep apart from "invalid" because the repair message differs. Errors the build finds in a schema itself are `workflow.invalid-schema`.
- **R11. Run input and step input are not checked yet.** docs/06 also asks to check a step's mapped input before dispatch. With every Claude step's output checked (R5), a mapping that reads a Claude step's answer reads checked data. Schemas for the run's input at `start` and for mapped inputs are left for later (Deferred).

## Spec

Normative wording to carry into `spec/kernel.md` 0.2 (with C07's and C13's changes), the package format `runspore.workflow/0.2`, `spec/host.md` 0.2, `spec/cli.md` 0.2 and the profile spec. Examples are illustrative; C11 writes the reference loop.

### `settings.json`

The settings are an excerpt: they show what C12 adds and leave out the `actions` table and the DOT (C02, C07).

```json
{
  "format": "runspore.workflow/0.2",
  "nodes": {
    "review": {
      "action": "review",
      "instructions": "instructions/review.md",
      "outputSchemas": {
        "ok": "schemas/summary.json",
        "changes": {
          "type": "object",
          "properties": {
            "requests": {
              "type": "array",
              "minItems": 1,
              "items": {
                "type": "object",
                "properties": {
                  "file": { "type": "string" },
                  "change": { "type": "string" }
                },
                "required": ["file", "change"],
                "additionalProperties": false
              }
            }
          },
          "required": ["requests"],
          "additionalProperties": false
        }
      }
    },
    "approve": {
      "signal": "approval",
      "outputSchemas": {
        "rejected": {
          "type": "object",
          "properties": { "reason": { "type": "string", "minLength": 1 } },
          "required": ["reason"],
          "additionalProperties": false
        }
      }
    }
  }
}
```

A string value is a path relative to the workflow folder. The build resolves it under C02's rules (a path, `..` or symlink that escapes the folder is a build error), parses it as JSON, and puts the object in the package. The package never holds a path.

### Schema subset

A schema is a JSON object using only these keywords, with JSON Schema 2020-12 meaning:

| Keyword | Rule |
| --- | --- |
| `type` | One of `object`, `array`, `string`, `integer`, `boolean`, `null`. A single string, not an array: unions use `anyOf`. `number` is rejected: spore's data has integers only (kernel §1). |
| `properties`, `required` | Only with `type: "object"`. Every `required` name is a key of `properties`. |
| `additionalProperties` | Only `false`. Absent means extra keys are allowed, as in JSON Schema; Claude Code closes every object anyway (Risks). |
| `items` | Only with `type: "array"`; a single schema. |
| `enum`, `const` | Strings, integers, booleans or `null`; `enum` values unique and non-empty. |
| `anyOf` | A non-empty list of schemas, with no other keyword beside it except `description` and `title`. |
| `minLength`, `maxLength`, `minItems`, `maxItems`, `minimum`, `maximum` | Non-negative integers (`minimum`/`maximum`: any integer within ±(2^53 − 1)), on the matching type. Enforced everywhere; in Claude Code they cost the grammar guarantee but are still checked (W02). |
| `description`, `title` | Strings. Carried into Claude's schema as guidance. |
| `$defs`, `$ref` | In the authoring form only: `$ref` must be `#/$defs/<name>`, must not recurse, and the build inlines it. The package holds no `$ref`. |

Nesting depth is at most 32 (kernel §1). Any other keyword (`pattern`, `oneOf`, `allOf`, `if`, `format`, remote `$ref`) is `workflow.invalid-schema` at build time.

### Package and validation (kernel 0.2)

An `activity` or `await-signal` node may carry `outputSchemas`: an object from outcome to schema. The kernel never reads it during a transition.

- **W01.** `outputSchemas` is allowed only in `runspore.workflow/0.2` (C13 R2).
- **W13 (new).** `outputSchemas` appears only on `activity` and `await-signal` nodes and is non-empty. Each key is an outcome the node can produce: one of its action's `outcomes` for an activity node, a key of `outcomes` for an await-signal node, never `failed` or `exhausted` (the kernel makes those outputs). Each value is a JSON object. Await-signal nodes that wait for the same `signal` declare equal schemas for every outcome they share.

The kernel checks only the shape of the field. The schema subset is checked by the host's `validate_workflow` (host §2), which `start` and `spore build` already run, so a bad schema fails the build with `workflow.invalid-schema`, the path of the offending keyword and the node and outcome it belongs to.

### Host 0.2: the check

- **Sync results.** When a runner returns `Success { outcome, output }` (any action kind), the host checks the outcome as C07 R7 says. Then, if the node declares a schema for that outcome and `output` does not fit, the host turns the result into `Failure { error: output.invalid, retryable: false }` (R10) before `finish_attempt`.
- **Watch results.** Checked the same way before they are recorded.
- **`validate_workflow`.** Also checks every schema against the subset.
- The kernel is unchanged in what it decides: it sees a non-retryable failure and applies C07 L3 (the `failed` arrow, else a stuck step).

### CLI 0.2

- `spore step complete <token> --outcome <o> [--output <json>]`, `spore resolve <run> <invocation> --complete <o> [--output <json>]` and `spore signal <run> <name> [--outcome <o>] [--data <json>]` check the data against the schema of that outcome (for `signal`, against the await-signal nodes that wait for `<name>`, R9). Data that does not fit exits 2 and prints the errors; nothing is recorded. A missing `--output` or `--data` counts as `null`.
- `spore build` fails with `workflow.invalid-schema` on a schema outside the subset or a file it cannot read.

### Plugin side: background step (`claude-spore run`)

This amends C07's background procedure, step 3.

1. The wrapper reads the node's outcomes and output schemas, through the same channel C14 settles for the instructions. Nothing is pushed to ordinary commands, so C07 R7 holds.
2. It builds one schema for `claude -p --json-schema`, with an `anyOf` branch per outcome inside a fixed root object, since Claude Code's strict form needs an object at the root and allows `anyOf` only as a lone keyword (W02):

   ```json
   {"type": "object", "additionalProperties": false, "required": ["answer"],
    "properties": {"answer": {"anyOf": [
      {"type": "object", "additionalProperties": false, "required": ["outcome", "output"],
       "properties": {"outcome": {"const": "changes"}, "output": <changes schema>}},
      {"type": "object", "additionalProperties": false, "required": ["outcome", "output"],
       "properties": {"outcome": {"const": "ok"}, "output": <ok schema>}}
    ]}}}
   ```

3. It runs Claude with `MAX_STRUCTURED_OUTPUT_RETRIES=3`, so Claude Code allows the first answer and two repairs (R3).
4. `structured_output` present: it prints `{"outcome": answer.outcome, "output": answer.output}` in result mode (C07 R3), after C04's integer rule (an integral float becomes an integer). The host checks it again (R1); with the same schema this only catches a wrapper bug.
5. Claude Code's repair limit reached (`terminal_reason: "structured_output_retry_exhausted"`): it prints `{"error": {"code": "output.invalid", "message": <last errors>}, "retryable": false}`.
6. `structured_output: null` with exit 0 (W02's "missing" case, not reproduced on 2.1.287+ by C04): the wrapper resumes the session once with "you ended without giving your answer in the required form; give it now". If that still gives nothing, it prints `{"error": {"code": "output.missing", ...}, "retryable": false}`. This replaces C04's retryable failure.
7. It writes the try's run facts to the facts file (R8).

### Plugin side: interactive step (`claude-spore report`)

This amends C07 R15 and the watch step of C07's interactive procedure.

1. Every start or resume message gives Claude the exact report command for this visit (it names the invocation, so it cannot report into another visit) and the formats for each outcome, as text with the schemas.
2. Claude ends by running it: `claude-spore report --outcome changes`, with the answer as JSON on stdin. The plugin's permission rules allow that one command.
3. The command checks the answer. If it fits, it writes `<runtime dir>/<RUNSPORE_INVOCATION_ID>.result.json` in watch format and prints "accepted". If not, it prints one line per error and exits 1, and counts the bad answer in `<runtime dir>/<RUNSPORE_INVOCATION_ID>.answers`. On the third bad answer it writes a failure result (`output.invalid`, not retryable) and tells Claude the step is handed to a person.
4. The watch is as C07 R15 and C13 say: it returns the file once the session is quiet. The host checks the result again (R1), so a file Claude wrote by hand cannot get past the schema.

### The reference loop, step by step

1. `review` (background) returns `changes` with no `requests`. Claude Code rejects the answer and tells Claude `$.answer.output: must have required property 'requests'`; Claude answers again with two requests. Spore receives one fitting answer and the run goes to `implement`, whose input maps `nodes.review.output.requests`.
2. In an interactive `implement`, Claude runs the report command with `{"summary": 12}`. The command prints `$.summary: expected type "string", got integer (12)`; Claude reports again with a string, the result file appears, and the watch returns `ok`.
3. If Claude gets it wrong three times, the step's result is `output.invalid`, not retryable. With no `failed` arrow the run waits (exit 4), and `spore status` shows the errors. The person runs `spore resolve <run> <invocation> --retry` (a new try that resumes the same conversation), or `--complete ok --output '{"summary": "done by hand"}'`, which is checked too (R2), or `--fail`.
4. At `approve`, `spore signal <run> approval --outcome rejected` with no data exits 2: `$: expected type "object", got null`. With `--data '{"reason": "tests are too weak"}'` it is accepted, and the run goes back to `implement` with the reason.

## Verified facts

Established by reading the repo, the old map and the Claude Code binary, not by asking:

- `claude -p --json-schema <schema>` exists in Claude Code 2.1.296 (`claude --help`: "JSON Schema for structured output validation"). Unlike `--max-budget-usd`, the help text does not mark it "only works with --print"; whether it enforces anything in an interactive session was not checked, and the design does not rely on it.
- W02 (skills-repo, branch `research/w02-agent-boundary-typing`, Claude Code 2.1.269, from the binary): `--json-schema` injects a `StructuredOutput` tool, validates each call with Ajv, and feeds a failure back into the same conversation as the tool's result. Default 5 attempts, overridable with `MAX_STRUCTURED_OUTPUT_RETRIES`; exhaustion ends with `terminal_reason: "structured_output_retry_exhausted"`. The strict-derivation allowlist is `$schema, type, description, title, properties, required, additionalProperties, items, enum, const, anyOf` over types `object, array, string, integer, number, boolean, null`, depth 32; an object must declare `properties`; `additionalProperties` must be absent or `false`; `anyOf` must be the only keyword; the root must be an object. Keywords outside the allowlist (`maxLength`, `minLength`, `uniqueItems`, `minimum`, proven by forced violations) are still enforced, by Ajv with retry, without the grammar guarantee.
- The 2.1.296 binary still contains `MAX_STRUCTURED_OUTPUT_RETRIES`, `structured_output_retry_exhausted` and the text "Failed to provide valid structured output after" (checked 2026-10-10). Its default was not re-read.
- W02: a model can end with `structured_output: null`, exit 0, `subtype: success`, either never calling the tool or giving up after a rejection, before the attempt budget is spent. C04 did not reproduce it on 2.1.287 or later. W02: enforcement guarantees shape, never truth (`minimum: 100` on a toddler's age gave `730`, in days).
- W02: a Task-tool subagent enforces no schema. C07 L2 removed in-session subagent steps from the profile.
- W06's follow-ups: keep "missing" apart from "invalid"; strict enforcement forces `additionalProperties: false`, so a Claude step cannot pass unknown keys through; `oneOf` is excluded by choice, not by an API limit; the reference validator's error format is `$.owner.name: expected type "string", got number (42)`, all errors collected.
- Kernel 0.1 (spec/kernel.md): the kernel reads only `outcomes`, `effect` and `retry` from an action (§2, M02); W01–W11 exist and C13 adds W12; data is canonical JSON with integers only, within ±(2^53 − 1), depth at most 32 (§1); a routed failure's output is `{"error": ErrorInfo}` (§7.4); a mapping error is terminal `failed` (§4, §7.4), and C07 keeps it terminal.
- spec/README.md lists "JSON Schema validation of inputs and outputs" as out of scope for 0.1, "additive under a later semantics version".
- CLI exit codes: 2 usage or validation error, 4 needs intervention (spec/cli.md).
- C03 R9: `step complete` refuses an undeclared outcome with exit 2 and requires canonical `--output` of at most 64 KiB. C07 R7: an undeclared outcome from a command is `command.undeclared-outcome`, not retryable. C07 R10: `claude-spore check` validates a workflow before a run starts.
- C04: the background wrapper emitted `session`, `result`, `turns`, `durationMs`, `costMicroUsd`, `deniedTools`; it made integral floats integers and failed on other non-integers; it treated a null `structured_output` as a retryable failure; it recommended declaring `integer`, not `number`.
- docs/06: the target subset is "objects, arrays, required properties, enums, bounded lengths, and supported primitive types", local references bundled at compile time, remote references prohibited; producer output is validated at ingestion; agent repair "creates a bounded child or rework activation with a new logical ID"; "commands receive no model-based repair by default".
- C01 (workflow-map carry-over): L14 (every boundary declares a schema), L15 (subset), L21 (repair, budget 2, exhausted becomes `failed`), L22 (no repair for commands), W02, W06 and W14 all name C12.

## Changes outside C12

- **Kernel 0.2 / package format 0.2.** Node field `outputSchemas` on `activity` and `await-signal` nodes; new rule W13. Transitions unchanged.
- **Host 0.2 (spec/host.md §2–3).** The output check for sync and watch results, `output.invalid`, and the subset check in `validate_workflow` (`workflow.invalid-schema`). The host now reads the node's `outputSchemas` in addition to the action.
- **CLI 0.2 (spec/cli.md).** `step complete`, `resolve --complete` and `signal` refuse data that does not fit (exit 2); `build` checks schemas.
- **spec/README.md.** "JSON Schema validation of … outputs" leaves the out-of-scope list for 0.2; inputs stay out (R11).
- **C02 (docs/design/c02-dot-format.md).** `settings.json` node fields gain `outputSchemas`, inline or as a file path the build inlines. The DOT is unchanged.
- **C03 (docs/design/c03-in-session-hand-off.md).** R9's `step complete` checks also cover the output schema. Its open thread "Output schemas … that is C12" is answered.
- **C04 (docs/research/c04-headless-agent-step.md).** The recipe's output is the checked answer only; run facts move to the facts file (R8). A null `structured_output` gets one resume with a nudge, then a non-retryable `output.missing`, instead of a retryable failure.
- **C07 (docs/design/c07-four-kinds.md).** Background procedure step 3 and R15 as in the Spec. "Output schemas are C12" is answered. C07's `review` example gains per-outcome schemas (as above).
- **docs/06.** Two departures from the target design, to note when the spec lands. Agent repair happens inside the try, in the same conversation, not as a new activation, because Claude Code's loop already keeps the rejected answer in context and spore never records it. Commands never get model-based repair, where docs/06 said "by default".
- **CONTEXT.md.** New terms Output schema (Steps and their actors) and Repair (Claude Code integration).
- **Tickets.** C09 (authoring interview): write a schema for every outcome of every Claude step, with the `summary` default (R5), and offer schemas for people's answers at `await-signal` nodes. C10 (packaging): `claude-spore check` refuses a Claude step outcome without a schema; the report command's name; the permission rule that allows it. C14 (delivery): the wrapper and the report command need the node's outcomes and schemas at dispatch, through the same channel as the instructions. C11 (reference loop): declare the schemas, for example a reason on a rejection. Map fog: the facts file feeds progress and evidence display.

## Risks

- **A fitting answer can still be wrong.** A schema checks shape, never truth (W02's toddler aged 730). Checking that the work is right is a separate step in the graph, such as `test`.
- **Claude Code internals are undocumented.** `MAX_STRUCTURED_OUTPUT_RETRIES`, the strict allowlist and the `StructuredOutput` loop were read from the 2.1.269 binary; the setting still exists in 2.1.296. If Claude Code changes them, the background repair budget drifts. The host's own check (R1) still stops a bad answer from reaching the run.
- **Claude steps cannot pass extra keys through.** Claude Code closes every object, so a field the schema does not name never comes out of a Claude step, even where spore would allow it.
- **An interactive Claude may skip the report command.** If it writes the result file itself, the host's check catches a bad answer, but as a stuck step (no repair), not as a repair in the conversation.
- **Strict rules for signals.** Every `await-signal` node that waits for the same signal name must declare the same schemas (W13), which may surprise an author who reuses a name.
- **Bigger packages.** Inline schemas count toward the package and the run's identity; a schema edit is a new package, so runs started before it keep the old one.

## Deferred

- **Input schemas.** A schema for the run's input, checked at `start`, and for a step's mapped input, checked before dispatch (docs/06). Reopens when a workflow takes input from outside that later steps rely on, or when a step's input comes from an unchecked source.

## Open threads

- **`pattern`.** Left out of the subset because regular-expression dialects differ between the Rust validator and Claude Code's Ajv. Can be added with a dialect rule if a workflow needs it.
- **A generic place for run facts.** R8 keeps them in a plugin file; whether spore should store a "report" beside each try (cost, duration, denied tools) is part of the fog item on progress and evidence display.
- **The repair count in a background step.** Claude Code counts answers within one `claude -p` call; the one resume after a missing answer (Spec step 6) starts its count again, so in that rare case a try can see more than three answers.
- **Names.** `outputSchemas`, `claude-spore report`, the `.answers` and `.facts.json` files and the `output.missing` code are provisional (C10).
