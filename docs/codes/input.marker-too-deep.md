# `input.marker-too-deep`: Marker copy nested too deep

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate input.marker-too-deep` reports:

```text
── Marker copy nested too deep ── input.marker-too-deep/settings.json:13  input.marker-too-deep
The copy that "$outputSchemas" makes in the "input" of step "build" reaches level 33, past the 32 levels a package may hold.

 13 │       "input": { "nested": { "$outputSchemas": {} } },
    │                              ^^^^^^^^^^^^^^^^

To fix: move the marker closer to the top, or make the schemas shallower.
```

The fix changes:

```text
input.marker-too-deep/settings.json:
 13 │-      "input": { "nested": { "$outputSchemas": {} } },
 13 │+      "input": { "$outputSchemas": {} },
```

The example folder is [`catalogue/examples/input.marker-too-deep`](../../crates/runspore-build/catalogue/examples/input.marker-too-deep).
