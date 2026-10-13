# `action.unknown-kind`: Unknown kind of action

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.unknown-kind` reports:

```text
── Unknown kind of action ── action.unknown-kind/settings.json:5  action.unknown-kind
Action "build" has kind "shell", which this spore cannot run.

  5 │       "kind": "shell",
    │               ^^^^^^^

To fix: use one of "command" | "native" | "external".
```

The fix changes:

```text
action.unknown-kind/settings.json:
  5 │-      "kind": "shell",
  5 │+      "kind": "command",
```

The example folder is [`catalogue/examples/action.unknown-kind`](../../crates/runspore-build/catalogue/examples/action.unknown-kind).
