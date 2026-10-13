# `input.marker-no-schemas`: Marker on a step with no schemas

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate input.marker-no-schemas` reports:

```text
── Marker on a step with no schemas ── input.marker-no-schemas/settings.json:11  input.marker-no-schemas
The input of step "build" copies its output schemas with "$outputSchemas", but the step has none.

 11 │     "build": { "action": "build", "input": { "schemas": { "$outputSchemas": {} } } }
    │                                                           ^^^^^^^^^^^^^^^^

To fix: add "outputSchemas" to step "build", or remove the marker.
```

The fix changes:

```text
input.marker-no-schemas/settings.json:
 11 │-    "build": { "action": "build", "input": { "schemas": { "$outputSchemas": {} } } }
 11 │+    "build": {
 12 │+      "action": "build",
 13 │+      "input": { "schemas": { "$outputSchemas": {} } },
 14 │+      "outputSchemas": {
 15 │+        "ok": {
 16 │+          "type": "object",
 17 │+          "properties": { "version": { "type": "string" } }
 18 │+        }
 19 │+      }
 20 │+    }
```

The example folder is [`catalogue/examples/input.marker-no-schemas`](../../crates/runspore-build/catalogue/examples/input.marker-no-schemas).
