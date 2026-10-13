# `schema.input-file-not-json`: Input schema file that is not JSON

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate schema.input-file-not-json` reports:

```text
── Input schema file that is not JSON ── schema.input-file-not-json/settings.json:3  schema.input-file-not-json
The input schema file "schemas/input.json" of "release" is not JSON that spore can read.

  3 │   "inputSchema": "schemas/input.json",
    │                  ^^^^^^^^^^^^^^^^^^^^

To fix: make it a JSON object, with whole numbers only.
```

The fix changes:

```text
schema.input-file-not-json/schemas/input.json:
  1 │-type: object
  2 │-properties:
  3 │-  version: { type: string }
  1 │+{ "type": "object", "properties": { "version": { "type": "string" } } }
```

The example folder is [`catalogue/examples/schema.input-file-not-json`](../../crates/runspore-build/catalogue/examples/schema.input-file-not-json).
