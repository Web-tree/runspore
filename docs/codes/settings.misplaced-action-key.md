# `settings.misplaced-action-key`: Key in the wrong place

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.misplaced-action-key` reports:

```text
── "retry" is in the wrong place ── settings.misplaced-action-key/settings.json:12  settings.misplaced-action-key
Every key under "nodes" must be a step in workflow.dot, and there is no step "retry".

 11 │     "build": { "action": "build" },
 12 │     "retry": { "maxAttempts": 3 }
    │     ^^^^^^^

To fix: retry is a key of an action. Move this line into the action it describes, for example "actions" → "build".
```

The fix changes:

```text
settings.misplaced-action-key/settings.json:
  7 │-      "effect": "idempotent"
  8 │-    }
  9 │-  },
 10 │-  "nodes": {
 11 │-    "build": { "action": "build" },
 12 │-    "retry": { "maxAttempts": 3 }
  7 │+      "effect": "idempotent",
  8 │+      "retry": {"maxAttempts": 3}
  9 │+    }
 10 │+  },
 11 │+  "nodes": {
 12 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/settings.misplaced-action-key`](../../crates/runspore-build/catalogue/examples/settings.misplaced-action-key).
