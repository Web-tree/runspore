# `action.no-result-channel`: No result channel this spore offers

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.no-result-channel` reports:

```text
── No result channel this spore offers ── action.no-result-channel/settings.json:8  action.no-result-channel
Action "review" reports its result through ["nats"], and this spore offers only "store".

  8 │       "async": { "channels": ["nats"] }
    │                              ^^^^^^^^

To fix: remove "channels", so the action reports through "store", or add "store" to the list.
Why: the run records the channel it picks at start and keeps it, so the step must have one this spore offers.
```

The fix changes:

```text
action.no-result-channel/settings.json:
  8 │-      "async": { "channels": ["nats"] }
  8 │+      "async": {}
```

The example folder is [`catalogue/examples/action.no-result-channel`](../../crates/runspore-build/catalogue/examples/action.no-result-channel).
