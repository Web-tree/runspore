# `file.backslash`: Backslash in a path

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate file.backslash` reports:

```text
── Backslash in a path ── file.backslash/settings.json:11  file.backslash
"steps\\build.md" separates folders with a backslash.

 11 │     "build": { "action": "build", "instructions": "steps\\build.md" }
    │                                                   ^^^^^^^^^^^^^^^^^

To fix: separate folders with "/".
```

The fix changes:

```text
file.backslash/settings.json:
 11 │-    "build": { "action": "build", "instructions": "steps\\build.md" }
 11 │+    "build": { "action": "build", "instructions": "steps/build.md" }
```

The example folder is [`catalogue/examples/file.backslash`](../../crates/runspore-build/catalogue/examples/file.backslash).
