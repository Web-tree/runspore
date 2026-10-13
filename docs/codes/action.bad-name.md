# `action.bad-name`: Action name with other characters

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.bad-name` reports:

```text
── Action name with other characters ── action.bad-name/settings.json:4  action.bad-name
Action "make all" has a name spore cannot use; an action name has letters, digits, "_" and "-", at most 64.

  4 │     "make all": {
    │     ^^^^^^^^^^

To fix: rename it, and every "action" that names it.
```

The fix changes:

```text
action.bad-name/settings.json:
  4 │-    "make all": {
  5 │-      "kind": "command",
  6 │-      "argv": ["make"],
  7 │-      "effect": "idempotent"
  8 │-    }
  9 │-  },
 10 │-  "nodes": {
 11 │-    "build": { "action": "make all" }
  4 │+    "make-all": {
  5 │+      "kind": "command",
  6 │+      "argv": ["make"],
  7 │+      "effect": "idempotent"
  8 │+    }
  9 │+  },
 10 │+  "nodes": {
 11 │+    "build": { "action": "make-all" }
```

The example folder is [`catalogue/examples/action.bad-name`](../../crates/runspore-build/catalogue/examples/action.bad-name).
