# `settings.no-format`: No format

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.no-format` reports:

```text
── No format ────────────────── settings.no-format/settings.json:1  settings.no-format
settings.json does not say its format.

  1 │ {
    │ ^

To fix: add "format": "runspore.workflow/0.2".
```

The fix changes:

```text
settings.no-format/settings.json:
 11 │-  }
 11 │+  },
 12 │+  "format": "runspore.workflow/0.2"
```

The example folder is [`catalogue/examples/settings.no-format`](../../crates/runspore-build/catalogue/examples/settings.no-format).
