# `dot.exhausted-arrow`: Arrow for "exhausted" without a loop limit

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.exhausted-arrow` reports:

```text
── Arrow for "exhausted" without a loop limit ── dot.exhausted-arrow/workflow.dot:10  dot.exhausted-arrow
Step "build" has an arrow for "exhausted", but no loop limit that could run out.

 10 │   build -> broken [on="exhausted"];
    │   ^^^^^^^^^^^^^^^

To fix: add "loopLimits" to step "build" in settings.json, or remove the arrow.
```

The fix changes:

```text
dot.exhausted-arrow/workflow.dot:
 10 │-  build -> broken [on="exhausted"];
 10 │+  build -> broken [on="failed"];
```

The example folder is [`catalogue/examples/dot.exhausted-arrow`](../../crates/runspore-build/catalogue/examples/dot.exhausted-arrow).
