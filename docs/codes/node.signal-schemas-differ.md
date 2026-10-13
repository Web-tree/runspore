# `node.signal-schemas-differ`: One signal, two schemas

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate node.signal-schemas-differ` reports:

```text
── One signal, two schemas ── node.signal-schemas-differ/settings.json:20  node.signal-schemas-differ
Steps "approve" and "recheck" wait for the signal "approval" with different schemas for "approved".

 20 │       "outputSchemas": { "approved": { "type": "object" } }
    │                          ^^^^^^^^^^

To fix: give both steps the same schema for "approved".
Why: a signal goes to whichever step waits for it, so its data must fit both.
```

The fix changes:

```text
node.signal-schemas-differ/settings.json:
 20 │-      "outputSchemas": { "approved": { "type": "object" } }
 20 │+      "outputSchemas": {
 21 │+        "approved": { "type": "object", "properties": { "by": { "type": "string" } } }
 22 │+      }
```

The example folder is [`catalogue/examples/node.signal-schemas-differ`](../../crates/runspore-build/catalogue/examples/node.signal-schemas-differ).
