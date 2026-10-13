# `action.lease-too-long`: Lease not shorter than the deadline

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.lease-too-long` reports:

```text
── Lease not shorter than the deadline ── action.lease-too-long/settings.json:8  action.lease-too-long
Action "review" asks whoever holds it to renew every "600000" ms, but the step ends after "600000" ms anyway.

  8 │       "async": { "timeoutMs": "600000", "leaseMs": "600000" }
    │                                                    ^^^^^^^^

To fix: make "leaseMs" smaller than "timeoutMs", or remove it so the deadline alone bounds the step.
Why: a lease only helps when it runs out before the deadline: it frees a step whose holder stopped renewing.
```

The fix changes:

```text
action.lease-too-long/settings.json:
  8 │-      "async": { "timeoutMs": "600000", "leaseMs": "600000" }
  8 │+      "async": { "timeoutMs": "600000" }
```

The example folder is [`catalogue/examples/action.lease-too-long`](../../crates/runspore-build/catalogue/examples/action.lease-too-long).
