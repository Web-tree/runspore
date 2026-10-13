# `action.reserved-outcome`: Outcome spore keeps for itself

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.reserved-outcome` reports:

```text
── Outcome spore keeps for itself ── action.reserved-outcome/settings.json:8  action.reserved-outcome
Action "build" lists "failed", an outcome spore keeps for itself.

  8 │       "outcomes": ["ok", "failed"]
    │                          ^^^^^^^^

To fix: remove it; spore gives a step "failed" on its own.
```

The fix changes:

```text
action.reserved-outcome/settings.json:
  8 │-      "outcomes": ["ok", "failed"]
  8 │+      "outcomes": ["ok"]
```

The example folder is [`catalogue/examples/action.reserved-outcome`](../../crates/runspore-build/catalogue/examples/action.reserved-outcome).
