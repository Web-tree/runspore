# `node.schema-unknown-outcome`: Schema for an outcome the step does not have

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.schema-unknown-outcome` reports:

```text
── Schema for an outcome the step does not have ── node.schema-unknown-outcome/settings.json:14  node.schema-unknown-outcome
Step "build" has a schema for "okay", an outcome it never ends with.

 14 │         "okay": {
    │         ^^^^^^

To fix: rename it to an outcome the step ends with, or remove it.
```

The fix changes:

```text
node.schema-unknown-outcome/settings.json:
 14 │-        "okay": {
 14 │+        "ok": {
```

The example folder is [`catalogue/examples/node.schema-unknown-outcome`](../../crates/runspore-build/catalogue/examples/node.schema-unknown-outcome).
