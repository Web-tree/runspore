# The engine stays neutral: what a step may tell spore

Design record of the review held on 2026-10-22 and 2026-11-01, after #234 ("a running step can be marked as needing a person") was built twice, in PRs #283 and #292, and then closed as not planned. It sets the test a new step-level feature has to pass before it goes into spore. **A run takes decisions through one channel, signals, and a held step reports what it sees through its action's own observers: the probe and the watch. A feature that only one kind of step needs belongs in that step's runner or plugin, or in the workflow, not in the engine.**

Max asked for the review: "I feel we completelly messed up here, as we try to add 3rd way to do the same thing, instead of having only one mechanism. let's re-thing the design of the interface we have. and analyze the best practices in the area." Then: "why don't solve it as a workflow loop? … why do we need to create something specific in the neutral workflow engine?"

## Terms

- **Decision** — Something that changes where a run goes: a signal at an `await-signal` node, or a resolution of a stuck step. It is an event in the run's history and the kernel replays it. *Avoid:* message, notification.
- **Report** — Something a held step's holder tells spore about the work: alive or dead (probe), a result (watch, `step complete`), a candidate answer (`step propose`). It ends with the attempt and never moves the run by itself. *Avoid:* signal, event.
- **Step-specific state** — State that only one kind of step sets or reads, such as "an interactive Claude waits at a prompt". *Avoid:* mark, flag.

## Why

By #234, spore already had two ways to say "a person is needed":
- an `await-signal` node, which is a decision the run waits for;
- a watch's `{"needs": "person"}` line, which is a report.

#234 added a third: a mark that the holder sets with `step needs-person`. That only Claude Code's hooks would set. Two more ideas came up in the same review:
- deliver the mark as a signal;
- turn it into a general holder status.

Both would have made the engine larger for one kind of step.

The established systems split the two directions and keep the engine generic:

| System | Into the run | Out of a running step |
|---|---|---|
| Temporal | signals and updates, in history | activity heartbeat details: the latest value, kept apart from history |
| AWS Step Functions | the task token's `SendTaskSuccess` / `SendTaskFailure` | `SendTaskHeartbeat`, liveness only |
| Kubernetes | the object's spec | status conditions of one shape, `{type, status, reason, message, lastTransitionTime}` |
| LangGraph `interrupt()`, Prefect `pause_flow_run(wait_for_input=…)`, Airflow 3.1 HITL operators | "needs a human" is a pause between nodes, answered with a typed value | — |

None of them has a "needs a person" state inside a running task. Asking a person is a pause in the workflow.

## Locked decisions

### L1. Decisions go into the run as signals; a step reports through its action's observers (Max)

**Decision.** A decision is a signal, or a resolution of a stuck step. A held step's holder reports through what its action already has:
- the probe (`alive`, `dead`, `unknown`, `stopped`);
- the watch (a `needs` line, a result);
- `step complete` and `step fail`, and the candidate commands that hand in an answer.

A report never becomes a signal.

Signals are not the channel for reports, for three reasons:
- **History.** A signal is an event in the run's history, which the kernel replays. Claude Code's hooks fire on every tool call, so reports sent as signals would fill the history with hundreds of events per step.
- **Buffering.** A signal sent while the run waits for nothing is buffered. A later `await-signal` node waiting for the same name would take it (spec/host.md, `signal`).
- **Scope.** A signal is addressed by run ID. A report carries the attempt's token, so a hook left over from one attempt cannot change the next.

### L2. A new step-level feature first has to show the workflow or the runner cannot do it (Max)

**Decision.** Before spore gets a new state on a step or an attempt, the proposal answers three questions in its issue:
1. Can the workflow express it with what exists: an outcome, an `await-signal` node, a loop?
2. Can the step's runner or plugin keep it, with the imports it is granted?
3. Would more than one kind of step use it?

If the answer to 1 or 2 is yes, it does not go into spore. If 3 is no, it goes into spore only in a general shape that any step could use.

### L3. A question that is part of the work is a loop in the workflow (Max)

**Decision.** A step that needs to ask a person ends with an outcome that carries the question, and the run waits at an `await-signal` node:

```dot
claude -> need_person [on="ask_question"]   // the step's output is the question
need_person -> claude [on="response"]       // the person answers with spore signal
```

The step the run comes back to reads the answer with `{"$get": ["from", "output"]}` (kernel.md, `from`), and the question stays in `nodes.claude.output`. `status`, `list` and `continue --wait` already show the wait. Nothing in spore is added for it.

### L4. What a step's own process is doing belongs to the step's plugin (Max)

**Decision.** An interactive Claude waiting at a permission prompt is a fact about its herdr pane, and the person answers in the pane. The ml-workflow plugin records it from Claude Code's hooks as a file for one attempt, and shows it in its menu (Max-Levitskiy/skills `docs/claude-step-runner-design.md`, decision 8). Spore does not record it. A background Claude never reaches a prompt: the action's grants decide up front what it may do.

## How the test applied

- **#234, a needs-person mark: rejected.** It fails all three questions of L2:
  1. Questions are L3's loop.
  2. The plugin can keep pane state (L4).
  3. Only one kind of step would set it.

  PRs #283 and #292 were closed. The reasons are in [the comment on #234](https://github.com/Web-tree/runspore/issues/234#issuecomment-6560786473).
- **#278, a candidate answer: accepted (#293).** It passes question 1 of L2 and fails questions 2 and 3:
  - **The workflow can't do it.** It is completion inside a running step.
  - **The runner couldn't do it either.** Its probe could not hand in a result.
  - **It isn't Claude-specific.** It is a general prepare-and-commit.

  A smaller shape was weighed in review: a probe that delivers a result, as a watch can, plus `step complete --dry-run`. That would also pass L2. It is recorded on [#278](https://github.com/Web-tree/runspore/issues/278#issuecomment-6565376636), with when to prefer it.

## Rejected

- **A needs-person mark on the attempt** (#234, PRs #283 and #292). This was a third way to say "a person is needed", serving one kind of step.
- **A signal from the holder to the run.** See L1.
- **One holder status per attempt** (`step renew --state working|waiting|stopped`, after Temporal's heartbeat details). This was a single channel out, and cleaner than the mark. But it is still a state machine that only agents in panes drive, and L3 and L4 cover both of its uses without it.

## Open threads

- A watch's `needs` line is kept only in the memory of the process running the watch, so `status` in another process does not show it (#299). Under L1 it is a report and stays one. #299 decides whether it is stored with the attempt or documented as process-local.
