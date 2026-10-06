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

**External step**:
A step whose work is done by an actor outside the worker, such as a person, a script or a Claude session, and reported back to the run.
_Avoid_: manual step, agent step (that is one use of it)

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
