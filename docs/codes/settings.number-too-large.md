# `settings.number-too-large`: Number too large

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.number-too-large` reports:

```text
── Number too large ────────────────── settings.number-too-large/settings.json:4  settings.number-too-large
9007199254740993 at "limits" → "maxActivations" is too large for spore to keep exactly.

  4 │     "maxActivations": 9007199254740993
    │                       ^^^^^^^^^^^^^^^^

To fix: use a number of at most 9007199254740991.
```

The fix changes:

```text
settings.number-too-large/settings.json:
  4 │-    "maxActivations": 9007199254740993
  4 │+    "maxActivations": 10000
```

The example folder is [`catalogue/examples/settings.number-too-large`](../../crates/runspore-build/catalogue/examples/settings.number-too-large).
