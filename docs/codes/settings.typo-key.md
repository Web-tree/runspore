# `settings.typo-key`: Unknown key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.typo-key` reports:

```text
── Unknown key ────────────────── settings.typo-key/settings.json:7  settings.typo-key
There is no key "efect" in "actions" → "build".

  6 │       "argv": ["make"],
  7 │       "efect": "idempotent"
    │       ^^^^^^^

To fix: rename it to "effect".
```

The fix changes:

```text
settings.typo-key/settings.json:
  7 │-      "efect": "idempotent"
  7 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/settings.typo-key`](../../crates/runspore-build/catalogue/examples/settings.typo-key).
