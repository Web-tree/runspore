# `dot.attribute-typo`: Unknown attribute

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.attribute-typo` reports:

```text
── Unknown attribute ────────────────── dot.attribute-typo/workflow.dot:5  dot.attribute-typo
Attribute "lable" in workflow.dot is not one spore reads.

  5 │   build [lable="Build the release"];
    │          ^^^^^

To fix: rename it to label.
```

The fix changes:

```text
dot.attribute-typo/workflow.dot:
  5 │-  build [lable="Build the release"];
  5 │+  build [label="Build the release"];
```

The example folder is [`catalogue/examples/dot.attribute-typo`](../../crates/runspore-build/catalogue/examples/dot.attribute-typo).
