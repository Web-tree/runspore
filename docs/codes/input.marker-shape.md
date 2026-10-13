# `input.marker-shape`: Marker written with something else

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate input.marker-shape` reports:

```text
── Marker written with something else ── input.marker-shape/settings.json:13  input.marker-shape
The marker "$outputSchemas" in the input of step "build" is written with something else.

 13 │       "input": { "$outputSchemas": true },
    │                  ^^^^^^^^^^^^^^^^

To fix: write it as {"$outputSchemas": {}}, on its own.
```

The fix changes:

```text
input.marker-shape/settings.json:
 13 │-      "input": { "$outputSchemas": true },
 13 │+      "input": {"$outputSchemas": {}},
```

The example folder is [`catalogue/examples/input.marker-shape`](../../crates/runspore-build/catalogue/examples/input.marker-shape).
