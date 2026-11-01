# `component.grant-unsupported`: Grant spore does not offer

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate component.grant-unsupported` reports:

```text
── Grant spore does not offer ── component.grant-unsupported/settings.json:8  component.grant-unsupported
Action "review" grants its runner "network", a kind of grant spore does not offer.

  8 │       "grants": { "programs": ["herdr"], "network": ["api.example.com"] }
    │                                          ^^^^^^^^^

To fix: remove it; the kinds of grant spore offers are "folders" | "programs" | "sockets" | "writeFolders".
Why: spore enforces every grant itself, so it refuses one it cannot enforce rather than ignore it.
```

The fix changes:

```text
component.grant-unsupported/settings.json:
  8 │-      "grants": { "programs": ["herdr"], "network": ["api.example.com"] }
  8 │+      "grants": { "programs": ["herdr"] }
```

The example folder is [`catalogue/examples/component.grant-unsupported`](../../crates/runspore-build/catalogue/examples/component.grant-unsupported).
