# `action.no-outcomes`: Empty "outcomes"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.no-outcomes` reports:

```text
── Empty "outcomes" ────────────────── action.no-outcomes/settings.json:8  action.no-outcomes
Action "build" lists no outcomes in "outcomes".

  7 │       "effect": "idempotent",
  8 │       "outcomes": []
    │       ^^^^^^^^^^

To fix: remove the empty list, so the action ends with "ok", or list its outcomes.
```

The fix changes:

```text
action.no-outcomes/settings.json:
  7 │-      "effect": "idempotent",
  8 │-      "outcomes": []
  7 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/action.no-outcomes`](../../crates/runspore-build/catalogue/examples/action.no-outcomes).
