# `dot.undeclared-outcome`: Arrow for an outcome the action does not have

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.undeclared-outcome` reports:

```text
── Arrow for an outcome the action does not have ── dot.undeclared-outcome/workflow.dot:9  dot.undeclared-outcome
Step "build" has an arrow for the outcome "red", which its action "build" does not list.

  9 │   build -> build [on="red"];
    │   ^^^^^^^^^^^^^^

To fix: add "red" to "outcomes" of action "build", or use one it lists: "ok".
```

The fix changes:

```text
dot.undeclared-outcome/settings.json:
  7 │-      "effect": "idempotent"
  7 │+      "effect": "idempotent",
  8 │+      "outcomes": ["ok", "red"]
```

The example folder is [`catalogue/examples/dot.undeclared-outcome`](../../crates/runspore-build/catalogue/examples/dot.undeclared-outcome).
