# `dot.bad-outcome`: Not an outcome name

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate dot.bad-outcome` reports:

```text
── Not an outcome name ── dot.bad-outcome/workflow.dot:8  dot.bad-outcome
The arrow outcome "ok,red" in workflow.dot is not one outcome name; an outcome has lowercase letters, digits and "-".

  8 │   build -> done [on="ok,red"];
    │                     ^^^^^^^^

To fix: draw one arrow per outcome, each with one name.
```

The fix changes:

```text
dot.bad-outcome/workflow.dot:
  8 │-  build -> done [on="ok,red"];
  8 │+  build -> done;
```

The example folder is [`catalogue/examples/dot.bad-outcome`](../../crates/runspore-build/catalogue/examples/dot.bad-outcome).
