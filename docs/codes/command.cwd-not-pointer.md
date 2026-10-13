# `command.cwd-not-pointer`: Working folder from input not a JSON Pointer

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.cwd-not-pointer` reports:

```text
── Working folder from input not a JSON Pointer ── command.cwd-not-pointer/settings.json:8  command.cwd-not-pointer
"checkout" in "cwd" of action "build" is not a JSON Pointer into the step's input; a pointer is empty or starts with "/".

  8 │       "cwd": { "fromInput": "checkout" }
    │                             ^^^^^^^^^^

To fix: write where the step's input holds the folder, starting with "/", such as "/checkout".
```

The fix changes:

```text
command.cwd-not-pointer/settings.json:
  8 │-      "cwd": { "fromInput": "checkout" }
  8 │+      "cwd": { "fromInput": "/checkout" }
```

The example folder is [`catalogue/examples/command.cwd-not-pointer`](../../crates/runspore-build/catalogue/examples/command.cwd-not-pointer).
