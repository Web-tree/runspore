# `dot.kind-conflict`: Two kinds for one step

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.kind-conflict` reports:

```text
── Two kinds for one step ── dot.kind-conflict/workflow.dot:7  dot.kind-conflict
Step "done" is of kind "fail" here, and of kind "complete" on line 6.

  7 │   done  [kind="fail"];
    │               ^^^^^^

To fix: give the step one kind.
```

The fix changes:

```text
dot.kind-conflict/workflow.dot:
  7 │-  done  [kind="fail"];
```

The example folder is [`catalogue/examples/dot.kind-conflict`](../../crates/runspore-build/catalogue/examples/dot.kind-conflict).
