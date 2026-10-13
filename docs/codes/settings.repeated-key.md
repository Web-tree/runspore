# `settings.repeated-key`: Key given twice

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.repeated-key` reports:

```text
── Key given twice ────────────────── settings.repeated-key/settings.json:8  settings.repeated-key
"effect" is given twice in "actions" → "build", and JSON keeps only the last one.

  8 │       "effect": "idempotent"
    │       ^^^^^^^^

To fix: remove the one you do not want.
```

The fix changes:

```text
settings.repeated-key/settings.json:
  7 │-      "effect": "pure",
```

The example folder is [`catalogue/examples/settings.repeated-key`](../../crates/runspore-build/catalogue/examples/settings.repeated-key).
