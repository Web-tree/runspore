# `mapping.literal-extra-key`: "$literal" with another key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.literal-extra-key` reports:

```text
── "$literal" with another key ── mapping.literal-extra-key/settings.json:13  mapping.literal-extra-key
The "$literal" object in step "build" also has "quiet", and it may have only "$literal".

 13 │       "input": { "flags": { "$literal": ["-j4"], "quiet": true } }
    │                                                  ^^^^^^^

To fix: remove "quiet", or move it into the "$literal" value.
```

The fix changes:

```text
mapping.literal-extra-key/settings.json:
 13 │-      "input": { "flags": { "$literal": ["-j4"], "quiet": true } }
 13 │+      "input": { "flags": { "$literal": ["-j4"] } }
```

The example folder is [`catalogue/examples/mapping.literal-extra-key`](../../crates/runspore-build/catalogue/examples/mapping.literal-extra-key).
