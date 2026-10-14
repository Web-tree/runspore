# `mapping.path-not-list`: "$get" path that is not a list

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.path-not-list` reports:

```text
── "$get" path that is not a list ── mapping.path-not-list/settings.json:11  mapping.path-not-list
The "$get" of step "build" is "input.repo", and it must be a list of steps such as ["input", "repo"].

 11 │     "build": { "action": "build", "input": { "repo": { "$get": "input.repo" } } }
    │                                                                ^^^^^^^^^^^^

To fix: write it as a list starting with one of "input" | "nodes" | "run" | "from".
```

The fix changes:

```text
mapping.path-not-list/settings.json:
 11 │-    "build": { "action": "build", "input": { "repo": { "$get": "input.repo" } } }
 11 │+    "build": {
 12 │+      "action": "build",
 13 │+      "input": { "repo": { "$get": ["input", "repo"] } }
 14 │+    }
```

The example folder is [`catalogue/examples/mapping.path-not-list`](../../crates/runspore-build/catalogue/examples/mapping.path-not-list).
