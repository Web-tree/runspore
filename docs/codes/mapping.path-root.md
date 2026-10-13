# `mapping.path-root`: "$get" path with an unknown start

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.path-root` reports:

```text
── "$get" path with an unknown start ── mapping.path-root/settings.json:13  mapping.path-root
The "$get" path of step "build" starts with "inputs", and a path starts with one of "input" | "nodes" | "run" | "from".

 13 │       "input": { "repo": { "$get": ["inputs", "repo"] } }
    │                                     ^^^^^^^^

To fix: start it with one of "input" | "nodes" | "run" | "from".
```

The fix changes:

```text
mapping.path-root/settings.json:
 13 │-      "input": { "repo": { "$get": ["inputs", "repo"] } }
 13 │+      "input": { "repo": { "$get": ["input", "repo"] } }
```

The example folder is [`catalogue/examples/mapping.path-root`](../../crates/runspore-build/catalogue/examples/mapping.path-root).
