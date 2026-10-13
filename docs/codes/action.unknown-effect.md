# `action.unknown-effect`: Unknown effect

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.unknown-effect` reports:

```text
── Unknown effect ────────────────── action.unknown-effect/settings.json:7  action.unknown-effect
Action "build" has effect "idempotant", which spore does not know.

  7 │       "effect": "idempotant"
    │                 ^^^^^^^^^^^^

To fix: use one of "pure" | "read-only" | "idempotent" | "unsafe".
```

The fix changes:

```text
action.unknown-effect/settings.json:
  7 │-      "effect": "idempotant"
  7 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/action.unknown-effect`](../../crates/runspore-build/catalogue/examples/action.unknown-effect).
