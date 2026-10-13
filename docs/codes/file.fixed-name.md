# `file.fixed-name`: Reference to workflow.dot or settings.json

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.fixed-name` reports:

```text
── Reference to workflow.dot or settings.json ── file.fixed-name/settings.json:11  file.fixed-name
"workflow.dot" names a file every build already includes.

 11 │     "build": { "action": "build", "instructions": "workflow.dot" }
    │                                                   ^^^^^^^^^^^^^^

To fix: name another file.
```

The fix changes:

```text
file.fixed-name/settings.json:
 11 │-    "build": { "action": "build", "instructions": "workflow.dot" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.fixed-name`](../../crates/runspore-build/catalogue/examples/file.fixed-name).
