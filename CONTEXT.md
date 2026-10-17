# Runspore

A durable workflow runtime: a workflow is a graph of steps, and a run of it survives crashes and restarts.

## Language

### Workflows and runs

**Workflow**:
A versioned definition of a graph of nodes and the actions bound to them. One workflow has many runs.
_Avoid_: pipeline, flow, "current workflow" (that is a run)

**Workflow folder**:
The folder an author writes one workflow in: `workflow.dot`, `settings.json`, and the instruction, asset and schema files the settings name. Instruction files usually sit in `instructions/`; `steps/` is the name of the step-files tree.
_Avoid_: project, bundle, source tree, package

**DOT dialect**:
The part of the DOT language `spore` reads in `workflow.dot`, with its own attribute vocabulary and a version (`dialect = "runspore/0.2"`). `spore` parses it itself and never needs Graphviz.
_Avoid_: DOT format, Graphviz file, "the DOT standard"

**Settings file**:
`settings.json` in a workflow folder: everything about the workflow except its arrows, such as what each action runs, each node's input and limits, and the paths of the files the workflow declares.
_Avoid_: bindings file, config, node file (and not the project's `.spore/settings.json`, which holds worker settings)

**Package**:
The canonical JSON document of one workflow that the kernel runs: the graph and its actions in one document. A build makes it from a workflow folder and seals it inside the workflow artifact. Its digest is a run's identity.
_Avoid_: compiled workflow, "the JSON", container (the manifest and files belong to the artifact, not the package)

**Workflow artifact**:
The immutable WebAssembly file a build makes from a workflow folder: the package and every file the workflow declares, listed in its manifest. A run records the artifact it was started from and never reads the folder again. "Artifact" alone means a workflow or composed artifact, never the kernel component; its digest is provenance, the package digest is identity.
_Avoid_: bundle (in any sense), binary, "the wasm", kernel artifact

**Composed artifact**:
The workflow artifact and the kernel component nested unchanged in one WebAssembly component that exports both of their interfaces, for a host that can load only one component. `spore build --composed` writes it; `spore` itself reads only its artifact half.
_Avoid_: kernel+graph wasm, standalone binary, fat artifact, bundle, graph-embedded component, workflow.wasm

**Kernel component**:
The one WebAssembly component that holds the kernel. `spore` embeds it, and every host runs the same bytes.
_Avoid_: kernel artifact, certified artifact, kernel bundle, "the wasm"

**Step runner**:
A WebAssembly component of world `runspore:runner/runner@0.1.0` that a `component` action names as its `module`: its exports start, probe and stop the step's work, and it reaches the machine only through the host imports the action's grants allow. The build seals it into the workflow artifact, so a run keeps the runner it started with.
_Avoid_: plugin, extension, wasm helper, "the module" alone (an artifact is a component too)

**Grant**:
One thing a step runner may do, listed under the action's `grants` by kind: a program it may start, a socket it may reach, a folder it may read. The host enforces the list and never asks; a call outside it fails the step.
_Avoid_: permission, capability (that is what a host offers), allowlist

**Run**:
One execution of a workflow, with its own identity and state.
_Avoid_: workflow instance, job, session

**Run ID**:
A run's identity, `run_` and 32 random hex digits, made once when the run is created and unique across databases. Every ID the run hands out, effect keys included, is built from it.
_Avoid_: start key, execution ID

**Start key**:
The name a caller gives a run so that starting it twice gives back the same run. It names a run only inside one database: the same start key in another database starts a different run.
_Avoid_: run ID, workflow ID

**Visit**:
One time the run is at a node; a loop-back makes a new visit. The spec calls it an activation (`node/visit`), and the work one visit of an activity asks for an invocation.
_Avoid_: attempt, iteration

**Try**:
One go at a visit's work; a retry is another try of the same visit. The spec calls it an attempt. A started try is **cut off** when nothing works it any more though it still holds its lease: on its machine its command, its worker or the machine's boot is gone, or its lease has run out. Its result never comes; the next driver runs it again.
_Avoid_: visit, run; "running" or "stale" for a cut-off try

**Retry**:
Another try of the same visit after a try was lost or failed in a way that may be repeated, with the same effect key, up to the action's `retry.maxAttempts`.
_Avoid_: loop-back (a new visit), repair (inside one try)

**Effect key**:
The key a step hands to services outside Runspore so that a repeated try of the step is not carried out twice. Every other visit of the step, in any run and any database, gets a different one.
_Avoid_: idempotency token, request ID

**Effect class**:
What a repeat of an action would do, declared on every action: `pure`, `read-only`, `idempotent` (repeated on its own after a lost try) or `unsafe` (waits for a person). `read-only` here is a promise about the action; the CLI's read-only commands (`status`, `list`, `events`, `step show`) are those that create no database, and step files are read-only on disk.
_Avoid_: side-effect flag

**Worker**:
The process that moves runs forward: it asks the kernel for each next step, records the decision, and carries out the actions it can perform itself.
_Avoid_: engine (the library inside it), runner (that performs one kind of action), daemon

**Slot**:
One of the steps a worker may run at once; the project's settings say how many (`worker.slots`). A step whose try is due but finds every slot taken is **queued** until one frees; once a worker claims it, it is **started**. Queued steps are taken by their action's **priority**, highest first, then the one queued longest.
_Avoid_: thread, executor, "scheduled" for a queued step (the kernel's word for both)

**Paused**:
A worker is paused while its disk is full: free space under the project's floor (`worker.diskFloorMb`). It takes no new step, and a try that ends meanwhile is **set aside**, not counted as the step's result; a repeatable one runs again in the same attempt once space returns.
_Avoid_: quarantined (a run's fault, lifted by `release`), stuck, failed

**Driver**:
Whoever keeps a run moving between steps by running `spore`: a session following the run, a separate process, or an outside agent that visits it. Several may act on one run at once; nothing has to run between their calls.
_Avoid_: owner (the store's name for who holds an attempt's lease), controller, scheduler

**Decision**:
The kernel's answer to one event: the run's next state and the commands to carry out, which the worker records. A person's reply at a waiting node or to a stuck step is an **answer** (a signal or a resolution).
_Avoid_: "decision" for a person's reply

**Supported setup**:
One machine, one OS user, one database per checkout. Several machines on one database, two repositories sharing one database on purpose, and other OS users are not supported and not detected. Whoever can write the database file may do anything to its runs: file permissions are the only boundary.
_Avoid_: multi-user, shared database

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
How many times in a row a step may send the run along one arrow before the run takes that step's exhausted arrow instead. Any other result from the step starts the count again. Written as `loopLimits` on the node.
_Avoid_: retry limit, max attempts, visit limit (the workflow-wide safety net)

**Exhausted arrow**:
The arrow `<node> -> <target> [on="exhausted"]`: where the run goes when one of the node's loop limits is used up. `exhausted` is a reserved outcome: no action may declare it and no signal can take it.
_Avoid_: give-up edge, escape edge

**Streak**:
The kernel's count for one node of which limited outcome it took last and how many times in a row. It is kept in the run state and starts again on any other outcome and after the exhausted arrow.
_Avoid_: counter, loop variable

**Safety net**:
The workflow-wide limits, `maxVisitsPerNode` and `maxActivations`. Reaching one fails the run; no arrow can route it.
_Avoid_: loop limit

**Parent run**:
The run whose step started another run, from inside that step: a build and the ticket runs it starts. The started run is its **child run**, and records the parent's run and node. `spore list` shows children under their parent.
_Avoid_: sub-workflow, nested run, caller

**Unfinished run**:
A run that has not completed, failed or been cancelled. A repository may have several at once. What it needs is one of: working, a driver to run its cut-off try again, a signal, a run it started (that run sends the signal it waits for), a person for a stuck step, an actor to claim its external step, the actor holding that step or the work its async command started, or a release from quarantine; `spore list` shows these as `needs`: `working`, `driver`, `signal`, `child`, `stuck`, `external-ready`, `held`, `quarantined`. `spore continue` does the next thing it needs, or says what a person must type.
_Avoid_: open run, active run, pending run

**Step description**:
What `spore step show` prints of the step a run is at, without claiming it: its node, how it may end, the output schema of each ending, the arrow that led there, and where its files and instructions are.
_Avoid_: manifest (that is the artifact's), step info

**Cancel**:
Ending an unfinished run on purpose. The run's history is kept, and a step in progress is told to stop. Like Temporal's terminate, nothing is undone or compensated, and the cancel is recorded on the run, not as a kernel event.
_Avoid_: abort (a crash), terminate, drop, cancellation request (the target architecture's compensating cancel)

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

**Decision record**:
One map ticket's decisions, in `docs/design/` (C02, C03, C06–C17), with rule IDs such as C13 R2 (L locked, R routine). The architecture decisions A01–A12 are in `docs/02` and the freeze decisions M01–M12 in `spec/README.md`.
_Avoid_: ADR for a C record, "M12 in docs/02"

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
A name on an external action saying which kind of actor should take its steps; Runspore matches it but never interprets it. It is a filter, not a permission: anyone who can write the database may claim any label.
_Avoid_: queue, assignee, role

**Token**:
What `spore step claim` returns: the attempt's identity, its owner and its fence. Only its bearer can renew, complete or fail that attempt.
_Avoid_: handle (the holder's identity), ticket

**Renew**:
A holder telling the store it is still at work, which extends its lease up to the step's deadline: `spore step renew` for an actor, the store's `heartbeat` for a worker.
_Avoid_: heartbeat_attempt, ping, keep-alive

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
The recorded identity of an async step's holder, such as a process, a session or a pod. Its form belongs to the action; Runspore keeps it without reading it. It may hold a PID, but Runspore never stops anything by it; that is the process record's job.
_Avoid_: job ID, token, process record

**Probe**:
An action's own check of a holder handle, answering whether the holder is alive, dead, or unknown.
_Avoid_: healthcheck, ping

**Watch**:
An action's own wait on a holder handle that ends when the job behind it ends, and may deliver its result.
_Avoid_: await, poll, "watching" a run (that is following it)

**Result mode**:
The command output mode `"output": "result"`, in which the program names the outcome itself: its stdout is `{"outcome", "output"}`.
_Avoid_: outcome-from-stdout, JSON mode (that is `"output": "json"`)

**Output schema**:
The shape a step's data must have when the step ends a given way, such as "needs changes" carrying the list of changes. Runspore checks each step's data against it the moment the data arrives; a step with no output schema is not checked.
_Avoid_: contract, type

**Step files**:
The read-only copy of the files a step declares (its instructions and its action's files), made from the run's stored workflow, never from the author's folder, under `<database>.files/steps/`.
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

These terms belong to the integration, which is specified in the `max-skills` repository. The spec in `spec/` never uses them.

**Integration**:
Everything that lets a person use Runspore from Claude Code: the plugin, with its mod, helper and skills. `spore` knows nothing about it.
_Avoid_: profile (the map's old word), adapter, `adapter-claude`

**Session**:
A Claude Code conversation, the person's own or one the integration starts for a step. Elsewhere name the kind: an OS process session (the guard's), a herdr pane. Two programs using one database are drivers, not sessions.
_Avoid_: session for a driver or a run

**Mod**:
The Claude Code extension through which a person starts, follows and resumes runs from inside a session.
_Avoid_: module, plugin (a plugin is the package a mod ships in)

**Plugin**:
The Claude Code package, `ml-workflow` in the `max-skills` marketplace, that ships the mod, the authoring interview and the helper. It lives outside this repository.
_Avoid_: extension, addon

**Sub-skill**:
One of the plugin's skills, typed as `/ml-workflow:<name>`: `menu`, `start`, `status`, `continue`, `cancel`, `author`.
_Avoid_: subcommand (a mod command is never namespaced)

**Helper**:
The plugin's own program, `claude-spore`, which runs Claude steps, checks workflows and hands in answers; `spore` never knows about it.
_Avoid_: wrapper, shim

**Oldest supported spore**:
The lowest `spore` version the plugin works with, stated in the plugin and checked against `spore capabilities --json`.
_Avoid_: pinned version, required version

**Following a run**:
A session running `spore continue <runId> --wait` in the background and reacting each time it returns. It is the plugin's usual driver; closing the session stops it and the run pauses.
_Avoid_: watching (a watch waits on a holder handle), attaching

**Coming back**:
What a person does after their Claude session died: start a new session or resume the old one. Both find the same run state, because the run lives in `spore`'s database, not in the session.
_Avoid_: recovery, reattach

**Entry menu**:
What a person sees after typing `/ml-workflow:menu` in their own Claude session: the unfinished runs, each with what it needs and a View, the workflows to start, writing a new one and changing one.
_Avoid_: dashboard, launcher

**Live view**:
The plugin's pane showing one run: its drawing, the highlighted file lines and the run's facts (status, node, visit, try, loop count). It changes when the run moves.
_Avoid_: dashboard, monitor, graph view

**Run line**:
The one line above the prompt for each run the session follows, with a button that opens the live view.
_Avoid_: status bar, banner

**Reference loop**:
The workflow in `examples/coding-loop/`: Claude implements, the tests run, red tests go back to implement at most three times in a row before a person is asked, and a person approves. It is the check the integration and `spore` must both pass, not a sample.
_Avoid_: demo workflow, example

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

**Report command**:
The helper command an interactive step's Claude runs as its last act to hand in its answer. It checks the answer against the step's output schemas and writes the result file only when it fits.
_Avoid_: result file (what it writes), `step complete` (`spore`'s command for external steps)

**Run facts**:
What a Claude try reports about itself rather than about the work: cost, turns, duration, tools that were denied. They are kept beside the result, not in it.
_Avoid_: output, metadata
