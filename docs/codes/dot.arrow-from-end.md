# `dot.arrow-from-end`: Arrow from an end

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.arrow-from-end` reports:

```text
── Arrow from an end ────────────────── dot.arrow-from-end/workflow.dot:10  dot.arrow-from-end
Step "done" is of kind "complete", where the run ends, so no arrow may leave it.

 10 │   done -> broken;
    │   ^^^^^^^^^^^^^^

To fix: remove the arrow   done -> broken;
```

The fix changes:

```text
dot.arrow-from-end/workflow.dot:
 10 │-  done -> broken;
```

The example folder is [`catalogue/examples/dot.arrow-from-end`](../../crates/runspore-build/catalogue/examples/dot.arrow-from-end).
