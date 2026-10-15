# `action.component-unsupported`: Component program

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.component-unsupported` reports:

```text
── Component program ────────────────── action.component-unsupported/settings.json:9  action.component-unsupported
Action "review" names a component as its "probe", which this spore cannot run.

  9 │         "probe": { "component": { "ref": "./alive.wasm", "export": "probe" } }
    │                    ^^^^^^^^^^^

To fix: write it as a program instead, in the form {"command": {"argv": ["check-alive"]}}.
Why: spore refuses a workflow it cannot run as written before the run starts, never part way through.
```

The fix changes:

```text
action.component-unsupported/settings.json:
  9 │-        "probe": { "component": { "ref": "./alive.wasm", "export": "probe" } }
  9 │+        "probe": { "command": { "argv": ["true"] } }
```

The example folder is [`catalogue/examples/action.component-unsupported`](../../crates/runspore-build/catalogue/examples/action.component-unsupported).
