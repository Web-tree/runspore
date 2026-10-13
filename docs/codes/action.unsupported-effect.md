# `action.unsupported-effect`: Effect this spore cannot run

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.unsupported-effect` reports:

```text
── Effect this spore cannot run ── action.unsupported-effect/settings.json:7  action.unsupported-effect
Action "build" has effect "reconcilable", which this spore cannot run yet.

  7 │       "effect": "reconcilable"
    │                 ^^^^^^^^^^^^^^

To fix: use one of "pure" | "read-only" | "idempotent" | "unsafe".
```

The fix changes:

```text
action.unsupported-effect/settings.json:
  7 │-      "effect": "reconcilable"
  7 │+      "effect": "pure"
```

The example folder is [`catalogue/examples/action.unsupported-effect`](../../crates/runspore-build/catalogue/examples/action.unsupported-effect).
