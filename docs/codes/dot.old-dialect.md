# `dot.old-dialect`: Old dialect

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.old-dialect` reports:

```text
── Old dialect ────────────────── dot.old-dialect/workflow.dot:2  dot.old-dialect
workflow.dot is written in dialect "runspore/0.1", which this spore no longer reads.

  2 │   dialect = "runspore/0.1";
    │             ^^^^^^^^^^^^^^

To fix: change the dialect to "runspore/0.2", and "format" in settings.json to "runspore.workflow/0.2".
```

The fix changes:

```text
dot.old-dialect/workflow.dot:
  2 │-  dialect = "runspore/0.1";
  2 │+  dialect = "runspore/0.2";
```

The example folder is [`catalogue/examples/dot.old-dialect`](../../crates/runspore-build/catalogue/examples/dot.old-dialect).
