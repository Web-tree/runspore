# `on-intervention.unknown-action`: Unknown onIntervention action

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate on-intervention.unknown-action` reports:

```text
── Unknown onIntervention action ── on-intervention.unknown-action/settings.json:18  on-intervention.unknown-action
"onIntervention" runs action "notfy", which "actions" does not define.

 18 │   "onIntervention": { "action": "notfy" }
    │                                 ^^^^^^^

To fix: name an action that "actions" defines, or define action "notfy" there.
```

The fix changes:

```text
on-intervention.unknown-action/settings.json:
 18 │-  "onIntervention": { "action": "notfy" }
 18 │+  "onIntervention": { "action": "notify" }
```

The example folder is [`catalogue/examples/on-intervention.unknown-action`](../../crates/runspore-build/catalogue/examples/on-intervention.unknown-action).
