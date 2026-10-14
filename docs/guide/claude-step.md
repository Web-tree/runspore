# A Claude step with plain spore

A workflow step can be done by Claude Code with nothing installed but `spore` and
`claude`. The step is an ordinary `command` action that runs `claude -p`, defined once
in `settings.json`; each node that uses it sets its own tools, answer schema and
instructions in `params`. This page walks through the example that does it,
[`examples/coding-loop/`](../../examples/coding-loop/): two files, `workflow.dot` and
`settings.json`, no helper program and no step files.

## What the example does

```text
implement ──▶ test ──ok──▶ approve ──approved──▶ done
    ▲          │              │
    ├───red────┘              │
    ├──────────rejected───────┘
    │          │ the 4th red in a row
    │          ▼
    └─keep-going── tests_stuck ──stop──▶ stopped
```

1. `implement`: Claude makes the change the run's `task` asks for and answers with a
   short `summary`.
2. `test`: `npm test` runs. Exit 0 is `ok`, exit 1 is `red`.
3. A red test sends the run back to `implement`, with the end of the test output. After
   three reds in a row the next one goes to `tests_stuck`, where you decide whether
   Claude keeps going (with an optional hint) or the run stops.
4. Green tests stop the run at `approve`. You read the change and approve it, which
   ends the run at `done`, or reject it with a reason, which sends it back to Claude.

Claude keeps one conversation for the whole run, so every return to `implement`
continues where it left off: it sees what it tried before, plus the reason it is back.

## What you need

- `spore` 0.2 or later on `PATH`.
- `claude` (Claude Code) on `PATH`, logged in. Each try of `implement` spends model
  usage.
- A git repository whose tests run with `npm test` and fail with exit 1. Another test
  command is one line to change (below).

`implement` runs with the tools `Read`, `Glob`, `Grep`, `Edit`, `Write` and `Bash`, all
approved, so Claude can edit files and run any shell command in the repository without
asking. Use it on repositories you trust.

## Get it into a project

From the repository root:

```sh
spore init
cp -R <runspore>/examples/coding-loop .spore/workflows/
spore validate .spore/workflows/coding-loop
```

`spore init` creates `.spore/`, its `workflows/` folder and a `.gitignore` that keeps
the database out of git. `<runspore>` is a checkout of this repository. `validate`
prints `.spore/workflows/coding-loop is valid`.

To try it without touching a real project, make a small one first, then run the three
commands above in it:

```sh
mkdir try-coding-loop && cd try-coding-loop
git init
npm init -y
npm pkg set scripts.test="node --test"
git add . && git commit -m "Start"
```

`node --test` runs every `*.test.js` file and exits 1 when a test fails. The tasks on
this page fit this project as they are.

To use another test command, change the `test` action's `argv` and its `exitOutcomes`,
the exit code that means red. For `cargo test`, which exits 101 on a failed test:

```json
"argv": ["cargo", "test"], "exitOutcomes": { "101": "red" }
```

## Run it

```sh
task='{"task": "Add math.js exporting a sum(a, b) function, with a test in math.test.js."}'
spore run coding-loop --key add-sum --input "$task"
```

The key names the run: the same command later continues the same run instead of
starting a new one. Repeat it whole, input included, since `run` checks the input
before it looks the key up. While it works, `run` prints a line on stderr for each change:
each step it starts, each try that fails, each arrow the run takes. It works until the
run needs you, then prints its status. Here it stops at `approve` and exits 3:

- `run` is the run ID, `run_…`, which the commands below take;
- `status` is `waiting`;
- `position` is `approve (approve/1, from test on ok)`.

Read the change with `git status` and `git diff`. Claude's summary is the `implement`
result in `spore events <run>`; `spore status <run>` shows where the run is at any time.
To watch runs from another terminal, and to get a notification when one needs you
instead of checking, see [Watch a run](watch-a-run.md).

## Send it back

Reject the change with a reason:

```sh
spore signal <run> approval --outcome rejected --id approve/1 --data '{"reason": "Name it add, not sum."}'
spore run coding-loop --key add-sum --input "$task"
```

