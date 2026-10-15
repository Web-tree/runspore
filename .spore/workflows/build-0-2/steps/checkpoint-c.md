# Checkpoint C: Claude does a step

Comes after these waves:

- wave 5 (R19, P3, D1);
- wave 5b (D2), after which the build runs with plain `spore` commands;
- waves 5c to 5e (C1 to C4), after which a Claude step is a plain `claude -p` command.

## A Claude step with plain spore

A workflow step can be done by Claude Code with nothing installed but `spore` and `claude`. The step is an ordinary `command` action that runs `claude -p`. It is defined once in `settings.json`, and each node that uses it sets its own `params`: the tools, the answer's schema and the instructions. spore gives the step what it needs to behave well:

- **One conversation per node and run.** `$uuid` makes the same session ID for this run and node on every try. `step.resumed` tells the step whether this node already ran, so the first try starts the session with `--session-id` and every later one continues it with `--resume`. That covers a retry after Ctrl-C, red tests and a rejected change.
- **An answer held to a schema.** `$resultSchema` turns the node's `outputSchemas` into the schema `claude` must answer in. The action's `result` block reads the outcome and the output from `claude`'s JSON (the rest of it, such as the cost, may be anything), and spore checks the output against `outputSchemas` before it records it. An answer that does not fit leaves the step stuck instead of being recorded.

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
    "instructions": "You are the implement step of a coding loop. Make the change the task asks for … answer with outcome `ok` and a `summary`."
  },
  "input": {
    "task":     { "$get": ["input", "task"] },
    "cameFrom": { "$get": ["from"] },
    "lastTest": { "$get": ["nodes", "test"], "default": null }
  }
}
```

The full guide, from getting the example into a project to stopping a run and going on, is [docs/guide/claude-step.md](https://github.com/Web-tree/runspore/blob/main/docs/guide/claude-step.md), written in C4.

## Try it

From a plain terminal, in a small new project whose `npm test` runs `node --test`:

```sh
cargo install --locked --git https://github.com/Web-tree/runspore runspore-cli   # or install.sh, once the first release exists
mkdir try-coding-loop && cd try-coding-loop
git init
npm init -y
npm pkg set scripts.test="node --test"
git add . && git commit -m "Start"
spore init
cp -R <runspore>/examples/coding-loop .spore/workflows/
spore validate .spore/workflows/coding-loop
spore run coding-loop --key try-c-{pass} --input '{"task": "Add math.js exporting a sum(a, b) function, with a test in math.test.js."}'
```

A start key names one run, tied to its first input. The keys here end in this pass's number ({pass}), and the number goes up each time this checkpoint is asked again, so the commands always start new runs.

1. It stops at `approve`. Read the change with `git diff`.
2. Reject it with a reason: `spore signal <run> approval --outcome rejected --id try-c-reject-{pass} --data '{"reason": "Name it add, not sum."}'`. The `--id` makes repeating the command harmless: the same message ID is applied only once. Then run the same `spore run` command again. Claude continues the same conversation and sees your reason, and the run stops at `approve` again.
3. Approve it with `spore signal <run> approval --outcome approved --id try-c-approve-{pass}`, then run the same `spore run` command again. The run ends at `done`.
4. Start a second run with a bigger task and its own key: `spore run coding-loop --key try-c-stop-{pass} --input '{"task": "Add stats.js exporting mean, median and mode of an array of numbers, each throwing on an empty array, with tests in stats.test.js. Then add cli.js, which prints all three for the numbers given as its arguments, with a test that runs it."}'`. Press Ctrl-C while `implement` is working, a few seconds in or later, then run the same command again. The step resumes the same conversation. A try stopped within its first second has not opened its conversation yet, so it cannot be resumed: the step ends stuck after three tries, and you start again with a new key (#124 lists this as a known limit).

## Feedback asked for

Is a plain `claude -p` step good enough to build the rest of the plugin on? Is anything in the action or the node settings harder to write than it should be?

## Answer

From the orchestrator folder, with the build run's ID from `spore list`:

- `spore signal <run> checkpoint-c --outcome go` starts the next wave, which closes this issue.
- `spore signal <run> checkpoint-c --outcome changes --data '{"changes": ["<change>", "skills: <change>"]}'` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
