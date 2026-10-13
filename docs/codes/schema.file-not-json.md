# `schema.file-not-json`: Schema file that is not JSON

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate schema.file-not-json` reports:

```text
── Schema file that is not JSON ── schema.file-not-json/settings.json:11  schema.file-not-json
The schema file "schemas/result.json" for "ok" on step "build" is not JSON that spore can read.

 11 │     "build": { "action": "build", "outputSchemas": { "ok": "schemas/result.json" } }
    │                                                            ^^^^^^^^^^^^^^^^^^^^^

To fix: make it a JSON object, with whole numbers only.
```

The fix changes:

```text
schema.file-not-json/schemas/result.json:
  1 │-type: object
  1 │+{"type": "object"}
```

The example folder is [`catalogue/examples/schema.file-not-json`](../../crates/runspore-build/catalogue/examples/schema.file-not-json).
