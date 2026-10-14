# `mapping.operator-extra-key`: Operator with another key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.operator-extra-key` reports:

```text
── Operator with another key ── mapping.operator-extra-key/settings.json:13  mapping.operator-extra-key
The "$join" object in step "build" also has "trim", and it may have only "$join".

 13 │       "params": { "targets": { "$join": [{ "$get": ["input", "targets"] }, " "], "trim": true } }
    │                                                                                  ^^^^^^

To fix: remove "trim", or put the object inside "$literal".
```

The fix changes:

```text
mapping.operator-extra-key/settings.json:
 13 │-      "params": { "targets": { "$join": [{ "$get": ["input", "targets"] }, " "], "trim": true } }
 13 │+      "params": { "targets": { "$join": [{ "$get": ["input", "targets"] }, " "] } }
```

The example folder is [`catalogue/examples/mapping.operator-extra-key`](../../crates/runspore-build/catalogue/examples/mapping.operator-extra-key).
