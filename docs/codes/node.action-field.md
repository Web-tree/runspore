# `node.action-field`: Action key on a step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.action-field` reports:

```text
── Action key on a step ── node.action-field/settings.json:11  node.action-field
"timeoutMs" is a key of an action, not of step "build".

 11 │     "build": { "action": "build", "timeoutMs": "600000" }
    │                                   ^^^^^^^^^^^

To fix: move it into the action that step "build" runs, under "actions".
```

The fix changes:

```text
node.action-field/settings.json:
  7 │-      "effect": "idempotent"
  8 │-    }
  9 │-  },
 10 │-  "nodes": {
 11 │-    "build": { "action": "build", "timeoutMs": "600000" }
  7 │+      "effect": "idempotent",
  8 │+      "timeoutMs": "600000"
  9 │+    }
 10 │+  },
 11 │+  "nodes": {
 12 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/node.action-field`](../../crates/runspore-build/catalogue/examples/node.action-field).
