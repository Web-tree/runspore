# `dot.number-id`: Unquoted number

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.number-id` reports:

```text
── Unquoted number ────────────────── dot.number-id/workflow.dot:5  dot.number-id
"2nd" in workflow.dot starts like a number, so it is not a name.

  5 │   build [label=2nd];
    │                ^^^

To fix: put it in quotes: "2nd"
```

The fix changes:

```text
dot.number-id/workflow.dot:
  5 │-  build [label=2nd];
  5 │+  build [label="2nd"];
```

The example folder is [`catalogue/examples/dot.number-id`](../../crates/runspore-build/catalogue/examples/dot.number-id).
