# `action.no-effect`: Missing "effect"

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A problem: the workflow does not build, and nothing runs.

A run can stop at any moment: the machine reboots, the process is killed, the
network goes away. When the run goes on, spore has to decide what to do with a step
that may or may not have finished. The action's effect tells it:

- "pure": the step only computes; running it again gives the same result.
- "read-only": it reads the world but changes nothing.
- "idempotent": it changes the world, and running it again changes nothing more,
  such as copying a file to the same place.
- "unsafe": running it twice could do harm, such as sending an email or charging a
  card. spore never runs it again on its own; the run waits for a person
  (spore resolve).

spore does not guess, because a wrong guess either repeats something harmful or
stops a run that could have gone on.

## Example

`spore validate nightly-backup` reports:

```text
── Missing "effect" ────────────────── nightly-backup/settings.json:9  action.no-effect
Action "upload" does not say whether it is safe to run again after a crash.

  9 │     "upload": {
    │     ^^^^^^^^

To fix: add one of "pure" | "read-only" | "idempotent" | "unsafe" to action "upload". For example:

 11 │       "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"],
 12 │+      "effect": "idempotent"

Why: spore runs a step again after a crash only when its effect says that is safe.
More: spore explain action.no-effect
```

The fix changes:

```text
nightly-backup/settings.json:
 11 │-      "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"]
 11 │+      "argv": ["aws", "s3", "cp", "backup.dump", "s3://acme-backups/shop/latest.dump"],
 12 │+      "effect": "idempotent"
```

The example folder is [`catalogue/examples/nightly-backup`](../../crates/runspore-build/catalogue/examples/nightly-backup).
