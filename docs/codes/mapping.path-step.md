# `mapping.path-step`: "$get" path with a wrong step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.path-step` reports:

```text
── "$get" path with a wrong step ── mapping.path-step/settings.json:13  mapping.path-step
-1 in a "$get" path of step "build" is not a step of a path; a step is a key or a whole number of zero or more.

 13 │       "input": { "first": { "$get": ["input", "repos", -1] } }
    │                                                        ^^

To fix: use a key or a whole number here.
```

The fix changes:

```text
mapping.path-step/settings.json:
 13 │-      "input": { "first": { "$get": ["input", "repos", -1] } }
 13 │+      "input": { "first": { "$get": ["input", "repos", 0] } }
```

The example folder is [`catalogue/examples/mapping.path-step`](../../crates/runspore-build/catalogue/examples/mapping.path-step).
