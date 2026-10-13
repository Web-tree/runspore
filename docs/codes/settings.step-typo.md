# `settings.step-typo`: Unknown step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.step-typo` reports:

```text
── Unknown step ────────────────── settings.step-typo/settings.json:11  settings.step-typo
Every key under "nodes" must be a step in workflow.dot, and there is no step "biuld".

 11 │     "biuld": { "action": "build" }
    │     ^^^^^^^

To fix: rename it to "build", the step of workflow.dot it is closest to.
```

The fix changes:

```text
settings.step-typo/settings.json:
 11 │-    "biuld": { "action": "build" }
 11 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/settings.step-typo`](../../crates/runspore-build/catalogue/examples/settings.step-typo).
