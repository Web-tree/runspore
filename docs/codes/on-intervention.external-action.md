# `on-intervention.external-action`: onIntervention runs an external action

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate on-intervention.external-action` reports:

```text
── onIntervention runs an external action ── on-intervention.external-action/settings.json:17  on-intervention.external-action
"onIntervention" runs action "notify", an external action, which waits for someone to claim it.

 17 │   "onIntervention": { "action": "notify" }
    │                                 ^^^^^^^^

To fix: name an action the worker runs itself, such as a command.
Why: spore runs "onIntervention" itself when a run stops for a person, and nobody claims it there.
```

The fix changes:

```text
on-intervention.external-action/settings.json:
 10 │-      "kind": "external",
 10 │+      "kind": "command",
 11 │+      "argv": ["make", "notify"],
```

The example folder is [`catalogue/examples/on-intervention.external-action`](../../crates/runspore-build/catalogue/examples/on-intervention.external-action).
