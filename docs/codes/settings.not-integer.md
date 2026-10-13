# `settings.not-integer`: Not a whole number

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.not-integer` reports:

```text
── Not a whole number ────────────────── settings.not-integer/settings.json:4  settings.not-integer
16.5 at "limits" → "maxVisitsPerNode" is not a whole number.

  4 │     "maxVisitsPerNode": 16.5
    │                         ^^^^

To fix: use a whole number, such as 17.
```

The fix changes:

```text
settings.not-integer/settings.json:
  4 │-    "maxVisitsPerNode": 16.5
  4 │+    "maxVisitsPerNode": 17
```

The example folder is [`catalogue/examples/settings.not-integer`](../../crates/runspore-build/catalogue/examples/settings.not-integer).
