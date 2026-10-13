# `dot.misplaced-attribute`: Attribute in the wrong place

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.misplaced-attribute` reports:

```text
── Attribute in the wrong place ── dot.misplaced-attribute/workflow.dot:5  dot.misplaced-attribute
Attribute "start" in workflow.dot belongs on the graph, not on a step.

  5 │   build [label="Build the release", start="yes"];
    │                                     ^^^^^

To fix: move it to the graph.
```

The fix changes:

```text
dot.misplaced-attribute/workflow.dot:
  5 │-  build [label="Build the release", start="yes"];
  5 │+  build [label="Build the release"];
```

The example folder is [`catalogue/examples/dot.misplaced-attribute`](../../crates/runspore-build/catalogue/examples/dot.misplaced-attribute).
