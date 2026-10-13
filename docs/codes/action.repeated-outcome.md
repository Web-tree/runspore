# `action.repeated-outcome`: Outcome listed twice

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.repeated-outcome` reports:

```text
── Outcome listed twice ── action.repeated-outcome/settings.json:8  action.repeated-outcome
Action "build" lists the outcome "ok" twice.

  8 │       "outcomes": ["ok", "ok"]
    │                          ^^^^

To fix: remove the second one.
```

The fix changes:

```text
action.repeated-outcome/settings.json:
  8 │-      "outcomes": ["ok", "ok"]
  8 │+      "outcomes": ["ok"]
```

The example folder is [`catalogue/examples/action.repeated-outcome`](../../crates/runspore-build/catalogue/examples/action.repeated-outcome).
