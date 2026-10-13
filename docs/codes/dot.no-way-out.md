# `dot.no-way-out`: No way to an end

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-way-out` reports:

```text
── No way to an end ────────────────── dot.no-way-out/workflow.dot:6  dot.no-way-out
From step "approve", no path of arrows leads to an end.

  6 │   approve [kind="await-signal"];
    │   ^^^^^^^

To fix: draw an arrow from it, or from a step after it, to a "complete" or "fail" step.
```

The fix changes:

```text
dot.no-way-out/workflow.dot:
 12 │+  approve -> done [on="approved"];
```

The example folder is [`catalogue/examples/dot.no-way-out`](../../crates/runspore-build/catalogue/examples/dot.no-way-out).
