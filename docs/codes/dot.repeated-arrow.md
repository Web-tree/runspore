# `dot.repeated-arrow`: Two arrows for one outcome

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.repeated-arrow` reports:

```text
── Two arrows for one outcome ── dot.repeated-arrow/workflow.dot:10  dot.repeated-arrow
Step "build" has two arrows for the outcome "ok", and an outcome leads to one step.

 10 │   build -> broken;
    │   ^^^^^^^^^^^^^^^

To fix: remove one, or give it another outcome with [on="…"].
```

The fix changes:

```text
dot.repeated-arrow/workflow.dot:
 10 │-  build -> broken;
 10 │+  build -> broken [on="failed"];
```

The example folder is [`catalogue/examples/dot.repeated-arrow`](../../crates/runspore-build/catalogue/examples/dot.repeated-arrow).
