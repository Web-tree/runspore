# Checkpoint C: Claude does a step

Comes after these waves:

- wave 5 (R19, P3, D1);
- wave 5b (D2), after which the build runs with plain `spore` commands;
- waves 5c to 5e (C1 to C4), after which a Claude step is a plain `claude -p` command.

## A Claude step with plain spore

A workflow step can be done by Claude Code with nothing installed but `spore` and `claude`. The step is an ordinary `command` action that runs `claude -p`. It is defined once in `settings.json`, and each node that uses it sets its own `params`: the tools, the answer's schema and the instructions. spore gives the step what it needs to behave well:

- **One conversation per node and run.** `$uuid` makes the same session ID for this run and node on every try. `step.resumed` tells the step whether this node already ran, so the first try starts the session with `--session-id` and every later one continues it with `--resume`. That covers a retry after Ctrl-C, red tests and a rejected change.
- **An answer held to a schema.** `$resultSchema` turns the node's `outputSchemas` into the schema `claude` must answer in. The action's `result` block reads the outcome and the output from `claude`'s JSON, and spore checks the output against `outputSchemas` before it records it.

The action, from `examples/coding-loop/settings.json`:

```jsonc
"claude": {
  "kind": "command",
  "argv": [
    "claude", "-p",
    { "$if": [{ "$get": ["step", "resumed"] }, "--resume", "--session-id"] },
    { "$uuid": [{ "$get": ["run", "id"] }, { "$get": ["step", "node"] }] },
    "--output-format", "json",
    "--json-schema", { "$json": { "$get": ["params", "schema"] } },
    "--tools",        { "$join": [{ "$get": ["params", "tools"] }, ","] },
    "--allowedTools", { "$join": [{ "$get": ["params", "tools"] }, ","] },
    "--append-system-prompt", { "$get": ["params", "instructions"] },
    "--permission-mode", "default"
  ],
  "output": "json",
  "result": {
    "outcome": { "$get": ["stdout", "structured_output", "outcome"] },
    "output":  { "$get": ["stdout", "structured_output", "output"] }
  },
  "effect": "idempotent", "retry": { "maxAttempts": 3 }, "timeoutMs": "3600000"
}
```

A node that uses it:

```jsonc
"implement": {
  "action": "claude",
  "outputSchemas": { "ok": { "type": "object", "required": ["summary"],
                             "properties": { "summary": { "type": "string" } } } },
  "params": {
    "tools": ["Read", "Glob", "Grep", "Edit", "Write", "Bash"],
    "schema": { "$resultSchema": {} },
    "instructions": "Make the change `task` asks for, then answer with a short summary."
  },
  "input": {
    "task":     { "$get": ["input", "task"] },
    "cameFrom": { "$get": ["from"] },
    "lastTest": { "$get": ["nodes", "test"], "default": null }
  }
}
```

The full guide, with the real output of each command, is [docs/guide/claude-step.md](https://github.com/Web-tree/runspore/blob/main/docs/guide/claude-step.md), written in C4.

## Try it

In any repository with an `npm test`, from a plain terminal:

```sh
cargo install --locked --git https://github.com/Web-tree/runspore runspore-cli   # or install.sh, once the first release exists
spore init
cp -R <runspore>/examples/coding-loop .spore/workflows/
spore run coding-loop --key try-c --input '{"task": "<a small change>"}'
```

1. It stops at `approve`. Read the change with `git diff`.
2. Reject it with a reason: `spore signal <run> approval --outcome rejected --id try-c-reject-1 --data '{"reason": "…"}'`. The `--id` makes repeating the command harmless: the same message ID is applied only once. Then run the same `spore run` command again. Claude continues the same conversation and sees your reason, and the run stops at `approve` again.
3. Approve it with `spore signal <run> approval --outcome approved --id try-c-approve-1`, then run the same `spore run` command again. The run ends at `done`.
4. Start a second run with a bigger task and its own key: `spore run coding-loop --key try-c2 --input '{"task": "<a bigger change>"}'`. Press Ctrl-C while `implement` is working, a few seconds in or later, then run the same command again. The step resumes the same conversation. A try stopped within its first second has not opened its conversation yet, so it cannot be resumed (#124 lists this as a known limit).

## Feedback asked for

Is a plain `claude -p` step good enough to build the rest of the plugin on? Is anything in the action or the node settings harder to write than it should be?

## Answer

From the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-c --outcome go` starts the next wave, which closes this issue.
- `spore signal <run> checkpoint-c --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
