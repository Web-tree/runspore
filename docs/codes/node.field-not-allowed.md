# `node.field-not-allowed`: Key that the step's kind does not take

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.field-not-allowed` reports:

```text
── Key that the step's kind does not take ── node.field-not-allowed/settings.json:11  node.field-not-allowed
Step "build" is of kind "activity", and "signal" does not go on that kind.

 11 │     "build": { "action": "build", "signal": "go" }
    │                                   ^^^^^^^^

To fix: remove it, or change the step's kind in workflow.dot.
```

The fix changes:

```text
node.field-not-allowed/settings.json:
 11 │-    "build": { "action": "build", "signal": "go" }
 11 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/node.field-not-allowed`](../../crates/runspore-build/catalogue/examples/node.field-not-allowed).
