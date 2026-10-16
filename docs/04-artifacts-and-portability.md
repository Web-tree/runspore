# 4 Artifact and portability model

Design revision 0.1 — 2026-10-05. Proposed architecture; implementation and production qualification are pending. Numbered research references resolve in [Sources](sources.md).

In 0.2 the package is one canonical JSON document, the graph and its action bindings, and the workflow artifact carries it: a WebAssembly component whose manifest lists the package, the source files, schemas, instruction files and assets by path and digest ([spec/artifact.md](../spec/artifact.md)). The target adds a lockfile, source mapping and an exact kernel and ABI to that manifest; the composed artifact already nests the kernel component, and jco output for JavaScript hosts is generated from it. Hash a canonical manifest listing normalized relative file paths and content digests, excluding its own root digest and signatures. Derived JavaScript output includes the source component digest and pinned jco version, options, and toolchain provenance. A content digest identifies a package; a signature establishes a publisher identity under a host trust policy. These are different checks.

The manifest pins graph format, kernel semantics, WIT ABI, canonical encoding, schema dialect, expression dialect, target profile, and resource limits. The run additionally pins the resolved binding set, retry policies, activity implementation digests, and initial permission ceiling. In 0.2 only files built into the artifact are pinned: a probe or watch named by a bare program name, and an integration's helper, are looked up on the machine at each use. Host revocations remain effective immediately; package pinning never freezes security vulnerabilities or grants forever.

Jco transpiles components to core Wasm plus JavaScript glue. Build those files in the trusted release pipeline with explicit instantiation and imports. Do not accept arbitrary publisher JavaScript just because the associated Wasm is signed. Standard JavaScript WebAssembly APIs do not directly instantiate a Component Model binary. [4, 5]

Cloudflare's standard Worker profile restricts dynamic Wasm compilation. Deploy a fixed kernel and accept new validated graph IR as data. A graph containing new custom Wasm code requires a new supported deployment or a remote activity worker. This is a useful constraint: changing workflow data need not redeploy the kernel. [6]

## Supported profiles and promotion order

| Profile | Reducer | Store and wakeup | Intended support |
| --- | --- | --- | --- |
| Rust desktop or service | Wasmtime component | SQLite, supervised process | First GA |
| Node service or CLI | Jco ESM and core Wasm | SQLite, supervised process | First GA |
| Bun service or CLI | Same core and Bun adapter | Bun SQLite, supervised process | First GA only after independent tests |
| Shared server workers | Rust or Node or Bun | PostgreSQL, persistent polling | Second gate |
| Browser | Dedicated worker and core Wasm | IndexedDB, resume while open | Local resumability profile |
| Cloudflare | Deployed kernel and IR | Durable Object, alarms and reconciliation | Separate certified profile |
| Other edge platforms | Provider adapter | Certified transactions and wakeups | Experimental until certified |

The portable ABI uses explicit state bytes and logical IDs, never live WIT resource handles as persisted state. WIT u64 becomes BigInt in JS bindings; JSON APIs encode sequences, timestamps, and counters as decimal strings. Do not accidentally round them through JavaScript Number.
