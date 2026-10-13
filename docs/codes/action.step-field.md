# `action.step-field`: Step key on an action

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.step-field` reports:

```text
── Step key on an action ── action.step-field/settings.json:8  action.step-field
"input" is a key of a step, not of action "build".

  7 │       "effect": "idempotent",
  8 │       "input": { "target": "release" }
    │       ^^^^^^^

To fix: move it to the steps that run action "build", under "nodes".
```

The fix changes:

```text
action.step-field/settings.json:
  7 │-      "effect": "idempotent",
  8 │-      "input": { "target": "release" }
  9 │-    }
 10 │-  },
 11 │-  "nodes": {
 12 │-    "build": { "action": "build" }
  7 │+      "effect": "idempotent"
  8 │+    }
  9 │+  },
 10 │+  "nodes": {
 11 │+    "build": { "action": "build", "input": {"target": "release"} }
```

The example folder is [`catalogue/examples/action.step-field`](../../crates/runspore-build/catalogue/examples/action.step-field).
