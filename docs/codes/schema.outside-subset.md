# `schema.outside-subset`: Schema outside what spore checks

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate schema.outside-subset` reports:

```text
── Schema outside what spore checks ── schema.outside-subset/settings.json:14  schema.outside-subset
The schema for "ok" on step "build" is outside what spore checks, at "/properties/size/type": `number` is not allowed: data holds integers only; use `integer`.

 14 │         "ok": { "type": "object", "properties": { "size": { "type": "number" } } }
    │                                                             ^^^^^^

To fix: use only the parts of JSON Schema that spec/schema.md lists.
```

The fix changes:

```text
schema.outside-subset/settings.json:
 14 │-        "ok": { "type": "object", "properties": { "size": { "type": "number" } } }
 14 │+        "ok": {
 15 │+          "type": "object",
 16 │+          "properties": { "size": { "type": "integer" } }
 17 │+        }
```

The example folder is [`catalogue/examples/schema.outside-subset`](../../crates/runspore-build/catalogue/examples/schema.outside-subset).
