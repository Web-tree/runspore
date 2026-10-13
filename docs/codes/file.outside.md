# `file.outside`: File outside the folder

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.outside` reports:

```text
── File outside the folder ── file.outside/settings.json:11  file.outside
"../build.md" leads outside the workflow folder.

 11 │     "build": { "action": "build", "instructions": "../build.md" }
    │                                                   ^^^^^^^^^^^^^

To fix: copy the file into the workflow folder, and name it there.
Why: the workflow artifact seals the folder's files, so a run sees the same files wherever it runs.
```

The fix changes:

```text
file.outside/settings.json:
 11 │-    "build": { "action": "build", "instructions": "../build.md" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.outside`](../../crates/runspore-build/catalogue/examples/file.outside).
