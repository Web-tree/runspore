# `node.from-dot`: Key that comes from workflow.dot

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.from-dot` reports:

```text
── Key that comes from workflow.dot ── node.from-dot/settings.json:11  node.from-dot
"kind" of step "build" comes from workflow.dot, not from settings.json.

 11 │     "build": { "action": "build", "kind": "activity" }
    │                                   ^^^^^^

To fix: remove it; workflow.dot gives a step its kind, and its arrows give it its outcomes.
```

The fix changes:

```text
node.from-dot/settings.json:
 11 │-    "build": { "action": "build", "kind": "activity" }
 11 │+    "build": { "action": "build" }
```

The example folder is [`catalogue/examples/node.from-dot`](../../crates/runspore-build/catalogue/examples/node.from-dot).
