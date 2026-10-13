# `node.schema-reserved-outcome`: Schema for an outcome with no output

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.schema-reserved-outcome` reports:

```text
── Schema for an outcome with no output ── node.schema-reserved-outcome/settings.json:18  node.schema-reserved-outcome
Step "build" has a schema for "failed", which never carries output of its own.

 18 │         "failed": {
    │         ^^^^^^^^

To fix: remove it.
```

The fix changes:

```text
node.schema-reserved-outcome/settings.json:
 17 │-        },
 18 │-        "failed": {
 19 │-          "type": "object",
 20 │-          "properties": { "version": { "type": "string" } }
```

The example folder is [`catalogue/examples/node.schema-reserved-outcome`](../../crates/runspore-build/catalogue/examples/node.schema-reserved-outcome).
