# `command.no-argv`: Missing "argv"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.no-argv` reports:

```text
── Missing "argv" ────────────────── command.no-argv/settings.json:4  command.no-argv
Action "build" runs a command, but has no "argv" to say which.

  4 │     "build": {
    │     ^^^^^^^

To fix: add "argv": the program, then each argument as its own item, such as ["make", "test"].
```

The fix changes:

```text
command.no-argv/settings.json:
  6 │+      "argv": ["make"],
```

The example folder is [`catalogue/examples/command.no-argv`](../../crates/runspore-build/catalogue/examples/command.no-argv).
