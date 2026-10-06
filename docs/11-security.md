# 11 Security model

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

The trust boundary includes the host process, kernel release, store adapter, policy engine, and credential broker. An untrusted graph is data validated by the kernel. Third-party activity code is more privileged and requires a declared execution profile. Signing proves origin, not safety. An administrator who owns the host can bypass in-process policy; hostile multi-tenant service isolation is a separate deployment design.

Effective permissions are the intersection of deployment policy, tenant policy, run grant, binding requirements, and any valid human approval. None of the graph, prompt, plugin manifest, or model output can widen that intersection. Freeze a run's maximum grant but allow current policy to revoke it. Resolve secret references at dispatch, return handles where possible, and keep actual secret values out of state, prompts, logs, errors, and package files.

## Threats and controls

| Threat | Required control |
| --- | --- |
| Malicious graph or huge payload | Strict parser, size and depth caps, bounded kernel, quotas |
| Untrusted Wasm activity | Import allowlist, memory and CPU limits, capability-scoped host calls |
| Native script escape | Isolated process or container, unprivileged identity, resource limits, restricted filesystem and network |
| Shell injection | Executable plus argv; shell mode requires explicit grant; no template concatenation |
| SSRF | Network allowlist, DNS and resolved-address validation, redirect checks, private and metadata network policy |
| Path or symlink escape | Rooted filesystem handles and validated final paths; isolated worktree |
| Prompt injection | Treat retrieved content as data; broker validates each effect independently of model instructions |
| Cross-tenant access | Tenant in every key and query; scoped credentials and authorization tests |
| Stolen approval | Bind actor, action, input digest, artifact, expiry, nonce, and policy version |
| Supply-chain change | Immutable dependencies, checksums, signatures, SBOM, trusted generated bindings |
| Stale worker | Conditional fenced completion plus downstream idempotency or reconciliation |
| Telemetry exfiltration | Attribute allowlist, redaction, quotas, host-owned exporters |

Wasmtime metering does not meter a blocking host call; adapters need I/O deadlines, cancellation, output caps, and host resource limits. Node's WASI API is not a security sandbox, and Worker thread resource limits do not cover all external memory. Use subprocess or stronger isolation for untrusted native execution. Cloudflare and browser profiles should accept the bounded project kernel plus approved activities, not arbitrary unmetered custom reducers. [9, 15, 16]

## Approvals and authorization

An approval record includes authenticated actor, tenant, run and activation, package digest, action binding digest, exact input digest, scope, expiry, nonce, and target external revision where relevant. The broker checks it again immediately before execution and transactionally binds consumption to one logical operation and receipt. Recovery reconciles that same authorized operation; it cannot reuse approval for a different operation. New dispatches recheck revocation and expiry; expired permission permits evidence reconciliation but no new mutation unless policy authorizes it. Changed code, target, arguments, or permission policy invalidates the old approval. A plain graph signal saying approved is not sufficient.

For local tools, use an authenticated Unix socket or equivalent local transport, restrictive file permissions, and CSRF and origin protection if a web UI is added. For remote workers, use authenticated transport, short-lived scoped credentials, per-tenant dispatch identities, and response validation. Generic MCP tool exposure must not allow a model to forge administrative completions or human decisions.

A workflow-level cancel does not undo a payment, merge, or deployment. Present effect status and reconciliation evidence clearly. Limit ordinary agent workers to a disposable worktree and task credentials; keep production and merge credentials exclusively in the broker. Same-user native code and installed mods are trusted in the local profile; they may access user-owned files or sockets. For restricted agents, isolate broker credentials and administrative sockets from their sandbox. Per-invocation data-plane tokens cannot call approval or admin endpoints. Human approval uses a separately authenticated control channel. Never deserialize untrusted Wasmtime native cache files as portable Wasm; build such caches locally from verified modules.
