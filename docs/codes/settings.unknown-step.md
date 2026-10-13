# `settings.unknown-step`: Unknown step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.unknown-step` reports:

```text
── Unknown step ────────────────── settings.unknown-step/settings.json:12  settings.unknown-step
Every key under "nodes" must be a step in workflow.dot, and there is no step "deploy".

 11 │     "build": { "action": "build" },
 12 │     "deploy": { "action": "build" }
    │     ^^^^^^^^

To fix: add step "deploy" to workflow.dot, or remove these settings.
```

The fix changes:

```text
settings.unknown-step/settings.json:
 11 │-    "build": { "action": "build" },
 12 │-    "deploy": { "action": "build" }
 11 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/settings.unknown-step`](../../crates/runspore-build/catalogue/examples/settings.unknown-step).
