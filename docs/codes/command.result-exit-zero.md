# `command.result-exit-zero`: Outcome for exit code 0 in result mode

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.result-exit-zero` reports:

```text
── Outcome for exit code 0 in result mode ── command.result-exit-zero/settings.json:9  command.result-exit-zero
Action "build" reads its result from what it prints when it exits with 0, so "exitOutcomes" cannot give 0 an outcome.

  9 │       "exitOutcomes": { "0": "ok" }
    │                              ^^^^

To fix: remove "0" from "exitOutcomes".
```

The fix changes:

```text
command.result-exit-zero/settings.json:
  9 │-      "exitOutcomes": { "0": "ok" }
  9 │+      "exitOutcomes": {}
```

The example folder is [`catalogue/examples/command.result-exit-zero`](../../crates/runspore-build/catalogue/examples/command.result-exit-zero).
