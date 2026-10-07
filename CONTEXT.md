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

**Worker**:
The process that moves runs forward: it asks the kernel for each next step, records the decision, and carries out the actions it can perform itself.
_Avoid_: engine (the library inside it), runner (that performs one kind of action), daemon

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
A step of a workflow whose work is done by a Claude agent, not by a command or a person.

**Headless step**:
An agent step performed by a separate Claude process that the worker starts.
_Avoid_: subprocess step

**In-session step**:
An external step performed by the Claude session the person is working in, which reports the outcome back to the run.
_Avoid_: controlled mode, agent-driven step
