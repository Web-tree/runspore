# `dot.bad-step-name`: Step name with other characters

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.bad-step-name` reports:

```text
── Step name with other characters ── dot.bad-step-name/workflow.dot:5  dot.bad-step-name
Step "build step" has a name spore cannot use; a step name has letters, digits, "_" and "-", at most 64 of them.

  5 │   "build step" [label="Build the release"];
    │   ^^^^^^^^^^^^

To fix: rename it to "build-step" everywhere in workflow.dot.
```

The fix changes:

```text
dot.bad-step-name/workflow.dot:
  5 │-  "build step" [label="Build the release"];
  6 │-  done  [kind="complete"];
  7 │-
  8 │-  "build step" -> done;
  5 │+  "build-step" [label="Build the release"];
  6 │+  done  [kind="complete"];
  7 │+
  8 │+  "build-step" -> done;
```

The example folder is [`catalogue/examples/dot.bad-step-name`](../../crates/runspore-build/catalogue/examples/dot.bad-step-name).
