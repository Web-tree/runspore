# 6 Workflow Graph semantics

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The authoring language is a workflow folder: `workflow.dot` in the named, versioned runspore DOT dialect holds the steps and arrows, and `settings.json` holds the actions, mappings and limits ([spec/dot.md](../spec/dot.md)). The build turns the folder into the canonical JSON package and seals it in a workflow artifact; JSON is the kernel's internal form and a test fixture, not something `spore` reads from a user. The dialect never invents semantics for arbitrary DOT attributes: an attribute outside its vocabulary and render allowlist is an error. YAML, if added, is another frontend to the same package that rejects aliases, custom tags, duplicate keys, and ambiguous scalar coercion. Rust and TypeScript builder APIs emit the same package.

V1 supports sequence, choose by declared outcome, activity, await-signal, timer, bounded loop, fork-all, and complete or fail. Pure projection, literal values, and type validation are kernel operations. Add child workflows and compensation before the general production milestone; race and quorum joins can wait until their cancellation and uncertain-effect tests exist.

| Construct | Exact rule |
| --- | --- |
| Activity | One logical invocation per activation; accepted result chooses one named outcome |
| Sequence | Child starts only after its predecessor's accepted outcome |
| Choice | First explicitly ordered matching route; overlapping unordered conditions are rejected |
| Loop | Explicit body and exit; immutable max iterations or overall run budget |
| Fork all | Create bounded child activations; join after all succeed or follow explicit failure policy |
| Await signal | Consume one matching buffered signal by durable acceptance order |
| Timer | Emit stored absolute deadline; wait for a normalized TimerFired event |
| Child workflow | Independent run with deterministic child ID and idempotent parent message |
| Completion | Terminal state is immutable except separate administrative metadata |

V1 choices should favor outcome dispatch and simple typed comparisons. Do not embed JavaScript eval, arbitrary jq, or user functions in the host. A future CEL frontend may compile a restricted expression AST into the same kernel, after its operator semantics and metering are fixed. Starting with outcome routing and projections avoids blocking the engine on cross-language expression compatibility.

## Activations and identity

A node definition is not a node execution. Each visit gets an activation ID derived from run ID, structured scope, branch index, iteration ordinal, and node ID. A logical invocation ID derives from activation and command ordinal. Physical attempt IDs add an attempt number and lease epoch. Command IDs are deterministic over run, activation, and command ordinal. This prevents two loop visits from being incorrectly deduplicated while keeping retries of one external operation on the same idempotency key.

All internal microsteps execute in stable graph order until blocked, terminal, or budget-limited. Ready branches do not race on shared workflow variables. They write branch-local results. A join constructs an ordered result object or array using declared mappings; key collisions are compile errors unless an explicit deterministic merge exists. Never use last network response wins as an implicit merge policy.

## Retry and cancellation

Separate delivery retries from business rework. A delivery retry repeats the same invocation, input, binding, and effect key after a transport or transient failure. A visible graph loop, such as implement then test then implement, creates a new activation and can change input. Retry backoff, cap, retryable error categories, and total deadline are pinned. Jitter is derived from invocation ID and a versioned algorithm or recorded once, not fresh ambient randomness during replay.

Cancellation in 0.2 is deliberately smaller: `spore cancel` closes the run in the store, like Temporal's terminate. The run ends `cancelled` with its history kept, a step in progress is stopped (an async action's `stop` runs once), a late result is kept as evidence, and nothing is compensated ([spec/cli.md](../spec/cli.md), store `cancel_run`). The rest of this paragraph is the target. Cancellation is a durable request. Its first receipt means accepted, not yet effective. Applying it atomically closes ordinary dispatch eligibility and advances a dispatch generation; claims and the broker verify that generation before starting work. Return a separate effective receipt after this commit. Cancel outstanding operations best-effort and continue reconciliation or compensation. An HTTP abort or killed process is not proof that a remote effect did not happen. Late results are retained as audit evidence and may trigger reconciliation; they cannot silently revive a terminal branch. Track terminal graph result separately from effects-settled status so pending reconciliation remains visible.

Compensation is a separately logged operation, not database rollback. Register compensations when the corresponding effect is accepted, run in reverse causal order within a scope, and retain parallel causal dependencies. Failed compensation leaves a visible needs-intervention state. Child cancellation policy is explicit: cancel, detach, or wait. Do not make cross-run database transactions a prerequisite for child workflows.

## Types and validation

Use a documented JSON Schema subset with objects, arrays, required properties, enums, bounded lengths, and supported primitive types. Local references are resolved and inlined by the build; remote references are prohibited. The 0.2 subset is [spec/schema.md](../spec/schema.md). Avoid pretending that arbitrary JSON Schema implication is decidable by a small lint function. The compiler performs sound checks on its subset and rejects an unprovable mapping or requires an explicit validating adapter.

Validate untrusted producer output at ingestion: in 0.2 every result, signal and resolution is checked against its node's output schema the moment it arrives, and a run's input against the workflow's input schema when it starts. Validating the mapped activity input before dispatch is deferred. Compile-time width compatibility is useful but does not replace runtime boundary checks across external workers, remote messages, or changed artifacts. Schema-correct LLM output can still be false; evidence validation is a separate node. Repairing an agent's answer happens inside one try, in the same conversation, before Runspore sees the answer; Runspore records only the final answer, and one that still does not fit is `output.invalid`, which makes the step stuck or takes its `failed` route. Commands never get repair.