The `--id` is the step's activation ID from `position`. A signal ID is applied once, so
sending the same command twice (after a lost connection, say) does nothing the second
time, instead of rejecting the next round too. A rejection without a reason is refused
with exit 2 (`output.invalid`), because the node's `outputSchemas` requires one.

The second `run` continues the run: Claude resumes the same conversation, gets your
reason, changes the code, the tests run, and the run stops at `approve` again, now
`approve/2`.

## Approve it

```sh
spore signal <run> approval --outcome approved --id approve/2
spore run coding-loop --key add-sum --input "$task"
```

The run ends at `done` and `run` exits 0. Its result holds the task and Claude's last
summary.

## When the tests keep failing

After a fourth red in a row the run stops at `tests_stuck` and `run` exits 3. The last
test output is the `test` result in `spore events <run>`. Then either:

```sh
spore signal <run> tests-stuck --outcome keep-going --id tests_stuck/1 --data '{"hint": "The fixture is stale; regenerate it."}'
spore signal <run> tests-stuck --outcome stop --id tests_stuck/1
```

and run the same `spore run` command again. `keep-going` sends the run back to Claude
with your hint, which may be left out (`--data '{}'`), and the count of reds starts
again. `stop` ends the run at `stopped`, as failed (`run` exits 1); the edits stay in
the working tree.

A run enters `implement` at most 16 times in all, rejections included. When none are
left the run fails (`limit.visits-exceeded`).

## Stop it and go on

A bigger task gives you time to stop `implement` while it works. Start a second run with
its own key:

```sh
big='{"task": "Add stats.js exporting mean, median and mode of an array of numbers, each throwing on an empty array, with tests in stats.test.js. Then add cli.js, which prints all three for the numbers given as its arguments, with a test that runs it."}'
spore run coding-loop --key add-stats --input "$big"
```

Press Ctrl-C while `implement` or `test` is working, a few seconds in or later. spore
stops the step, records the try as stopped, and exits 130. Closing the terminal does the
same. Nothing is lost:

```sh
spore run coding-loop --key add-stats --input "$big"
```

goes on from the step that was stopped. A stopped `implement` resumes the same
conversation and gets the same message again; its earlier edits are still in the
working tree. A stopped `test` runs the tests again from the start. Steps that already
finished are not run again.

**Known limit.** A try stopped within about the first second has not opened its
conversation yet, and every later try's `--resume` fails with "No conversation found".
After three tries the step is stuck and `run` exits 4; the run cannot get that
conversation back, so start a new run with a new key. A try stopped after 3 seconds or
more resumes correctly. spore knows that a try happened, not how far `claude` got; the
fix, a step reporting its progress, is left to a later version
([spec/host.md](../../spec/host.md) §3).

## How the answer is held to the schema

The `implement` node declares what a good answer is, in `outputSchemas`: outcome `ok`
with an object holding a non-empty `summary`. spore holds Claude to it twice:

1. **Before Claude answers.** `"schema": {"$resultSchema": {}}` in the node's `params`
   is replaced, when the workflow is built, with the schema of a whole answer:
   `{"outcome": "ok", "output": {"summary": "…"}}`, an object whose `outcome` is one
   of the outcomes the action declares and whose `output` fits their schemas. Its top
   level is an object, because the Claude API refuses a schema whose top level is
   `anyOf`. The action passes it to `claude` as `--json-schema`, and Claude Code makes
   Claude answer in that shape, in `structured_output`.
2. **Before spore records it.** The action's `result` block reads the outcome and the
   output from `claude`'s JSON, and spore checks the output against `outputSchemas`. An
   answer that does not fit (no `summary`, or an outcome the action does not declare) is
   not recorded and is not retried, since another try would read the same kind of
   answer. The step is stuck, `run` exits 4, and `spore status <run>` shows the error
   (`output.invalid` or `command.undeclared-outcome`) with the end of `claude`'s
   output. `spore resolve <run> --retry` asks again; the conversation is resumed.

