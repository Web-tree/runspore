# `command.exit-outcome`: Exit code for an outcome the action does not list

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.exit-outcome` reports:

```text
── Exit code for an outcome the action does not list ── command.exit-outcome/settings.json:8  command.exit-outcome
Exit code "1" of action "build" gives the outcome "red", which the action does not list in "outcomes".

  8 │       "exitOutcomes": { "1": "red" }
    │                              ^^^^^

To fix: add "red" to "outcomes", or give the exit code an outcome the action lists.
```

The fix changes:

```text
command.exit-outcome/settings.json:
  8 │-      "exitOutcomes": { "1": "red" }
  8 │+      "exitOutcomes": { "1": "red" },
  9 │+      "outcomes": ["ok", "red"]
```

The example folder is [`catalogue/examples/command.exit-outcome`](../../crates/runspore-build/catalogue/examples/command.exit-outcome).
