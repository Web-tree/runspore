# Runspore

A durable workflow runtime: a workflow is a graph of steps, and a run of it survives crashes and restarts.

## Language

### Workflows and runs

**Workflow**:
A versioned definition of a graph of nodes and the actions bound to them. One workflow has many runs.
_Avoid_: pipeline, flow, "current workflow" (that is a run)

**Workflow folder**:
The folder an author writes one workflow in: `workflow.dot`, `settings.json`, and the instruction, asset and schema files the settings name.
_Avoid_: project, bundle, source tree

**Package**:
The canonical document of one workflow that the kernel runs. Its digest is a run's identity.
_Avoid_: compiled workflow, "the JSON"

**Workflow artifact**:
The immutable WebAssembly file a build makes from a workflow folder: the package and every file the workflow declares, listed in its manifest. A run records the artifact it was started from and never reads the folder again.
_Avoid_: bundle, binary, "the wasm" (the kernel is a different component)

**Composed artifact**:
The workflow artifact and the kernel component nested unchanged in one WebAssembly component that exports both of their interfaces, for a host that can load only one component. `spore build --composed` writes it; `spore` itself reads only its artifact half.
_Avoid_: kernel+graph wasm, standalone binary, fat artifact, bundle

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

**Slot**:
One of the steps a worker may run at once; the project's settings say how many (`worker.slots`). A step whose try is due but finds every slot taken is **queued** until one frees; once a worker claims it, it is **started**.
_Avoid_: thread, executor, "scheduled" for a queued step (the kernel's word for both)

**Driver**:
Whoever keeps a run moving between steps by running `spore`: a session following the run, a separate process, or an outside agent that visits it. Several may act on one run at once; nothing has to run between their calls.
_Avoid_: owner, controller, scheduler

**Leftover command**:
A command still running after the worker that started it has died, so nobody will record its result. Runspore stops it at once, and before the step is tried again.
_Avoid_: orphan, zombie, stray process

**Guard**:
The small process `spore` puts in front of every command it runs for a step, outside the command's process group. It lets the command run only once the worker says go, and stops the command's group when the worker dies. It is never named in a workflow.
_Avoid_: wrapper, supervisor, shim

**Process record**:
What a try of a step keeps about the process group doing its work: the machine, its boot, the group and the start time of the command that leads it. A later worker reads it to stop a leftover command before the step is tried again. `spore` writes and reads it itself.
_Avoid_: holder handle (an async job's identity, which `spore` never reads), PID file

**Loop-back**:
The run going back to an earlier step along an arrow, such as failing tests sending it back to implement. Each loop-back is a new visit of that step.
_Avoid_: retry (that repeats one visit after it was lost)

**Arrived from**:
The arrow that brought the run into the step it is entering: the step whose result routed it there, that result's outcome and its output. A mapping reads it as `from`; at the start step it is null.
_Avoid_: previous step (the previous visit of the same step), parent

**Loop limit**:
How many times in a row a step may send the run along one arrow before the run takes that step's exhausted arrow instead. Any other result from the step starts the count again.
_Avoid_: retry limit, max attempts, visit limit (the workflow-wide safety net)

**Parent run**:
The run whose step started another run, from inside that step: a build and the ticket runs it starts. The started run is its **child run**, and records the parent's run and node. `spore list` shows children under their parent.
_Avoid_: sub-workflow, nested run, caller

**Unfinished run**:
A run that has not completed, failed or been cancelled. A repository may have several at once. What it needs is one of: working, a signal, a person for a stuck step, an actor to claim its external step, the actor holding that step or the work its async command started, or a release from quarantine. `spore continue` does the next thing it needs, or says what a person must type.
_Avoid_: open run, active run, pending run

**Step description**:
What `spore step show` prints of the step a run is at, without claiming it: its node, how it may end, the output schema of each ending, the arrow that led there, and where its files and instructions are.
_Avoid_: manifest (that is the artifact's), step info

**Cancel**:
Ending an unfinished run on purpose. The run's history is kept, and a step in progress is told to stop.
_Avoid_: abort (a crash), terminate, drop

**Input schema**:
The shape of the input a workflow accepts, declared with the workflow and checked when a run starts.
_Avoid_: parameters (node parameters belong to one node), arguments

**Project folder**:
The `.spore/` folder at a repository's root. It holds the repository's workflows, which are committed, and its run database, which is not.
_Avoid_: workspace, config dir

**Base folder**:
The folder a run's commands run from, recorded when the run starts: the folder holding the project folder, unless the person starting it names another. A run keeps it wherever it is continued from.
_Avoid_: working directory (that is one command's), project root

**Diagnostic**:
One thing a build found wrong with a workflow folder: a code, the place in the author's file, the values it quotes, and its fixes. A problem stops the build; a warning does not. It holds no text: its words come from the catalogue.
_Avoid_: error, lint, finding

**Catalogue**:
The one place every diagnostic's words are written, one entry per code with its message, its fix and an example folder that produces it.
_Avoid_: message table, translations

**Fix**:
A concrete change to the author's files that removes a diagnostic, marked safe, needs-review or needs-choice. `spore validate --fix` applies only the safe ones.
_Avoid_: suggestion, quick fix, autofix

### Steps and their actors

**Step**:
A node of a workflow that does something (runs an action, or waits for a signal), seen while a run is at it.
_Avoid_: task, stage, job

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

**Capability set**:
What a host offers: the action kinds it runs, those it runs async, and its result channels. A workflow that asks for more is refused before its run starts; a run records the result channel picked for each async action.
_Avoid_: features, feature flags

**Capability-missing**:
The state of a step whose action needs something this host lacks. The host leaves it untouched, never failed, until a host that has the capability serves the run.
_Avoid_: runner missing, unsupported step

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

**Output schema**:
The shape a step's data must have when the step ends a given way, such as "needs changes" carrying the list of changes. Runspore checks each step's data against it the moment the data arrives; a step with no output schema is not checked.
_Avoid_: contract, type

**Step files**:
The read-only copy of the files a step declares (its instructions and its action's files), made from the run's stored workflow, never from the author's folder.
_Avoid_: asset dir, bundle, workspace

**Work folder**:
A writable folder Runspore gives each visit of a step, shared by all its tries and kept with the run.
_Avoid_: runtime dir, temp dir, scratch

**Node parameters**:
The values a workflow sets on a node, in its `params`, for how the program its action runs is set up (its tools, its schema, its instructions), kept apart from the node's input and delivered with it on every try. One action can then serve many nodes, each with its own parameters. Runspore evaluates them like the input and passes them on without reading them.
_Avoid_: frontmatter, flags, options

**Result schema**:
The shape of a step's whole answer when its program names the outcome itself: one of the outcomes the step may end with, and output that fits that outcome's output schema. A workflow asks for it with `{"$resultSchema": {}}`, which the build replaces from the step's output schemas.
_Avoid_: response format, answer type

### Releases

**Release key**:
The Ed25519 key that signs every release's checksum list. Its public half is pinned in `install.sh` and the README; an installer trusts a downloaded `spore` only when the checksum list carries its signature and the archive matches the list.
_Avoid_: signing key (ambiguous with package signing), GPG key

### Claude Code integration

**Mod**:
The Claude Code extension through which a person starts, watches and resumes runs from inside a session.
_Avoid_: module, plugin (a plugin is the package a mod ships in)

**Plugin**:
The Claude Code package, `ml-workflow` in the `max-skills` marketplace, that ships the mod, the authoring interview and the helper. It lives outside this repository.
_Avoid_: extension, addon

**Helper**:
The plugin's own program, `claude-spore`, which runs Claude steps, checks workflows and hands in answers; `spore` never knows about it.
_Avoid_: wrapper, shim

**Entry menu**:
What a person sees after typing `/ml-workflow:menu` in their own Claude session: the unfinished runs and what each needs, the workflows to start, and writing a new one.
_Avoid_: dashboard, launcher

**Authoring interview**:
The conversation in the person's own Claude session in which Claude drafts a workflow from their description, corrects it with them, and writes and checks its folder. It also changes an existing workflow.
_Avoid_: generator, wizard, scaffold

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

**On-intervention action**:
The action a workflow names in `onIntervention`, which spore runs once each time a run stops for a person or ends failed, so someone is told without asking. What it returns never moves the run; its failure is only recorded.
_Avoid_: error workflow, exit handler, alert, webhook

**Repair**:
Claude fixing its own answer in the same conversation after being told exactly why it does not fit the step's output schema. It happens before Runspore sees the answer, at most twice per try.
_Avoid_: retry (a new try of the step), loop-back (a new visit)
