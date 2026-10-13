# `settings.not-object`: Settings that are not an object

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.not-object` reports:

```text
── Settings that are not an object ── settings.not-object/settings.json:1  settings.not-object
settings.json holds ["build"], and it must hold one JSON object.

  1 │ ["build"]
    │ ^^^^^^^^^

To fix: write the settings as one object, with "format", "actions" and "nodes".
```

The fix changes:

```text
settings.not-object/settings.json:
  1 │-["build"]
  1 │+{
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
 13 │+}
```

The example folder is [`catalogue/examples/settings.not-object`](../../crates/runspore-build/catalogue/examples/settings.not-object).
