# Projects and settings

Runspore keeps workflow source, run data, and the folder a command operates in
separate. Understanding those three locations helps when you return from another
terminal, use worktrees, or upgrade the CLI.

## Project source

Run `spore init` at any level inside a Git repository:

```sh
spore init
```

It creates `.spore/` and `.spore/workflows/` at the repository root. Outside Git, it
uses the current folder. Repeating it is safe: it does not overwrite existing
files. It does not open a database, start a worker, or create project settings.

| Path | Purpose | Commit it? |
| --- | --- | --- |
| `.spore/workflows/NAME/workflow.dot` | A workflow's graph | Yes |
| `.spore/workflows/NAME/settings.json` | That workflow's actions, mappings, and schemas | Yes |
| Files declared by the workflow | Instructions, assets, and schema sources | Yes |
| `.spore/settings.json` | Optional settings for this project's workers | Yes, if shared project settings are intended |
| Run database and its `.files/` directory | Saved runs, artifacts, and work folders | No |

The two settings files have different purposes: an individual workflow's
`settings.json` says what to run; `.spore/settings.json` sets worker behavior.

## Where runs live

The CLI chooses a database in this order:

1. `--db PATH` on the command.
2. `RUNSPORE_DB` in the environment.
3. An existing `runspore.db` inside the nearest `.spore/` project folder at or above
   the current folder. This preserves databases created at the older location.
4. `$SPORE_HOME/db/<repository key>/runspore.db`. `SPORE_HOME` defaults to
   `~/.spore`.

For a Git repository with an `origin`, the key is derived from that origin. SSH and
HTTPS URLs for the same repository produce the same key. Its clones and worktrees
on this machine therefore see the same runs and start-key namespace by default.
Without `origin`, the key uses the Git common directory; outside Git it uses the
project folder's path, or the current path when there is no project.

Inspect the database actually selected:

```sh
spore list --json
```

The `db` field is its absolute path. No database is created by `list`; a missing
database is treated as empty. If runs seem missing, compare this path with the
database used to start them before creating replacement runs.

To select a project-local database explicitly:

```sh
spore --db "$PWD/.spore/runspore.db" list
```

Use that same database selection for all commands that operate on its runs.
`--db` is not saved as a CLI preference. Relative database paths are resolved from
the current folder. A writing command using `.spore/runspore.db` maintains ignore
rules for the database, its WAL/SHM files, and its files directory; for another
custom location, keep those paths out of version control yourself.

A worker operates on the selected database. A worker in another worktree of the
same repository can drive runs started in this one. This does not share a run's
base folder or change the source snapshot it started with.

Runspore's supported setup is one machine and one OS user, using SQLite on a local
filesystem. Sharing the database across machines, mounting it on a network
filesystem, or deliberately mixing unrelated repositories is unsupported. File
permissions are the access boundary; actor labels do not grant or restrict access.

## Where commands run

When a run starts, it records a **base folder**:

- `--base-dir` when supplied;
- otherwise, the folder holding the nearest `.spore/` project folder;
- otherwise, the current folder.

For example, from the project containing the `hello` workflow:

```sh
spore run hello --key hello-in-project --base-dir "$PWD" \
  --input '{"message":"Hello"}'
```

Later drivers use that recorded absolute folder even when they run from another
subfolder or worktree. An action may further specify its own `cwd`, relative to the
base folder when it is a relative path.

Removing a worktree preserves its shared database but can remove the base folder a
run needs. Its next command then fails with `command.base-dir-missing`. Continuing
from a different directory does not retarget it. Restore the required folder or
start a new run with the intended base folder and a new key.

Programs started as steps receive `RUNSPORE_DB` pointing to the run's database.
Calling `spore start` from a step therefore reaches the same store even when the
step's current folder would otherwise select another one.

## Step files and work folders

Each run stores its workflow artifact. Declared instructions and assets are
materialized from that stored artifact, not reread from the author's folder.
`spore step show RUN_ID` reports the files and instructions for the current step.

The directory `<database path>.files/`, beside the database, holds read-only step
files and writable work folders. A work folder belongs to one visit and is shared
by that visit's retries. A command receives paths through `RUNSPORE_STEP_FILES`,
`RUNSPORE_INSTRUCTIONS` when applicable, and `RUNSPORE_WORK_DIR`.

Keep the database and its associated run files together operationally. Editing a
workflow's source is the way to change future runs; editing generated step files
does not update the stored artifact.

## Worker settings

Create `.spore/settings.json` when you need to change the defaults:

```json
{
  "worker": {
    "slots": 4,
    "diskFloorMb": 1024
  }
}
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `worker.slots` | `4` | Maximum concurrent steps in one worker; integer at least 1 |
| `worker.diskFloorMb` | `1024` | Minimum free space in MiB on the filesystem holding run files before claiming new work; integer at least 0 |

Setting the disk floor to `0` disables the free-space pause. Unknown keys and
invalid values are refused as `project.settings-invalid`.

Commands read settings from the project found by walking up from their current
folder, even when `--db` chooses another database. There are no project settings
when no `.spore/` is found. Workers in separate worktrees can therefore use the
settings present in their own checkout while sharing run data.

Advanced global flags tune leases, heartbeats, polling, shutdown grace, and async
watches. Leave their defaults for normal use; the [CLI reference](cli-reference.md#global-options)
lists them and points to the full contracts.

## Upgrade the CLI and database

Installing a newer binary does not automatically migrate a database. If it reports
`schema.needs-migrate`, stop workers using the older version and run:

```sh
spore migrate
```

Use `--db` or `RUNSPORE_DB` if the database is explicitly selected. Migration
updates that database's schema in place, in one transaction. It does not move the
database to a new location or migrate a running workflow to a new definition.

| Message | Next step |
| --- | --- |
| `schema.needs-migrate` | Stop older workers, then migrate with the intended newer binary |
| `migrate.workers-live` | Let the named older workers finish or stop them, then retry |
| `schema.too-new` | Use the software version named in the error, or a compatible newer binary |
| `schema.too-old` | Finish the 0.1 runs with the matching old binary; this is not a supported 0.1-to-0.2 migration |
| `db.not-found` | Check the selected database path; `migrate` does not create a missing database |

A worker killed without a clean shutdown can remain recorded as live for up to 30
seconds. `--migrate` can opt a writing command into the same schema upgrade;
read-only commands never migrate, even with that flag.

The full rules are in the [CLI database contract](../../spec/cli.md#databases-from-another-version-02).
