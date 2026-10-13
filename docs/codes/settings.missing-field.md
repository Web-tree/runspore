# `settings.missing-field`: Missing key

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.missing-field` reports:

```text
── Missing key ────────────────── settings.missing-field/settings.json:12  settings.missing-field
This object in settings.json needs "message".

 12 │     "broken": { "error": { "code": "build.broken" } }
    │                 ^^^^^^^

To fix: add "message".
```

The fix changes:

```text
settings.missing-field/settings.json:
 12 │-    "broken": { "error": { "code": "build.broken" } }
 12 │+    "broken": { "error": { "code": "build.broken", "message": "The build broke." } }
```

The example folder is [`catalogue/examples/settings.missing-field`](../../crates/runspore-build/catalogue/examples/settings.missing-field).