## The action, line by line

From [`settings.json`](../../examples/coding-loop/settings.json):

```jsonc
"claude": {
  "kind": "command",
  "argv": [
    "claude", "-p",
    { "$if": [{ "$get": ["step", "resumed"] }, "--resume", "--session-id"] },
    { "$uuid": [{ "$get": ["run", "id"] }, { "$get": ["step", "node"] }] },
    "--output-format", "json",
    "--json-schema", { "$json": { "$get": ["params", "schema"] } },
    "--tools", { "$join": [{ "$get": ["params", "tools"] }, ","] },
    "--allowedTools", { "$join": [{ "$get": ["params", "tools"] }, ","] },
    "--append-system-prompt", { "$get": ["params", "instructions"] },
    "--permission-mode", "default"
  ],
  "output": "json",
  "result": {
    "outcome": { "$get": ["stdout", "structured_output", "outcome"] },
    "output": { "$get": ["stdout", "structured_output", "output"] }
  },
  "effect": "idempotent", "retry": { "maxAttempts": 3 }, "timeoutMs": "3600000"
}
```

| Part | What it does |
| --- | --- |
| `$uuid` over the run ID and the node ID | The same session ID on every try and visit of this node in this run, and a different one in every other run or node |
| `$if` on `step.resumed` | `--session-id` on the first try of the first visit, which creates the conversation; `--resume` on every later try or visit, which continues it |
| `$json` of `params.schema` | The answer's schema as JSON text for `--json-schema` |
| `$join` of `params.tools` | The node's tools as a comma list, both offered (`--tools`) and approved (`--allowedTools`) |
| `params.instructions` | The node's instructions, added to Claude's system prompt |
| stdin | The node's `input` as JSON, which `claude -p` reads as the message |
| `output` and `result` | Read `claude`'s JSON and take the outcome and output from `structured_output` |
| `effect`, `retry`, `timeoutMs` | Safe to repeat, up to 3 tries, at most an hour each |

The node that uses it:

```jsonc
"implement": {
  "action": "claude",
  "outputSchemas": { "ok": { "type": "object", "required": ["summary"], … } },
  "params": {
    "tools": ["Read", "Glob", "Grep", "Edit", "Write", "Bash"],
    "schema": { "$resultSchema": {} },
    "instructions": "You are the implement step of a coding loop. …"
  },
  "input": {
    "task":     { "$get": ["input", "task"] },
    "cameFrom": { "$get": ["from"] },
    "lastTest": { "$get": ["nodes", "test"], "default": null }
  }
}
```

`input` is the message Claude gets on every visit: the task; `cameFrom`, the step whose
arrow brought the run back here and what it said (`null` on the first visit); and
`lastTest`, the newest test run. The instructions tell Claude what each part means.

## Another Claude step

Any node can use the same action with its own `params`. A read-only review step, for
example, with two outcomes:

```jsonc
"review": {
  "action": "claude_review",
  "outputSchemas": { "changes": { "type": "object", "required": ["points"], … } },
  "params": {
    "tools": ["Read", "Glob", "Grep"],
    "schema": { "$resultSchema": {} },
    "instructions": "Review the change in the working tree. Answer approved, or changes with the points to fix."
  }
}
```

An action declares its outcomes, so a step with other outcomes than `ok` needs its own
copy of the action with `"outcomes": ["approved", "changes"]`, here `claude_review`.
Each node gets its own conversation, because the node ID is part of the session ID.

## Reference

- [spec/host.md](../../spec/host.md) §3: `argv` from mappings, `step.resumed`, and the
  `result` block.
- [spec/dot.md](../../spec/dot.md) §4: a node's `params` and the `$resultSchema`
  marker.
- [docs/design/c11-reference-loop.md](../design/c11-reference-loop.md): the design of
  the loop and what happens when a session or the machine dies at each step.
- [watch-a-run.md](watch-a-run.md): watching runs with `spore worker`, `spore list`
  and `spore status`, and `onIntervention` for a notification when a run needs you.
