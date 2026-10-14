# `command.result-not-json`: "result" without JSON output

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate command.result-not-json` reports:

```text
── "result" without JSON output ── command.result-not-json/settings.json:10  command.result-not-json
Action "review" maps its outcome and output from what it prints with "result", which reads that as JSON, but its "output" is not "json".

  9 │       "output": "text",
 10 │       "result": {
    │       ^^^^^^^^

To fix: set "output" to "json", or remove "result".
```

The fix changes:

```text
command.result-not-json/settings.json:
  9 │-      "output": "text",
  9 │+      "output": "json",
```

The example folder is [`catalogue/examples/command.result-not-json`](../../crates/runspore-build/catalogue/examples/command.result-not-json).
