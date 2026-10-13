# `dot.unknown-kind`: Unknown kind

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.unknown-kind` reports:

```text
── Unknown kind ────────────────── dot.unknown-kind/workflow.dot:6  dot.unknown-kind
Step "done" has kind "completed", which spore does not know.

  6 │   done  [kind="completed"];
    │               ^^^^^^^^^^^

To fix: use one of "activity" | "await-signal" | "complete" | "fail".
```

The fix changes:

```text
dot.unknown-kind/workflow.dot:
  6 │-  done  [kind="completed"];
  6 │+  done  [kind="complete"];
```

The example folder is [`catalogue/examples/dot.unknown-kind`](../../crates/runspore-build/catalogue/examples/dot.unknown-kind).
