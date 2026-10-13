# `command.bad-exit-code`: Not an exit code

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.bad-exit-code` reports:

```text
── Not an exit code ────────────────── command.bad-exit-code/settings.json:9  command.bad-exit-code
"one" in "exitOutcomes" of action "build" is not an exit code.

  9 │       "exitOutcomes": { "one": "red" }
    │                         ^^^^^

To fix: use a whole number, such as "1".
```

The fix changes:

```text
command.bad-exit-code/settings.json:
  9 │-      "exitOutcomes": { "one": "red" }
  9 │+      "exitOutcomes": { "1": "red" }
```

The example folder is [`catalogue/examples/command.bad-exit-code`](../../crates/runspore-build/catalogue/examples/command.bad-exit-code).
