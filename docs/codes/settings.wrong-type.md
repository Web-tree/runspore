# `settings.wrong-type`: Wrong type

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.wrong-type` reports:

```text
── Wrong type ────────────────── settings.wrong-type/settings.json:6  settings.wrong-type
"make test" at "actions" → "build" → "argv" is a string, and it must be a list.

  6 │       "argv": "make test",
    │               ^^^^^^^^^^^

To fix: write a list here.
```

The fix changes:

```text
settings.wrong-type/settings.json:
  6 │-      "argv": "make test",
  6 │+      "argv": ["make", "test"],
```

The example folder is [`catalogue/examples/settings.wrong-type`](../../crates/runspore-build/catalogue/examples/settings.wrong-type).
