# `settings.empty`: Empty

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.empty` reports:

```text
── Empty ────────────────── settings.empty/settings.json:6  settings.empty
"actions" → "build" → "argv" is empty.

  6 │       "argv": [],
    │               ^^

To fix: give it at least one entry, or remove it.
```

The fix changes:

```text
settings.empty/settings.json:
  6 │-      "argv": [],
  6 │+      "argv": ["make"],
```

The example folder is [`catalogue/examples/settings.empty`](../../crates/runspore-build/catalogue/examples/settings.empty).
