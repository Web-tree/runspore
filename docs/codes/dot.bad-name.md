# `dot.bad-name`: Workflow name with other characters

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.bad-name` reports:

```text
── Workflow name with other characters ── dot.bad-name/workflow.dot:1  dot.bad-name
The workflow name "nightly release" in workflow.dot has characters a name cannot have; it may use letters, digits, ".", "_" and "-".

  1 │ digraph "nightly release" {
    │         ^^^^^^^^^^^^^^^^^

To fix: rename it to "nightly-release".
```

The fix changes:

```text
dot.bad-name/workflow.dot:
  1 │-digraph "nightly release" {
  1 │+digraph "nightly-release" {
```

The example folder is [`catalogue/examples/dot.bad-name`](../../crates/runspore-build/catalogue/examples/dot.bad-name).
