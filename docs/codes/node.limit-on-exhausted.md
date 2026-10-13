# `node.limit-on-exhausted`: Loop limit on "exhausted"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.limit-on-exhausted` reports:

```text
── Loop limit on "exhausted" ── node.limit-on-exhausted/settings.json:12  node.limit-on-exhausted
Step "build" has a loop limit for "exhausted", the outcome a limit gives when it runs out.

 12 │     "build": { "action": "build", "loopLimits": { "red": 3, "exhausted": 1 } },
    │                                                             ^^^^^^^^^^^

To fix: remove it, and limit the outcome that loops instead.
```

The fix changes:

```text
node.limit-on-exhausted/settings.json:
 12 │-    "build": { "action": "build", "loopLimits": { "red": 3, "exhausted": 1 } },
 12 │+    "build": { "action": "build", "loopLimits": { "red": 3 } },
```

The example folder is [`catalogue/examples/node.limit-on-exhausted`](../../crates/runspore-build/catalogue/examples/node.limit-on-exhausted).
