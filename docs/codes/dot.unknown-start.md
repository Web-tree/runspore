# `dot.unknown-start`: Unknown first step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unknown-start` reports:

```text
── Unknown first step ────────────────── dot.unknown-start/workflow.dot:3  dot.unknown-start
The first step "biuld" is not a step in workflow.dot.

  3 │   start   = "biuld";
    │             ^^^^^^^

To fix: name one of its steps, for example "build".
```

The fix changes:

```text
dot.unknown-start/workflow.dot:
  3 │-  start   = "biuld";
  3 │+  start   = "build";
```

The example folder is [`catalogue/examples/dot.unknown-start`](../../crates/runspore-build/catalogue/examples/dot.unknown-start).
