# `settings.from-dot`: Key that comes from workflow.dot

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate settings.from-dot` reports:

```text
── Key that comes from workflow.dot ── settings.from-dot/settings.json:13  settings.from-dot
"name" comes from workflow.dot, not from settings.json.

 13 │   "name": "release"
    │   ^^^^^^

To fix: remove it; workflow.dot gives the workflow its name and its first step.
```

The fix changes:

```text
settings.from-dot/settings.json:
 12 │-  },
 13 │-  "name": "release"
 12 │+  }
```

The example folder is [`catalogue/examples/settings.from-dot`](../../crates/runspore-build/catalogue/examples/settings.from-dot).
