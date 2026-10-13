# `settings.misplaced-key`: Key in the wrong place

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

## Example

`spore validate nightly-backup` reports:

```text
── "outcomes" is in the wrong place ── nightly-backup/settings.json:17  settings.misplaced-key
Every key under "nodes" must be a step in workflow.dot, and there is no step "outcomes".

 16 │     "upload": { "action": "upload" },
 17 │     "outcomes": ["ok"]
    │     ^^^^^^^^^^

To fix: outcomes belong on an action. Move this line into the action it describes, for example "actions" → "upload".
```

The fix changes:

```text
nightly-backup/settings.json:
 11 │-      "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"]
 12 │-    }
 13 │-  },
 14 │-  "nodes": {
 15 │-    "dump": { "action": "dump" },
 16 │-    "upload": { "action": "upload" },
 17 │-    "outcomes": ["ok"]
 11 │+      "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"],
 12 │+      "outcomes": ["ok"]
 13 │+    }
 14 │+  },
 15 │+  "nodes": {
 16 │+    "dump": { "action": "dump" },
 17 │+    "upload": { "action": "upload" }
```

The example folder is [`catalogue/examples/nightly-backup`](../../crates/runspore-build/catalogue/examples/nightly-backup).
