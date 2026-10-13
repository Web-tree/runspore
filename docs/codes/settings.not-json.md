# `settings.not-json`: Not JSON

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.not-json` reports:

```text
── Not JSON ────────────────── settings.not-json/settings.json:8  settings.not-json
settings.json stops being JSON here.

  8 │     }
    │     ^

To fix: correct the JSON at this spot; a missing comma, quote or brace is the usual cause.
```

The fix changes:

```text
settings.not-json/settings.json:
  7 │-      "effect": "idempotent",
  7 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/settings.not-json`](../../crates/runspore-build/catalogue/examples/settings.not-json).
