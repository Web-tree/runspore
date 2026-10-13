# `dot.unknown-attribute`: Unknown attribute

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unknown-attribute` reports:

```text
── Unknown attribute ────────────────── dot.unknown-attribute/workflow.dot:5  dot.unknown-attribute
Attribute "priority" in workflow.dot is not one spore reads.

  5 │   build [label="Build the release", priority=1];
    │                                     ^^^^^^^^

To fix: remove it, or use one spore reads: kind, on, or a render attribute such as label, shape or color.
```

The fix changes:

```text
dot.unknown-attribute/workflow.dot:
  5 │-  build [label="Build the release", priority=1];
  5 │+  build [label="Build the release"];
```

The example folder is [`catalogue/examples/dot.unknown-attribute`](../../crates/runspore-build/catalogue/examples/dot.unknown-attribute).
