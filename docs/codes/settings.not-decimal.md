# `settings.not-decimal`: Milliseconds not written as digits

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.not-decimal` reports:

```text
── Milliseconds not written as digits ── settings.not-decimal/settings.json:8  settings.not-decimal
"10m" at "actions" → "build" → "timeoutMs" is not a count of milliseconds written as digits, such as "500".

  8 │       "timeoutMs": "10m"
    │                    ^^^^^

To fix: write it as a string of digits, such as "500".
```

The fix changes:

```text
settings.not-decimal/settings.json:
  8 │-      "timeoutMs": "10m"
  8 │+      "timeoutMs": "600000"
```

The example folder is [`catalogue/examples/settings.not-decimal`](../../crates/runspore-build/catalogue/examples/settings.not-decimal).
