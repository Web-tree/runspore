# `settings.too-large`: Number too large

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.too-large` reports:

```text
── Number too large ────────────────── settings.too-large/settings.json:4  settings.too-large
5000 at "limits" → "maxVisitsPerNode" is too large.

  4 │     "maxVisitsPerNode": 5000
    │                         ^^^^

To fix: use 1000 or less.
```

The fix changes:

```text
settings.too-large/settings.json:
  4 │-    "maxVisitsPerNode": 5000
  4 │+    "maxVisitsPerNode": 1000
```

The example folder is [`catalogue/examples/settings.too-large`](../../crates/runspore-build/catalogue/examples/settings.too-large).
