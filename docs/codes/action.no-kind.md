# `action.no-kind`: Missing "kind"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.no-kind` reports:

```text
── Missing "kind" ────────────────── action.no-kind/settings.json:4  action.no-kind
Action "build" does not say what kind of action it is.

  4 │     "build": {
    │     ^^^^^^^

To fix: add "kind": "command" to run a program.
```

The fix changes:

```text
action.no-kind/settings.json:
  6 │-      "effect": "idempotent"
  6 │+      "effect": "idempotent",
  7 │+      "kind": "command"
```

The example folder is [`catalogue/examples/action.no-kind`](../../crates/runspore-build/catalogue/examples/action.no-kind).
