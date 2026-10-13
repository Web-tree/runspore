# `dot.unclosed`: Never closed

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unclosed` reports:

```text
── Never closed ────────────────── dot.unclosed/workflow.dot:8  dot.unclosed
a /* comment that starts here in workflow.dot is never closed.

  8 │   /* the last arrow
    │   ^^

To fix: close it with */.
```

The fix changes:

```text
dot.unclosed/workflow.dot:
  8 │-  /* the last arrow
  8 │+  /* the last arrow */
```

The example folder is [`catalogue/examples/dot.unclosed`](../../crates/runspore-build/catalogue/examples/dot.unclosed).
