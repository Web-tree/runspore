# `dot.missing-outcome-arrow`: Missing arrow

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.missing-outcome-arrow` reports:

```text
── Missing arrow ────────────────── dot.missing-outcome-arrow/workflow.dot:5  dot.missing-outcome-arrow
Step "build" can end with the outcome "red", but no arrow leaves it for "red".

  5 │   build [label="Build the release"];
    │   ^^^^^

To fix: draw the arrow, for example   build -> done [on="red"];
Why: action "build" lists ["ok", "red"], so the step can end with "red", and the run would have nowhere to go.
```

The fix changes:

```text
dot.missing-outcome-arrow/workflow.dot:
  9 │+  build -> done [on="red"];
```

The example folder is [`catalogue/examples/dot.missing-outcome-arrow`](../../crates/runspore-build/catalogue/examples/dot.missing-outcome-arrow).
