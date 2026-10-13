# `settings.not-allowed`: Value not allowed

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.not-allowed` reports:

```text
── Value not allowed ────────────────── settings.not-allowed/settings.json:8  settings.not-allowed
"stdout" at "actions" → "build" → "output" is not one of the values allowed there.

  8 │       "output": "stdout"
    │                 ^^^^^^^^

To fix: use one of "text" | "json" | "result" | "none".
```

The fix changes:

```text
settings.not-allowed/settings.json:
  8 │-      "output": "stdout"
  8 │+      "output": "text"
```

The example folder is [`catalogue/examples/settings.not-allowed`](../../crates/runspore-build/catalogue/examples/settings.not-allowed).
