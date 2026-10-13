# `settings.too-small`: Number too small

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.too-small` reports:

```text
── Number too small ────────────────── settings.too-small/settings.json:8  settings.too-small
0 at "actions" → "build" → "retry" → "maxAttempts" is too small.

  8 │       "retry": { "maxAttempts": 0 }
    │                                 ^

To fix: use 1 or more.
```

The fix changes:

```text
settings.too-small/settings.json:
  8 │-      "retry": { "maxAttempts": 0 }
  8 │+      "retry": { "maxAttempts": 1 }
```

The example folder is [`catalogue/examples/settings.too-small`](../../crates/runspore-build/catalogue/examples/settings.too-small).
