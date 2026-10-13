# `node.missing-field`: Step without what its kind needs

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.missing-field` reports:

```text
── Step without what its kind needs ── node.missing-field/workflow.dot:6  node.missing-field
Step "approve" is of kind "await-signal", so settings.json must give it "signal".

  6 │   approve [kind="await-signal"];
    │   ^^^^^^^

To fix: add "signal" to step "approve" under "nodes" in settings.json.
```

The fix changes:

```text
node.missing-field/settings.json:
 11 │-    "build": { "action": "build" }
 11 │+    "build": { "action": "build" },
 12 │+    "approve": {"signal": "approve"}
```

The example folder is [`catalogue/examples/node.missing-field`](../../crates/runspore-build/catalogue/examples/node.missing-field).
