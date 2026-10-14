# `mapping.operator-shape`: Operator with the wrong operands

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.operator-shape` reports:

```text
── Operator with the wrong operands ── mapping.operator-shape/settings.json:13  mapping.operator-shape
"$if" in step "build" is given [{"$get": ["input", "fresh"]}, "clean"], and it takes a list shaped [cond, then, else].

 13 │       "params": { "mode": { "$if": [{ "$get": ["input", "fresh"] }, "clean"] } }
    │                                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To fix: give "$if" a list shaped [cond, then, else], each item a value or a mapping.
```

The fix changes:

```text
mapping.operator-shape/settings.json:
 13 │-      "params": { "mode": { "$if": [{ "$get": ["input", "fresh"] }, "clean"] } }
 13 │+      "params": { "mode": { "$if": [{ "$get": ["input", "fresh"] }, "clean", "incremental"] } }
```

The example folder is [`catalogue/examples/mapping.operator-shape`](../../crates/runspore-build/catalogue/examples/mapping.operator-shape).
