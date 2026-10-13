# `action.bad-outcome`: Not an outcome name

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate action.bad-outcome` reports:

```text
── Not an outcome name ── action.bad-outcome/settings.json:8  action.bad-outcome
Outcome "Red" of action "build" is not an outcome name; an outcome has lowercase letters, digits and "-", and starts with a letter or digit.

  8 │       "outcomes": ["ok", "Red"]
    │                          ^^^^^

To fix: rename it with those characters only.
```

The fix changes:

```text
action.bad-outcome/settings.json:
  8 │-      "outcomes": ["ok", "Red"]
  8 │+      "outcomes": ["ok", "red"]
```

The example folder is [`catalogue/examples/action.bad-outcome`](../../crates/runspore-build/catalogue/examples/action.bad-outcome).
