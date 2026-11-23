# runspore

The `spore` CLI of [Runspore](https://runspore.com). It runs workflows from your terminal
and keeps their progress in SQLite. It runs commands, waits for an approval, hands a
step to another actor or an agent, and continues after a process stops.

```sh
cargo install runspore --locked
spore --version
```

`cargo install` builds `spore` with your toolchain (Rust 1.96 or newer, and a C
compiler for the bundled SQLite). To install the signed release binary instead:

```sh
curl -fsSL https://raw.githubusercontent.com/Web-tree/runspore/main/install.sh | sh
# or
brew install web-tree/tap/runspore
```

A workflow is a folder holding `workflow.dot` (the steps and the arrows between them) and
`settings.json` (what each step runs):

```sh
spore validate my-workflow
spore run my-workflow --key first
spore list
```

- [Getting started](https://github.com/Web-tree/runspore/blob/main/docs/guide/getting-started.md)
- [Authoring workflows](https://github.com/Web-tree/runspore/blob/main/docs/guide/authoring-workflows.md)
- [A Claude step](https://github.com/Web-tree/runspore/blob/main/docs/guide/claude-step.md)
- [CLI reference](https://github.com/Web-tree/runspore/blob/main/docs/guide/cli-reference.md)

The other `runspore-*` crates are its parts. They are published so this one builds, and
their Rust APIs are not a stable interface.

MIT licensed.
