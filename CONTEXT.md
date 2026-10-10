# Runspore

A durable workflow runtime: a workflow is a graph of steps, and a run of it survives crashes and restarts.

## Language

### Workflows and runs

**Workflow**:
A versioned definition of a graph of nodes and the actions bound to them. One workflow has many runs.
_Avoid_: pipeline, flow, "current workflow" (that is a run)

**Run**:
One execution of a workflow, with its own identity and state.
_Avoid_: workflow instance, job, session

**Start key**:
The name a caller gives a run so that starting it twice gives back the same run. It names a run only inside one database: the same start key in another database starts a different run.
_Avoid_: run ID, workflow ID

**Effect key**:
The key a step hands to services outside Runspore so that a repeated try of the step is not carried out twice. Every other visit of the step, in any run and any database, gets a different one.
_Avoid_: idempotency token, request ID

**Worker**:
The process that moves runs forward: it asks the kernel for each next step, records the decision, and carries out the actions it can perform itself.
_Avoid_: engine (the library inside it), runner (that performs one kind of action), daemon

**Loop-back**:
The run going back to an earlier step along an arrow, such as failing tests sending it back to implement. Each loop-back is a new visit of that step.
_Avoid_: retry (that repeats one visit after it was lost)

**Loop limit**:
How many times in a row a step may send the run along one arrow before the run takes that step's exhausted arrow instead. Any other result from the step starts the count again.
_Avoid_: retry limit, max attempts, visit limit (the workflow-wide safety net)

### Steps and their actors

**External step**:
A step whose work is done by an actor outside the worker, such as a person, a script or a Claude session, and reported back to the run.
_Avoid_: manual step, agent step (that is one use of it)

**Actor**:
Whoever takes an external step, does its work, and reports the outcome back to the run.
_Avoid_: agent (one kind of actor), client, user

**Actor label**:
A name on an external step saying which kind of actor should take it; Runspore matches it but never interprets it.
_Avoid_: queue, assignee, role

**Async action**:
An action whose result does not come back to the worker that started it, but is reported later through the result channel. Every other action is sync: the worker performs it and waits.
_Avoid_: background step, detached step

**Result channel**:
The way an async action's result reaches its run.
_Avoid_: callback, webhook

**Holder**:
Whoever a step in progress is waiting on: the worker for a sync action, the actor or the started job for an async one.
_Avoid_: owner, assignee

**Holder handle**:
The recorded identity of an async step's holder, such as a process, a session or a pod. Its form belongs to the action; Runspore keeps it without reading it.
_Avoid_: job ID, token

**Probe**:
An action's own check of a holder handle, answering whether the holder is alive, dead, or unknown.
_Avoid_: healthcheck, ping

**Watch**:
An action's own wait on a holder handle that ends when the job behind it ends, and may deliver its result.
_Avoid_: await, poll

### Claude Code integration

**Mod**:
The Claude Code extension through which a person starts, watches and resumes runs from inside a session.
_Avoid_: module, plugin (a plugin is the package a mod ships in)

**Agent step**:
A step of a workflow whose work is done by a Claude agent, not by a command or a person. It always runs in a Claude session the integration starts for that step, never in the person's own session. Every time a run comes back to the same agent step, the step continues that conversation.

**Profile kind**:
One of `cmd`, `skill`, `agent`, `human`: the integration's words for a step. Workflow files never contain them; they hold plain actions and node kinds.
_Avoid_: action kind, node kind, step type

**Background step**:
An agent step performed by `claude -p`, which the worker starts and waits on, with nobody watching live.
_Avoid_: headless step, subprocess step

**Interactive step**:
An agent step performed in an interactive Claude session that the integration starts for it in a herdr pane, so a person can watch and type into it; the session reports back when it is done.
_Avoid_: in-session step, controlled mode, agent-driven step

**Stuck step**:
A step that cannot recover (its tries ran out, or it failed with nothing left to try) and has no failure path; the run waits at it for a person's decision.
_Avoid_: failed step, blocked step
