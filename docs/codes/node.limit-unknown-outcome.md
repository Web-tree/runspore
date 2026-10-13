# `node.limit-unknown-outcome`: Loop limit for an outcome with no arrow

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.limit-unknown-outcome` reports:

```text
── Loop limit for an outcome with no arrow ── node.limit-unknown-outcome/settings.json:12  node.limit-unknown-outcome
Step "build" has a loop limit for "rde", but no arrow for "rde".

 12 │     "build": { "action": "build", "loopLimits": { "rde": 3 } },
    │                                                   ^^^^^

To fix: limit an outcome the step has an arrow for, or draw the arrow.
```

The fix changes:

```text
node.limit-unknown-outcome/settings.json:
 12 │-    "build": { "action": "build", "loopLimits": { "rde": 3 } },
 12 │+    "build": { "action": "build", "loopLimits": { "red": 3 } },
```

The example folder is [`catalogue/examples/node.limit-unknown-outcome`](../../crates/runspore-build/catalogue/examples/node.limit-unknown-outcome).
