# `mapping.get-extra-key`: "$get" with another key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.get-extra-key` reports:

```text
── "$get" with another key ── mapping.get-extra-key/settings.json:13  mapping.get-extra-key
The "$get" object in step "build" also has "fallback", and it may have only "$get" and "default".

 13 │       "input": { "repo": { "$get": ["input", "repo"], "fallback": "shop" } }
    │                                                       ^^^^^^^^^^

To fix: remove "fallback", or put the object inside "$literal".
```

The fix changes:

```text
mapping.get-extra-key/settings.json:
 13 │-      "input": { "repo": { "$get": ["input", "repo"], "fallback": "shop" } }
 13 │+      "input": { "repo": { "$get": ["input", "repo"] } }
```

The example folder is [`catalogue/examples/mapping.get-extra-key`](../../crates/runspore-build/catalogue/examples/mapping.get-extra-key).
