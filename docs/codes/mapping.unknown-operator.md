# `mapping.unknown-operator`: Unknown "$" key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.unknown-operator` reports:

```text
── Unknown "$" key ────────────────── mapping.unknown-operator/settings.json:13  mapping.unknown-operator
"$gte" in step "build" is not something spore can work out; a key starting with "$" is "$get" or "$literal".

 13 │       "input": { "repo": { "$gte": ["input", "repo"] } }
    │                            ^^^^^^

To fix: use "$get" or "$literal", or put the value inside "$literal" to keep the key as it is.
```

The fix changes:

```text
mapping.unknown-operator/settings.json:
 13 │-      "input": { "repo": { "$gte": ["input", "repo"] } }
 13 │+      "input": { "repo": { "$get": ["input", "repo"] } }
```

The example folder is [`catalogue/examples/mapping.unknown-operator`](../../crates/runspore-build/catalogue/examples/mapping.unknown-operator).
