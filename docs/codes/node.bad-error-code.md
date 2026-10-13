# `node.bad-error-code`: Not an error code

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.bad-error-code` reports:

```text
── Not an error code ────────────────── node.bad-error-code/settings.json:12  node.bad-error-code
Error code "Build Broken" of step "broken" is not a code; a code has lowercase letters, digits, "." and "-", and starts with a letter or digit.

 12 │     "broken": { "error": { "code": "Build Broken", "message": "The build broke." } }
    │                                    ^^^^^^^^^^^^^^

To fix: rename it, for example "deploy.failed".
```

The fix changes:

```text
node.bad-error-code/settings.json:
 12 │-    "broken": { "error": { "code": "Build Broken", "message": "The build broke." } }
 12 │+    "broken": { "error": { "code": "build.broken", "message": "The build broke." } }
```

The example folder is [`catalogue/examples/node.bad-error-code`](../../crates/runspore-build/catalogue/examples/node.bad-error-code).
