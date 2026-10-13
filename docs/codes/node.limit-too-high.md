# `node.limit-too-high`: Loop limit too high

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.limit-too-high` reports:

```text
── Loop limit too high ── node.limit-too-high/settings.json:12  node.limit-too-high
The loop limit 20 for "red" on step "build" leaves no room under the visit limit 16 of the workflow.

 12 │     "build": { "action": "build", "loopLimits": { "red": 20 } },
    │                                                          ^^

To fix: lower it to 15, or raise "maxVisitsPerNode" in "limits".
Why: a step is entered at most "maxVisitsPerNode" times, and a loop that runs out of tries enters it once more.
```

The fix changes:

```text
node.limit-too-high/settings.json:
 12 │-    "build": { "action": "build", "loopLimits": { "red": 20 } },
 12 │+    "build": { "action": "build", "loopLimits": { "red": 15 } },
```

The example folder is [`catalogue/examples/node.limit-too-high`](../../crates/runspore-build/catalogue/examples/node.limit-too-high).
