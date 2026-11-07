# Runspore

Runspore runs workflows from your terminal and keeps their progress in SQLite. Use
its `spore` CLI to run commands, wait for an approval, hand a step to another actor,
and continue after a process stops. A workflow is a folder containing
`workflow.dot` and `settings.json`; a run records the workflow it started with.

**Start here: [Runspore documentation](https://runspore.com/docs) ·
[First workflow](docs/guide/getting-started.md) ·
[CLI reference](docs/guide/cli-reference.md)**

## Install

On macOS or Linux (or WSL on Windows), on arm64 or x86_64:

```sh
curl -fsSL https://raw.githubusercontent.com/Web-tree/runspore/main/install.sh | sh
spore --version
```

The installer puts the newest release in `~/.local/bin`, after checking the release
key's signature and the archive's checksum. Run it again to update. The
[installation guide](docs/guide/getting-started.md#install-spore) covers its options
and prerequisites.

To build from source instead, with Git, Rust managed by `rustup`, and a native C
toolchain:

```sh
git clone https://github.com/Web-tree/runspore.git
cd runspore
cargo install --path crates/runspore-cli --locked
```

Running Cargo from the checkout selects the toolchain and WebAssembly target in
`rust-toolchain.toml`. Cargo installs `spore` in `~/.cargo/bin`; add that folder to
your `PATH` if needed.

The release key pinned in `install.sh` is:

```text
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGy3mjvtp10UZgB9Y8u3KNBOh66DHPo3bdbZEszh9b1k runspore-release
```

## Try a workflow

From a checkout of this repository, the included `hello` workflow is ready to run:

```sh
spore validate examples/hello
spore run examples/hello --key first-hello
spore list
```

The run completes after two steps and returns a greeting. Repeating the command
with the same key returns the same run. Choose another key to start a new one.

To create your own workflow from an empty folder, follow
[First workflow](docs/guide/getting-started.md). It includes both source files,
input data, validation, execution, and inspection; no repository examples or agent
integration are required.

## Use Runspore

| I want to… | Read |
| --- | --- |
| Install the CLI and complete a first run | [Getting started](docs/guide/getting-started.md) |
| Write steps, route outcomes, pass data, and build an artifact | [Authoring workflows](docs/guide/authoring-workflows.md) |
| Start, inspect, continue, approve, recover, or cancel runs | [Running workflows](docs/guide/running-workflows.md) |
| Work on a step from another process or an agent | [External and asynchronous steps](docs/guide/external-steps.md) |
| Use scripts, JSON output, and exit codes | [Automation](docs/guide/automation.md) |
| Choose the database, base folder, worker settings, or upgrade path | [Projects and settings](docs/guide/project-settings.md) |
| Find a command or option | [CLI reference](docs/guide/cli-reference.md) |
| Follow runs and send notifications | [Watch a run](docs/guide/watch-a-run.md) |
| Run Claude Code as an ordinary command step | [A Claude step](docs/guide/claude-step.md) |

## Current scope

The supported runtime is the `spore` binary on one machine with one OS user and
SQLite. Several local processes can work on the same repository's runs. By
default, worktrees and clones with the same Git origin share a database under
`~/.spore/db/`; see [database selection](docs/guide/project-settings.md#where-runs-live)
before choosing an explicit database. File permissions are the access boundary.

The CLI supports command actions, external actors, asynchronous work, WebAssembly
step runners, signals, bounded loops, schema checks, and cancellation. Run
`spore capabilities` to inspect the installed binary. The software release number
and the workflow/CLI contract versions are separate: the current authoring dialect
is `runspore/0.2`, and the settings format is `runspore.workflow/0.2`.

Timers, fork/join execution, a native `workflow` node, compensation, PostgreSQL, and
a JavaScript host are not implemented in this runtime. Node and Bun conformance
checks exercise the kernel; they do not provide a JavaScript Runspore CLI or host.
The [implementation specifications](spec/README.md) describe the supported contracts.

## SWL standard

[Spore Workflow Language (SWL)](https://runspore.com/docs/swl) is one chapter of the
Runspore documentation. It is an independent working draft for portable workflow
interfaces, maintained in
[Web-tree/spore-workflow-language](https://github.com/Web-tree/spore-workflow-language).

Write executable Runspore workflows with the
[authoring guide](docs/guide/authoring-workflows.md). SWL notation is not currently
claimed to run directly in `spore`; an adapter and shared conformance evidence are
needed before making that compatibility claim.

## For contributors

The [documentation index](docs/README.md) separates user guides, implementation
contracts, and the proposed architecture. The architecture describes future
targets as well as current work; it is not a feature list for the released CLI.

- [Implementation contracts](spec/README.md), including the full [CLI contract](spec/cli.md).
- [Architecture and design records](docs/README.md#architecture-and-design).
- [Project vocabulary](CONTEXT.md).
- [Release process and verification](spec/release.md).
- [Conformance checks](conformance/js/README.md) and [contract seeds](contracts/README.md).

The existing validation commands are `cargo test --workspace --locked` and
`conformance/js/check.sh`. The runtime has not completed production qualification;
see the [evidence and qualification plan](docs/16-conformance-and-production-gates.md).

## License

MIT. See [LICENSE](LICENSE). Third-party dependencies retain their own licences.
