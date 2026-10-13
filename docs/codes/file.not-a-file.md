# `file.not-a-file`: Not a file

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.not-a-file` reports:

```text
── Not a file ────────────────── file.not-a-file/settings.json:11  file.not-a-file
"steps" is not a regular file.

 11 │     "build": { "action": "build", "instructions": "steps" }
    │                                                   ^^^^^^^

To fix: name a file, not a folder.
```

The fix changes:

```text
file.not-a-file/settings.json:
 11 │-    "build": { "action": "build", "instructions": "steps" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.not-a-file`](../../crates/runspore-build/catalogue/examples/file.not-a-file).
