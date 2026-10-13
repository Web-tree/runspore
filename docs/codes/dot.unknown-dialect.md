# `dot.unknown-dialect`: Unknown dialect

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unknown-dialect` reports:

```text
── Unknown dialect ────────────────── dot.unknown-dialect/workflow.dot:2  dot.unknown-dialect
workflow.dot is written in dialect "runspore/2", which this spore does not know.

  2 │   dialect = "runspore/2";
    │             ^^^^^^^^^^^^

To fix: change it to "runspore/0.2", the dialect this spore reads.
```

The fix changes:

```text
dot.unknown-dialect/workflow.dot:
  2 │-  dialect = "runspore/2";
  2 │+  dialect = "runspore/0.2";
```

The example folder is [`catalogue/examples/dot.unknown-dialect`](../../crates/runspore-build/catalogue/examples/dot.unknown-dialect).
