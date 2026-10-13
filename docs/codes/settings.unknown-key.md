# `settings.unknown-key`: Unknown key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.unknown-key` reports:

```text
── Unknown key ────────────────── settings.unknown-key/settings.json:8  settings.unknown-key
There is no key "priority" in "actions" → "build".

  7 │       "effect": "idempotent",
  8 │       "priority": 3
    │       ^^^^^^^^^^

To fix: remove it; the keys allowed here are "argv" | "assets" | "async" | "cwd" | "effect" | "env" | "exitOutcomes" | "function" | "kind" | "outcomes" | "output" | "retry" | "timeoutMs".
```

The fix changes:

```text
settings.unknown-key/settings.json:
  7 │-      "effect": "idempotent",
  8 │-      "priority": 3
  7 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/settings.unknown-key`](../../crates/runspore-build/catalogue/examples/settings.unknown-key).
