# `dot.no-steps`: No steps

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-steps` reports:

```text
── No steps ────────────────── dot.no-steps/workflow.dot:1  dot.no-steps
workflow.dot has no steps.

  1 │ digraph "release" {

To fix: add the steps of the workflow, and the arrows between them.
```

The fix changes:

```text
dot.no-steps/settings.json:
  2 │-  "format": "runspore.workflow/0.2"
  2 │+  "format": "runspore.workflow/0.2",
  3 │+  "actions": {
  4 │+    "build": {
  5 │+      "kind": "command",
  6 │+      "argv": ["make"],
  7 │+      "effect": "idempotent"
  8 │+    }
  9 │+  },
 10 │+  "nodes": {
 11 │+    "build": { "action": "build" }
 12 │+  }
dot.no-steps/workflow.dot:
  3 │+  start   = "build";
  4 │+
  5 │+  build [label="Build the release"];
  6 │+  done  [kind="complete"];
  7 │+
  8 │+  build -> done;
```

The example folder is [`catalogue/examples/dot.no-steps`](../../crates/runspore-build/catalogue/examples/dot.no-steps).
