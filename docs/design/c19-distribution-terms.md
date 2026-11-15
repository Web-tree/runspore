# C19 — Names for spore distribution

Design record of the first ticket on the spore distribution map (#240), held on 2026-11-15. It fixes the vocabulary every later ticket on that map uses: what a spore is, what it exports for steps to call, the two manifests and the lock, and where spores come from and are kept. **A spore is the versioned unit of distribution. The reusable WebAssembly piece it exports is a Wasm action, always written in full, because "action" alone keeps meaning the binding in a settings file.** The entries themselves live in `CONTEXT.md` under *Distribution*; this record keeps why each name was picked and what it was picked over.

No existing term changes meaning. Two wordings in other entries are narrowed: the **Plugin** entry now says "the `max-skills` Claude Code marketplace", and the **Release key** entry says "signing spores" for what it called "package signing", since **Package** now names only the workflow document, and what a publisher would sign is a spore version (whether they do is #245's). The MVP scope line in `spec/README.md` is reworded the same way.

## Locked decisions

### L1. The reusable WebAssembly piece is a "Wasm action" (Max)

**Decision.** A pure WebAssembly component that a spore exports for steps to call (input and node parameters in, outcome and output back, no process control) is a **Wasm action**. An action in a workflow's settings file names one. "Action" alone always means that binding; the component is always "Wasm action" in full, in docs, diagnostics and CLI output.

The map's destination, its decisions and five ticket titles already say "WebAssembly action", so the name costs no renaming. GitHub Actions carries the same double meaning (a step `uses:` an action) without trouble.

Rejected:
- *A new word, such as "function" or "operation".* No overlap with Action, but in the component model a component's exports are already functions, so "the function's functions" would be common in the WIT and the docs, and the map would need renaming.
- *Widen **Step runner** to cover it.* One word for every WebAssembly piece a step uses, but a step runner starts and supervises processes through granted host imports (#233), while a Wasm action only computes. Their worlds differ (#244), and one name would hide that.

## Routine choices

### R1. Spore is the unit; `spore` is the CLI

The maintainer chose the name on 2026-10-16 (map decisions). In prose the unit is "spore" and the CLI is always `spore` in code. "Package" is never used for the unit: it is the canonical workflow document (A-series, `CONTEXT.md`), and Cargo's "package" versus "crate" split shows the cost of two words for one thing.

### R2. Author manifest, artifact manifest, lock

`spore.toml` is the **author manifest**; the JSON inside a workflow artifact (`spec/artifact.md` §2) gets its own entry as the **artifact manifest**; "manifest" alone is avoided. `spore.lock` is the **lock**, written by resolution, and neither manifest. Cargo keeps the same three apart (`Cargo.toml`, `Cargo.lock`, the `.crate` contents). `docs/04` said the target adds a lockfile to the artifact manifest; it now points here: the artifact is not a lock.

### R3. Registry and kind registry

**Registry** alone means a spore registry. C18's **kind registry** (the CLI's table of kinds) is always named in full. "Index" is one part of a registry (#245 decides its layout), not a synonym.

### R4. Listing, not "catalog metadata"

What a registry and the marketplace show to help people find a spore is its **listing**. "Catalog metadata" was the ticket's working name; it would sit next to **Catalogue**, which is where every diagnostic's words are written. Resolution and installation never read a listing, as crates.io keeps `description` and `keywords` out of resolution.

### R5. Marketplace

**Marketplace** is the planned website over the registries (map decisions; out of the first release, `docs/17`). It reads registries and is not one. The plugin's Claude Code marketplace is always named in full.

### R6. Spore cache, not "installed store"

Installed content lives in a **spore cache**. "Store" already means the run database (`spec/store.md`, "the store's heartbeat"). The map decided that an unfinished run keeps what it needs even if this folder is cleaned, so it is reusable, refetchable content: what Cargo and Go call a cache. Its location relative to `$SPORE_HOME` and its cleanup rules are #246's.

### R7. Publisher namespace

The `@publisher` part of a spore's name is its **publisher namespace**. It belongs to the registry and is not a GitHub organization (#245). npm's "scope" is avoided so the term does not suggest npm's rules.

## Left to later tickets

- The author manifest's fields, exports and how a workflow refers to one: #242.
- The action kind that names a Wasm action, and its WIT world: #244.
- Name syntax, versions and the index: #245.
- Install order, the cache's place and cleanup: #246.
