# `dot.repeated-attribute`: Attribute given twice

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.repeated-attribute` reports:

```text
── Attribute given twice ── dot.repeated-attribute/workflow.dot:4  dot.repeated-attribute
Attribute "start" is given twice in workflow.dot.

  4 │   start   = "build";
    │   ^^^^^

To fix: keep one of them.
```

The fix changes:

```text
dot.repeated-attribute/workflow.dot:
  4 │-  start   = "build";
```

The example folder is [`catalogue/examples/dot.repeated-attribute`](../../crates/runspore-build/catalogue/examples/dot.repeated-attribute).
