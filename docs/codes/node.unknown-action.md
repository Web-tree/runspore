# `node.unknown-action`: Unknown action

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.unknown-action` reports:

```text
── Unknown action ────────────────── node.unknown-action/settings.json:11  node.unknown-action
Step "build" runs action "biuld", which "actions" does not define.

 11 │     "build": { "action": "biuld" }
    │                          ^^^^^^^

To fix: name an action that "actions" defines, or define action "biuld" there.
```

The fix changes:

```text
node.unknown-action/settings.json:
 11 │-    "build": { "action": "biuld" }
 11 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/node.unknown-action`](../../crates/runspore-build/catalogue/examples/node.unknown-action).
