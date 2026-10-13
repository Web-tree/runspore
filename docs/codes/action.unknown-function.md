# `action.unknown-function`: Unknown native function

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.unknown-function` reports:

```text
── Unknown native function ── action.unknown-function/settings.json:6  action.unknown-function
Action "build" calls the native function "build", which this spore does not have.

  6 │       "function": "build",
    │                   ^^^^^^^

To fix: run a program instead, with "kind": "command" and "argv".
```

The fix changes:

```text
action.unknown-function/settings.json:
  5 │-      "kind": "native",
  6 │-      "function": "build",
  5 │+      "kind": "command",
  6 │+      "argv": ["make"],
```

The example folder is [`catalogue/examples/action.unknown-function`](../../crates/runspore-build/catalogue/examples/action.unknown-function).
