# `dot.no-value`: Attribute with no value

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-value` reports:

```text
── Attribute with no value ── dot.no-value/workflow.dot:5  dot.no-value
Attribute "bold" in workflow.dot has no value.

  5 │   build [label="Build the release", bold];
    │                                     ^^^^

To fix: give it one, as in bold="…".
```

The fix changes:

```text
dot.no-value/workflow.dot:
  5 │-  build [label="Build the release", bold];
  5 │+  build [label="Build the release"];
```

The example folder is [`catalogue/examples/dot.no-value`](../../crates/runspore-build/catalogue/examples/dot.no-value).
