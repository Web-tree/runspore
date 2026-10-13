# `dot.unreachable`: Step that cannot be reached

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unreachable` reports:

```text
── Step that cannot be reached ── dot.unreachable/workflow.dot:6  dot.unreachable
Step "cleanup" can't be reached from the first step "build".

  6 │   cleanup [label="Clean up"];
    │   ^^^^^^^

To fix: draw an arrow into it, or remove it.
```

The fix changes:

```text
dot.unreachable/workflow.dot:
  9 │-  build -> done;
  9 │+  build -> cleanup;
```

The example folder is [`catalogue/examples/dot.unreachable`](../../crates/runspore-build/catalogue/examples/dot.unreachable).
