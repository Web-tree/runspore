# `dot.no-arrows`: Signal step with no arrows

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.no-arrows` reports:

```text
── Signal step with no arrows ── dot.no-arrows/workflow.dot:6  dot.no-arrows
Step "approve" waits for a signal, but no arrow leaves it.

  6 │   approve [kind="await-signal"];
    │   ^^^^^^^

To fix: draw an arrow for each outcome the signal can bring, for example   approve -> done [on="received"];
```

The fix changes:

```text
dot.no-arrows/workflow.dot:
 10 │+  approve -> done [on="received"];
```

The example folder is [`catalogue/examples/dot.no-arrows`](../../crates/runspore-build/catalogue/examples/dot.no-arrows).
