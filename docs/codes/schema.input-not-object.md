# `schema.input-not-object`: Input schema that is not an object

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate schema.input-not-object` reports:

```text
── Input schema that is not an object ── schema.input-not-object/settings.json:3  schema.input-not-object
The input schema of "release" does not have "type": "object" at its root.

  3 │   "inputSchema": { "type": "string" },
    │                            ^^^^^^^^

To fix: give its root "type": "object".
Why: A run's input is a JSON object, its fields named, so steps can read them.
```

The fix changes:

```text
schema.input-not-object/settings.json:
  3 │-  "inputSchema": { "type": "string" },
  3 │+  "inputSchema": { "type": "object" },
```

The example folder is [`catalogue/examples/schema.input-not-object`](../../crates/runspore-build/catalogue/examples/schema.input-not-object).
