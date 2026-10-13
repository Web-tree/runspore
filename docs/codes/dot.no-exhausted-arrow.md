# `dot.no-exhausted-arrow`: No arrow for "exhausted"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-exhausted-arrow` reports:

```text
── No arrow for "exhausted" ── dot.no-exhausted-arrow/workflow.dot:5  dot.no-exhausted-arrow
Step "build" has loop limits, but no arrow for "exhausted", the outcome it ends with when a limit runs out.

  5 │   build [label="Build the release"];
    │   ^^^^^

To fix: draw it, for example   build -> done [on="exhausted"];
```

The fix changes:

```text
dot.no-exhausted-arrow/workflow.dot:
 10 │+  build -> done [on="exhausted"];
```

The example folder is [`catalogue/examples/dot.no-exhausted-arrow`](../../crates/runspore-build/catalogue/examples/dot.no-exhausted-arrow).
