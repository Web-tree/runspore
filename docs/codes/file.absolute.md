# `file.absolute`: Absolute path

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.absolute` reports:

```text
── Absolute path ────────────────── file.absolute/settings.json:11  file.absolute
"/home/me/steps/build.md" is an absolute path, and a file reference names a file inside the workflow folder.

 11 │     "build": { "action": "build", "instructions": "/home/me/steps/build.md" }
    │                                                   ^^^^^^^^^^^^^^^^^^^^^^^^^

To fix: write the path relative to the workflow folder.
```

The fix changes:

```text
file.absolute/settings.json:
 11 │-    "build": { "action": "build", "instructions": "/home/me/steps/build.md" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.absolute`](../../crates/runspore-build/catalogue/examples/file.absolute).
