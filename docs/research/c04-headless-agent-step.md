# C04 — A background agent step on spore 0.1

Findings for the map ticket **C04 — Prove a headless agent step on 0.1** (Web-tree/runspore#9, map #5). Measured on 2026-10-10 with `spore` built from `main` at f217f94 (CLI 0.1, release build) and Claude Code 2.1.296 on macOS, model `haiku`. C07 renamed the headless step to **background step**; this record uses the new word.

## Answer

Yes: spore 0.1 runs a background agent step today, with no change to Runspore. The step is a plain `command` action that runs a wrapper, and the wrapper runs `claude -p`. The example workflow ([examples/headless-agent](../../examples/headless-agent/)) runs the reference loop's shape: `test` fails, `implement` fixes the code through `claude -p`, `test` passes, and the run waits for an `approval` signal. It survives a worker killed mid-step (SIGKILL and SIGINT): the next `spore run` gives the step a new try, and the new try resumes the same Claude conversation.

The wrapper cannot be skipped. Calling `claude -p --output-format json` directly fails the step: the envelope holds floats, which spore rejects. So headless steps need a **documented recipe** (the wrapper's rules below), not new Runspore functionality. The runs also found two generic gaps and confirmed one C07 decision. These are not needed for the profile to work, but they change its rules:

1. **A dead worker's command keeps running** beside the retry (§Kill and resume). The wrapper handles it for Claude steps. Whether spore should handle it for every command is a new question.
2. **Effect keys repeat across databases.** The same start key in two databases gives the same effect key, so C07 R14's session UUID (derived from the effect key alone) collides (§Session ids).
3. **0.1 has no non-retryable command failure**, so a step that fails the same way every time burns all three tries. C07's result mode (`output: "result"`, host 0.2) fixes this; the runs below show the cost.

## What was run

All runs used scratch copies of `examples/headless-agent` under `/private/tmp`, each with its own database.

| Scenario | What happened | Result |
| --- | --- | --- |
| Happy path | `test` red → `implement` (1 try, 8 turns, 9.2 s, $0.08) → `test` ok → waiting; `signal approval approved` → completed | exit 3, then exit 0 |
| `claude -p` direct, `output: "json"` | `command.malformed-output`: "stdout is not canonical-domain JSON: non-integer number" | run failed |
| `claude -p` direct, `output: "text"` | Completes; the node's output is `{"exitCode": 0, "stdout": "<the whole envelope as one string>"}`, which no `$get` can reach into | completed |
| SIGKILL the worker mid-step, no parent watch | The `claude -p` process outlived the worker and finished the edit about 14 s later. The next `spore run` expired try 1 and resumed the session in try 2 | waiting |
| As above, with the parent watch | Claude was gone within 2 s of the kill. Try 2 resumed and finished | waiting |
| SIGINT mid-step (`--grace-ms 1000`) | Try 1 `unknown` (`command.stopped`), no process left, `spore` exited 130. The next `spore run` resumed the session in try 2 | waiting |
| `timeoutMs: "4000"` | Each try ended `unknown` (`command.timeout`) with no process left; after 3 tries, `activity.attempts-exhausted` | run failed (C07 L3 would pause it instead) |
| `acceptEdits` with no allow rule for `./test.sh` | Every try was denied Bash. The first wrapper treated a denial as a failure, so all 3 tries were spent | run failed |

## What `claude -p` does

Measured directly, outside spore:

- **The envelope** (`--output-format json`) is one JSON object of about 2 KB: `type`, `subtype`, `is_error`, `result` (Claude's last text), `session_id`, `num_turns`, `duration_ms`, `usage`, `modelUsage`, `permission_denials`, `terminal_reason`, `structured_output` (with `--json-schema`), and about a dozen timing fields. `total_cost_usd` and `modelUsage.*.costUSD` are floats, so the envelope is never in spore's canonical domain (kernel §1).
- **`--session-id <uuid>`** names a new session. If a transcript with that id exists, it exits 1 with `Error: Session ID <uuid> is already in use.` on stderr and empty stdout. This happens even after the earlier process was killed. The check is whether the transcript exists, not a lock.
- **`--resume <uuid>`** continues the session under the same id (no fork) and works from any working directory: it found a session started in another directory and appended to that session's transcript. An unknown id exits 1 with `No conversation found with session ID: <uuid>`.
- **No lock.** A `--resume` while another process still runs the same session succeeds, and both append to one transcript.
- **A kill mid-turn** leaves a transcript holding the prompt (killed after 6 s). Killed within about 2 s, the transcript may not exist yet, and `--session-id` then starts fresh.
- **`--json-schema`** delivers the answer through a `StructuredOutput` tool call (3 turns for a one-shot answer) and fills `structured_output`. When Claude ends without calling it, Claude Code injects `[structured-output-enforce] You MUST call the StructuredOutput tool` and continues. Asking Claude to ignore the schema with `--tools ""` still produced structured output. **`structured_output: null` with exit 0 did not reproduce.**
- **Limits end with exit 1**, `is_error: true`, `structured_output: null`: `--max-turns 1` gives `subtype: error_max_turns`, `--max-budget-usd` gives `error_max_budget_usd`. The reason is in `errors`.
- **A permission denial is not an error**: exit 0, `subtype: success`, the denial listed only in `permission_denials` (`tool_name`, `tool_input`). Claude often works around it: a compound `./test.sh; echo "exit=$?"` was denied, then plain `./test.sh` was allowed.
- **Permissions come from the user's settings when no flag is given.** This machine's `~/.claude/settings.json` has `defaultMode: auto`, so a bare `claude -p` edited files freely. With `--permission-mode default`, Write was denied. `acceptEdits` allows edits but not `./test.sh`; `--allowedTools 'Bash(./test.sh)'` allows it.
- Running `claude -p` from a `spore` started inside a Claude Code session works; the inherited environment did not get in the way.

## The recipe

What a background step's wrapper must do on spore 0.1. [`claude-step`](../../examples/headless-agent/claude-step) is a working version (Python 3, standard library only). C07's provisional `claude-spore run` is this wrapper's successor.

| Concern | Rule |
| --- | --- |
| Floats | Never pass the envelope through. Emit a new object: integers as integers, `total_cost_usd` as integer micro-dollars (`costMicroUsd`). Inside `structured_output`, an integral float becomes an integer, and any other non-integer number fails the step with the JSON path in the message. Output schemas should declare `integer`, not `number` (input to C12). |
| 64 KiB stdout cap | Emit only what later steps need: `session`, `result`, `turns`, `durationMs`, `costMicroUsd`, `deniedTools`. Without a schema, `result` is Claude's last text, cut at 32 KiB with a marker. The full transcript stays in the session. Large structured output still hits spore's `command.output-too-large`, which is not retryable: a schema should not ask for bulk data. |
| `timeoutMs` | 1 h on the action (C07 R12). On timeout spore kills the process group, which includes Claude and its tool processes (verified: none left), and the try ends `unknown`. An `idempotent` step then gets a new try that resumes the session. |
| `cwd` | Relative to the directory `spore` was started from (the engine's `base_dir` is its working directory), not to the workflow file. The example runs `spore` from the workflow folder and sets `"cwd": "project"`. Paths in `argv` are relative to `cwd`; the example passes them to `python3` to avoid Rust's platform-specific lookup of a relative program path. C14 (delivery) replaces these paths. |
| Permissions | Always pass `--permission-mode`, defaulting to `default`, so a step never inherits the user's own mode. The frontmatter carries `permissionMode` and `allow` (one `--allowedTools` rule each) next to `model` and `tools`; these extend C07's key list (`skill`, `model`, `tools`), whose final form is C09/C10. Denials are reported in `deniedTools`, not failed. |
| Outcome | 0.1 maps an outcome only through exit codes. The schema's `outcome` field picks it; the wrapper exits 0 for `ok` and the code given by `--exit <outcome>=<code>` for others, mirrored in the action's `exitOutcomes`. C07's result mode replaces this with `{"outcome", "output"}` on stdout. |
| Session id | `uuid5(namespace, RUNSPORE_EFFECT_KEY)`, so every try of one step visit uses the same id. Try `--session-id`; on "already in use", rerun with `--resume` and a short "you were interrupted, finish the step" prompt. Checking for the transcript file is unreliable because its directory depends on the session's first working directory. But see §Session ids. |
| Worker death | Watch the parent: if the wrapper's parent PID changes, kill the wrapper's own process group. See §Kill and resume. |
| Failure | Any failure exits 1, which spore 0.1 records as retryable `command.exit`, with the wrapper's message in `stderrTail`. |

**`structured_output: null` with exit 0** is not observed. Should it happen, the wrapper treats it as a failure (exit 1, "claude finished without structured output"). That is retryable: the new try resumes the session and the CLI asks again. Under C07 it stays a retryable failure in result mode, and L3 pauses the step after three.

## Kill and resume

spore starts each command in its own process group and kills the group on timeout, on stop, and when the command ends. A SIGKILLed worker kills nothing, so the group runs on. Without a guard, the wrapper's `claude -p` finished its work after its worker was dead, and nobody recorded the result. The next worker expired try 1 after its lease (10 s by default) and started try 2, which resumes the same session. An orphan still running at that point shares the conversation with the new try (verified: no lock, both write one transcript). C07's risk "a wrong *dead* puts two sessions on one conversation" also applies to sync background steps, not only to probes.

The example wrapper polls `getppid()` every second while Claude runs and, when it changes, kills its own process group. Measured: no Claude process 2 s after the SIGKILL; try 2 resumed cleanly.

This is a generic gap, not a Claude one. Any long command keeps running after its worker dies, and a repeatable effect then runs twice at once. A spore-level fix would record the command's process group and start time with the attempt, and kill the group when the attempt expires on the same machine. Ticketed as a question (see the map).

## Session ids

Run IDs derive from the tenant and the start key alone (`ids::run_from_start_key`), and effect keys from the run and activation. Two databases that start a run with the same `--key` produce identical effect keys. In this work, a second scratch database reused the key `c04-happy`. Its `implement` step computed the same session UUID, found the first database's transcript ("already in use"), and resumed a conversation about a different directory. Keys that `spore` generates are random (`key_<hex>`), so only explicit keys collide. But explicit keys are the documented way to make `start` idempotent.

C07 R14 (session UUID from the effect key, nothing stored) is therefore unsafe as written. The same hazard reaches any external service that receives `RUNSPORE_EFFECT_KEY` as an idempotency key, which host 0.1 recommends. Candidates: a random store id created with the database and passed to commands (generic, small), or a random session UUID the integration stores. Ticketed (see the map).

## A detached `claude -p` as an async command

C03 asked whether a detached `claude -p` fits the async form. It does, with one catch:

- **Start.** The start command must launch Claude in a **new session** (`setsid`, or `start_new_session=True`). Host 0.1 kills the command's process group as soon as the command exits, so a child left in that group dies with it. Stdout must go to a file (`<runtime dir>/<session uuid>.envelope.json`), because nobody reads the pipe after the start command exits. It prints the handle and exits 0.
- **Handle** (C07 R9): `{"pid": <n>, "started": "<process start time>", "session": "<uuid>"}`. The start time (from `ps -o lstart= -p <pid>`) guards against PID reuse.
- **Probe.** No process with that PID: `dead`. Same PID and same start time: `alive`. Same PID, different start time: `dead` (the PID was reused). `ps` failing: `unknown`.
- **Watch.** Wait until the process is gone and the envelope file is complete JSON, then apply the same rules as the sync wrapper (floats, outcome, `structured_output`) and print the watch result. If the process is gone and there is no envelope, the result is a retryable failure, and the next try resumes the session.
- **Kill-safety** improves: the detached Claude belongs to no worker, so a worker's death leaves nothing to orphan, and the probe tells the next worker whether it is still running. It needs C03's async host (0.2); on 0.1 only the sync form exists.

## Inputs for other tickets

- **C07 / map.** The `--session-id` and `-p --resume` behaviour that C07 left unmeasured is now measured (above). R14 needs a store-unique part (above). The frontmatter key list gains `permissionMode` and `allow` as candidates.
- **C12 (output schemas).** `--json-schema` is enforced by the CLI. Floats in structured output are the remaining hazard: declare integers, or have the build reject `number` in a step's schema.
- **C05 (spike mod).** The `-p` half of its session-id question is answered here; the herdr-pane half remains.
- **C14 (delivery).** The wrapper reads its instructions and schema through paths relative to `cwd`, which is the 0.1 stopgap C14 replaces.
