# `dot.unsupported`: DOT that spore does not read

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unsupported` reports:

```text
── DOT that spore does not read ── dot.unsupported/workflow.dot:8  dot.unsupported
workflow.dot uses subgraphs, which spore does not read.

  8 │   subgraph cluster_main { build -> done; }
    │   ^^^^^^^^

To fix: write it with what spore reads: steps, arrows written with "->", and attributes in [brackets].
Why: spore reads a small part of DOT, so that every workflow.dot means one thing.
```

The fix changes:

```text
dot.unsupported/workflow.dot:
  8 │-  subgraph cluster_main { build -> done; }
  8 │+  build -> done;
```

The example folder is [`catalogue/examples/dot.unsupported`](../../crates/runspore-build/catalogue/examples/dot.unsupported).
