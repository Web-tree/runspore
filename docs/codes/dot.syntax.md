# `dot.syntax`: DOT syntax

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.syntax` reports:

```text
── DOT syntax ────────────────── dot.syntax/workflow.dot:8  dot.syntax
workflow.dot has ";" here, where spore expects a node ID.

  8 │   build -> ;
    │            ^

To fix: check this line against the DOT spore reads: steps, arrows written with "->", and attributes in [brackets].
```

The fix changes:

```text
dot.syntax/workflow.dot:
  8 │-  build -> ;
  8 │+  build -> done;
```

The example folder is [`catalogue/examples/dot.syntax`](../../crates/runspore-build/catalogue/examples/dot.syntax).
