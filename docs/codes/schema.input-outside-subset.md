# `schema.input-outside-subset`: Input schema outside what spore checks

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate schema.input-outside-subset` reports:

```text
── Input schema outside what spore checks ── schema.input-outside-subset/settings.json:5  schema.input-outside-subset
The input schema of "release" is outside what spore checks, at "/properties/version/pattern": `pattern` is not in the schema subset.

  5 │     "properties": { "version": { "type": "string", "pattern": "^v[0-9]+" } }
    │                                                    ^^^^^^^^^

To fix: use only the parts of JSON Schema that spec/schema.md lists.
```

The fix changes:

```text
schema.input-outside-subset/settings.json:
  5 │-    "properties": { "version": { "type": "string", "pattern": "^v[0-9]+" } }
  5 │+    "properties": { "version": { "type": "string" } }
```

The example folder is [`catalogue/examples/schema.input-outside-subset`](../../crates/runspore-build/catalogue/examples/schema.input-outside-subset).
