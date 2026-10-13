# `dot.no-name`: No workflow name

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-name` reports:

```text
── No workflow name ────────────────── dot.no-name/workflow.dot:1  dot.no-name
The graph in workflow.dot has no name, and its name is the workflow's name.

  1 │ digraph {

To fix: name it, for example   digraph "dot.no-name" {
```

The fix changes:

```text
dot.no-name/workflow.dot:
  1 │-digraph {
  1 │+digraph "dot.no-name" {
```

The example folder is [`catalogue/examples/dot.no-name`](../../crates/runspore-build/catalogue/examples/dot.no-name).
