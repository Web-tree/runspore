# `mapping.missing-path`: "$get" path in "argv" that the step does not give

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate mapping.missing-path` reports:

```text
── "$get" path in "argv" that the step does not give ── mapping.missing-path/settings.json:6  mapping.missing-path
The "argv" of action "build" reads ["input", "target"], which step "build" does not give, so the step fails each time it starts.

  6 │       "argv": ["make", { "$get": ["input", "target"] }],
    │                                            ^^^^^^^^

To fix: map it in the "input" of step "build", or give the "$get" a "default".
Why: a "$get" without a "default" that finds nothing stops the run.
```

The fix changes:

```text
mapping.missing-path/settings.json:
 13 │-      "input": { "repo": { "$get": ["input", "repo"] } }
 13 │+      "input": { "repo": { "$get": ["input", "repo"] }, "target": {"$get": ["input", "target"]} }
```

The example folder is [`catalogue/examples/mapping.missing-path`](../../crates/runspore-build/catalogue/examples/mapping.missing-path).
