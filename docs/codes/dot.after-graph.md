# `dot.after-graph`: Text after the graph

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.after-graph` reports:

```text
── Text after the graph ── dot.after-graph/workflow.dot:10  dot.after-graph
workflow.dot goes on after the closing brace of its graph.

 10 │ }
    │ ^

To fix: move it inside the braces, or remove it.
```

The fix changes:

```text
dot.after-graph/workflow.dot:
 10 │-}
```

The example folder is [`catalogue/examples/dot.after-graph`](../../crates/runspore-build/catalogue/examples/dot.after-graph).
