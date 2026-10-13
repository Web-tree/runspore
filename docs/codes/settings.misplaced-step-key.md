# `settings.misplaced-step-key`: Key in the wrong place

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.misplaced-step-key` reports:

```text
── "input" is in the wrong place ── settings.misplaced-step-key/settings.json:12  settings.misplaced-step-key
Every key under "nodes" must be a step in workflow.dot, and there is no step "input".

 11 │     "build": { "action": "build" },
 12 │     "input": { "target": "release" }
    │     ^^^^^^^

To fix: input is a key of a step. Move this line into the step it describes, for example "nodes" → "build".
```

The fix changes:

```text
settings.misplaced-step-key/settings.json:
 11 │-    "build": { "action": "build" },
 12 │-    "input": { "target": "release" }
 11 │+    "build": { "action": "build", "input": {"target": "release"} }
```

The example folder is [`catalogue/examples/settings.misplaced-step-key`](../../crates/runspore-build/catalogue/examples/settings.misplaced-step-key).
