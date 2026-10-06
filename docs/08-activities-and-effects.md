# 8 Effects and non deterministic activities

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The abstraction is an Action binding used by a graph node. A binding declares input and output contracts, named outcomes, implementation identity, required capabilities, effect policy, timeouts, retryability, cancellation behavior, and optional reconciliation or compensation. A node supplies graph position, mappings, and activation policy. This distinction preserves the useful idea in epic 18 without mixing topology with implementations. [18]

| Binding kind | Meaning | Portability |
| --- | --- | --- |
| Pure builtin | Literal, projection, comparison, bounded transform | All certified kernels |
| Wasm activity | Component invoked with granted capabilities | Host profile must support its imports |
| Native function | Registered Rust or JS callback | Bound to that worker capability |
| Command | Executable and argv, working directory, limits | OS worker only |
| HTTP or MCP | Typed remote call with authenticated adapter | Network-capable host or remote worker |
| Agent or skill | Agent runner plus pinned instruction assets | Agent-capable worker |
| Human | Durable request and authenticated response | UI or external client |
| Child workflow | A pinned graph package invocation | Supported store messaging profile |

Static text is an immutable asset or a literal value. Displaying it is a UI action; asking a model to follow it is an agent action; waiting for a person is a human action. A skill is instructions plus a runner and granted tools, not executable behavior on its own. Avoid inventing a text activity that implicitly chooses a model or side effect.

## Effect classes

| Policy | Retry rule | Example |
| --- | --- | --- |
| Pure | Recompute within budget | Local deterministic validation |
| Read only | Repeat allowed; first durably accepted result becomes history | Fetch current CI status |
| Idempotent | Same effect key and identical payload on each attempt | API supporting durable idempotency |
| Reconcilable | Query external identity before reissue | Ensure a PR exists for a branch |
| Unsafe to repeat | Ambiguous outcome pauses for reconciliation or a person | Non-idempotent external command |

Non-determinism and danger are separate dimensions. A read-only LLM request can return a different answer and cost money on retry; an apparently deterministic shell command can cause an irreversible side effect. Record accepted model outputs, tool results, external IDs, execution environment, and model/tool configuration. Replaying the graph does not call the model again. Resuming a failed agent attempt may call it again, so its tool effects need their own ledger or must be isolated and reconcilable.

For a tool-using agent, expose effectful tools through the runtime's broker when they can change important external state. Each tool operation gets a persisted identity under the logical agent invocation, preserved across physical attempt retries. Do not infer equivalence from an LLM-generated description or assign a fresh effect key because the agent restarted. If a resumed model cannot reliably identify its previous operation, enter unknown and reconcile. Providers without stable resumable call IDs require a journaled wrapper or a coarse activity that enters unknown on interruption.

Keep unknown as a first-class operational status. A timeout does not prove failure; a successful local commit does not prove external execution. An idempotency key must remain valid for the entire possible retry window. If the downstream service forgets keys after a short retention period, the adapter must stop retrying blindly once that window expires.
