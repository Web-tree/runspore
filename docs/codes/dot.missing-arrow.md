# `dot.missing-arrow`: Missing arrow

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate nightly-backup` reports:

```text
── Missing arrow ────────────────── nightly-backup/workflow.dot:5  dot.missing-arrow
Step "upload" can end with the outcome "ok", but no arrow leaves it for "ok".

  5 │   upload [label="Upload the dump"];
    │   ^^^^^^

To fix: draw the arrow, for example   upload -> done;
Why: action "upload" lists no outcomes, so it ends with "ok", and the run would have nowhere to go.
```

The fix changes:

```text
nightly-backup/workflow.dot:
  9 │+  upload -> done;
```

The example folder is [`catalogue/examples/nightly-backup`](../../crates/runspore-build/catalogue/examples/nightly-backup).
