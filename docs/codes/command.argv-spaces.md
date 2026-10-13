# `command.argv-spaces`: Program name with spaces

<!-- Generated from crates/runspore-build/catalogue; `cargo test -p runspore-build --test catalogue -- --ignored bless` rewrites it. -->

A warning: the workflow still builds and runs; `spore validate --strict` fails on it.

spore starts a command's program directly, the way exec does, without a shell. The
first item of "argv" is the program and each other item is one argument, passed as it
is: no splitting at spaces, no quotes, no "$VARIABLES", no globs, no pipes.

So ["pg_dump --format=custom shop"] looks for a program whose name is that whole
line, and the step fails when it starts. Give each word its own item:
["pg_dump", "--format=custom", "shop"]. spore marks this split as safe to apply with
`spore validate --fix` when the line has no quotes, "$", globs or backslashes; with
any of those, it needs a look, since a shell would have read them.

To run shell syntax on purpose, run the shell: ["sh", "-c", "pg_dump … | gzip"].

This is a warning, not a problem, because a program whose file name really has
spaces in it is allowed.

## Example

`spore validate nightly-backup` reports:

```text
── Program name with spaces (warning) ── nightly-backup/settings.json:6  command.argv-spaces
"pg_dump --format=custom --file=backup.dump shop" is read as one program name. spore does not use a shell, so the words are not split.

  6 │       "argv": ["pg_dump --format=custom --file=backup.dump shop"],
    │                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

To fix: give each word its own item.

  6 │       "argv": ["pg_dump", "--format=custom", "--file=backup.dump", "shop"],

More: spore explain command.argv-spaces
```

The fix changes:

```text
nightly-backup/settings.json:
  6 │-      "argv": ["pg_dump --format=custom --file=backup.dump shop"],
  6 │+      "argv": ["pg_dump", "--format=custom", "--file=backup.dump", "shop"],
```

The example folder is [`catalogue/examples/nightly-backup`](../../crates/runspore-build/catalogue/examples/nightly-backup).
