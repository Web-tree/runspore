# `settings.wrong-format`: Wrong format

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.wrong-format` reports:

```text
── Wrong format ────────────────── settings.wrong-format/settings.json:2  settings.wrong-format
Format "runspore.workflow/0.1" does not agree with the dialect "runspore/0.2" of workflow.dot.

  2 │   "format": "runspore.workflow/0.1",
    │             ^^^^^^^^^^^^^^^^^^^^^^^

To fix: change it to "runspore.workflow/0.2".
```

The fix changes:

```text
settings.wrong-format/settings.json:
  2 │-  "format": "runspore.workflow/0.1",
  2 │+  "format": "runspore.workflow/0.2",
```

The example folder is [`catalogue/examples/settings.wrong-format`](../../crates/runspore-build/catalogue/examples/settings.wrong-format).
