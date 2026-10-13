# `dot.no-dialect`: No dialect line

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-dialect` reports:

```text
── No dialect line ────────────────── dot.no-dialect/workflow.dot:1  dot.no-dialect
workflow.dot does not say which DOT dialect it is written in.

  1 │ digraph "release" {

To fix: add this line inside the braces:   dialect = "runspore/0.2";
Why: the dialect says how spore reads the rest of the file, so spore reads nothing else until it is there.
```

The fix changes:

```text
dot.no-dialect/workflow.dot:
  2 │+  dialect = "runspore/0.2";
```

The example folder is [`catalogue/examples/dot.no-dialect`](../../crates/runspore-build/catalogue/examples/dot.no-dialect).
