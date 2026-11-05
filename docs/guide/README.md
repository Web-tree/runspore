# Runspore user guide

Use Runspore through the `spore` CLI: write a workflow, start a run, inspect what it
needs, and keep it moving until it finishes. These pages describe the implemented
CLI and workflow dialect. You do not need to read the SWL standard or the runtime's
architecture before using it.

## Start here

1. [Getting started](getting-started.md): install `spore` and run a complete example
   from an empty folder.
2. [Authoring workflows](authoring-workflows.md): commands, input and output,
   decisions, signals, retries, loops, validation, and artifacts.
3. [Running workflows](running-workflows.md): choose a driver, inspect progress,
   send answers, recover interrupted work, and cancel.

## Common tasks

| Task | Guide |
| --- | --- |
| Let a person, script, or agent perform a step | [External and asynchronous steps](external-steps.md) |
| Drive Runspore from scripts or CI | [Automation and JSON output](automation.md) |
| Find a run, understand its database, or configure a worker | [Projects and settings](project-settings.md) |
| Look up a command, flag, or exit code | [CLI reference](cli-reference.md) |
| See progress across runs and get a notification | [Watch a run](watch-a-run.md) |
| Use Claude Code in a workflow | [A Claude step](claude-step.md) |
| Fix a workflow that does not validate | [Validation guide](../validation.md) and [diagnostic catalogue](../codes/README.md) |
| Try an existing workflow | [Example catalogue](../../examples/README.md) |

## SWL standard

The [SWL chapter](https://runspore.com/docs/swl) covers the independent Spore
Workflow Language working draft. Its [public source](https://github.com/Web-tree/spore-workflow-language)
defines portable contracts for implementations. It is a reference chapter, not an
installation or Runspore command guide. Runspore's current `workflow.dot` dialect is
implementation-specific; direct execution of SWL notation is not yet claimed.

## Which documentation applies?

These guides use the current `runspore/0.2` authoring dialect and
`runspore.cli/0.2` command contract. Check `spore --version` and
`spore capabilities` for the software you installed; a software release such as
`0.9.1` does not imply a workflow format named `runspore/0.9.1`.

For exact field definitions and behavior, use the
[implementation specifications](../../spec/README.md). The
[architecture and design documents](../README.md#architecture-and-design) also
describe future functionality. Neither those plans nor a feature in the SWL draft
means the current CLI implements it.

Runspore currently supports one machine, one OS user, and SQLite. Workflows can
execute programs with that user's access. Keep workflow source under version
control and keep run data out of it; [Projects and settings](project-settings.md)
explains where both live.
