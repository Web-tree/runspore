# `command.program-not-found`: Program not found

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A warning: the workflow still builds and runs; `spore validate --strict` fails on it.

## Example

`spore validate command.program-not-found` reports:

```text
── Program not found (warning) ── command.program-not-found/settings.json:6  command.program-not-found
Program "mkae" of action "build" is not on this machine's PATH.

  6 │       "argv": ["mkae"],
    │                ^^^^^^

To fix: install it, or name it by its full path.
Why: the run may happen on another machine, so this is only a warning.
```

The fix changes:

```text
command.program-not-found/settings.json:
  6 │-      "argv": ["mkae"],
  6 │+      "argv": ["make"],
```

The example folder is [`catalogue/examples/command.program-not-found`](../../crates/runspore-build/catalogue/examples/command.program-not-found).
