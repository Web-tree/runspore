# `file.missing`: File not found

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.missing` reports:

```text
── File not found ────────────────── file.missing/settings.json:11  file.missing
"steps/biuld.md" names no file in the workflow folder.

 11 │     "build": { "action": "build", "instructions": "steps/biuld.md" }
    │                                                   ^^^^^^^^^^^^^^^^

To fix: name a file inside the folder; paths are relative to the folder.
```

The fix changes:

```text
file.missing/settings.json:
 11 │-    "build": { "action": "build", "instructions": "steps/biuld.md" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.missing`](../../crates/runspore-build/catalogue/examples/file.missing).
