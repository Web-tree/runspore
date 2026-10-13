# `node.bad-signal`: Signal name with other characters

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.bad-signal` reports:

```text
── Signal name with other characters ── node.bad-signal/settings.json:12  node.bad-signal
Signal "release approval" of step "approve" has characters a signal name cannot have; it may use letters, digits, ".", "_" and "-", at most 64.

 12 │     "approve": { "signal": "release approval" }
    │                            ^^^^^^^^^^^^^^^^^^

To fix: rename it with those characters only.
```

The fix changes:

```text
node.bad-signal/settings.json:
 12 │-    "approve": { "signal": "release approval" }
 12 │+    "approve": { "signal": "release-approval" }
```

The example folder is [`catalogue/examples/node.bad-signal`](../../crates/runspore-build/catalogue/examples/node.bad-signal).
