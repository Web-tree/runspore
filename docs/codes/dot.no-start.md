# `dot.no-start`: No first step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate nightly-backup` reports:

```text
── No first step ────────────────── nightly-backup/workflow.dot:1  dot.no-start
The workflow does not say which step runs first.

  1 │ digraph "nightly-backup" {

To fix: add this line inside the braces:   start = "dump";
```

The fix changes:

```text
nightly-backup/workflow.dot:
  3 │+  start   = "dump";
```

The example folder is [`catalogue/examples/nightly-backup`](../../crates/runspore-build/catalogue/examples/nightly-backup).
