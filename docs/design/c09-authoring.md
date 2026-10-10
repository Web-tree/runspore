# C09 — Authoring a workflow by interview

This is the design record of the grill held on 2026-10-11 for the map ticket **C09 — Authoring a workflow by interview** (Web-tree/runspore#14, map #5). It settles how Claude interviews a person and writes a workflow, what files it writes, how it checks them, and how the result reaches the menu.

**Plugin side.** The **authoring interview** runs in the person's own Claude session as a plugin skill. The menu's "write a new workflow" and "change a workflow" hand off to it.
- The person describes the job in a few sentences. Claude drafts the whole workflow and shows it as a short numbered list of nodes and arrows. The person corrects it in plain words until they say save.
- Claude asks only what changes the outcome and fills in the rest from the defaults C07, C12 and C13 set. The summary lists every default it filled in.
- Each agent step's instructions appear as one line in the draft, and the full text is shown on request.
- On save, Claude writes `.spore/workflows/<name>/`, runs `spore validate` and `claude-spore check`, and fixes its own errors before it shows the result. It never commits.
- The same interview changes an existing workflow. It edits in place, and only new runs get the change.

**Runspore side.** There is one generic addition: `spore validate` (kernel 0.2) rejects a node that can't be reached from `start`, and a node from which no end can be reached. It applies to every workflow, written by hand or by interview.

There is no visual.

## Terms

- **Authoring interview** — The conversation in which Claude asks a person what a workflow should do and writes its folder. It is a plugin feature; Runspore knows nothing of it. *Avoid:* generator, wizard, scaffold.
- **Workflow folder** — `.spore/workflows/<name>/`: `workflow.dot`, `settings.json`, and the files they name (C02, C08). *Avoid:* bundle; package (that is the built form).
- **Draft** — The workflow as a numbered list in the chat, before any file is written. *Avoid:* preview, plan.

## Why

The ticket asks how Claude interviews the person and writes a workflow:
- what it asks: the steps, their kinds, outcomes and loops, and the human gates;
- what files it writes: the DOT file and whatever C02 puts beside it;
- how it checks the result with `spore`;
- how the new workflow then appears in the menu (C08).

Inputs from closed tickets:
- **C02:** a workflow is a folder holding `workflow.dot` and `settings.json`, with the build errors of C02 R7.
- **C07:** the four kinds are words only. Files hold plain actions, and "the authoring interview writes" the extra JSON (C07 L1). The kind defaults are `idempotent` and 3 tries for Claude steps, plus time limits per kind. A step whose effects leave the machine "stays a question for the author". The authoring interview runs `claude-spore check`.
- **C08:** a new workflow goes to `.spore/workflows/<name>/` after `spore init`. The interview asks for the run's inputs and writes `inputSchema`. Keeping two runs apart is the author's choice.
- **C12:** every outcome of a Claude step needs a schema, with a `summary` default the interview writes.
- **C13:** `loopLimits` and the reserved `exhausted` arrow.
- **C14:** a Claude node's settings are node parameters in `input`, and instructions files are plain prose that "explains what each ending means".

The old skills-repo map decided nothing about authoring by interview; the carry-over calls C09 "new ground". Its only prior art: Claude Code's own workflow scripts are "a script an agent wrote", saved under `.claude/workflows/`.

## Locked decisions

None. No decision here is hard to reverse: the interview is plugin behaviour that can be reshaped without touching a workflow or a run. The two `validate` rules are additions to kernel 0.2 that only reject graphs which could never finish.

## Routine choices

Max's calls:
- **R1. Draft first (Q1 → A).** The person describes the job; Claude drafts the whole workflow, shows it, and the person corrects it.
  - *Rejected B, node-by-node questions:* ten small answers for the common case, and the big picture only at the end.
  - *Rejected C, starters:* with no real workflows written yet, a starter would be a guess. See Deferred.
- **R2. The interview also changes existing workflows (Q2 → A).** The menu gains "change a workflow".
  - *Rejected B, new only:* a change would then mean hand-editing `settings.json`, which is where mistakes happen.
- **R3. One line per step's instructions in the draft (Q3 → A).** Full text is shown on request.
  - *Rejected B, full text for approval:* it turns the review into a wall of reading and buries the graph.

Settled from standard practice, shown answered on the grill page:
- **R4. The interview runs in the person's own session (s1).** It is a conversation with the person, not a step of a run, so C07's rule that steps never run in the person's own session does not apply. The map's Locked bullet, "the person's own session runs the menu only", is about run steps. Authoring is a menu action.
- **R5. Ask the graph and the risks; default the rest (s2).** See "What the interview asks" below.
- **R6. Write, check, fix, then show; never commit (s3).** The folder name equals the workflow's name (the DOT graph id), which removes C08's "shown by name" ambiguity.
- **R7. Two new `validate` rules (s4).** Unreachable nodes, and nodes with no way to an end. AWS Step Functions rejects the same cases.
- **R8. Draft as a list, picture on request, write on save (s5).**
- **R9. Changing edits in place, and runs keep their version (s6).**
- **R10. The ending (s7).** Claude proposes the name. It shows the path and the defaults, and offers to start a run.

## Spec

### Where it runs

- The interview is a skill shipped in the Claude Code plugin. Its name and packaging are C10's to decide; `spore-author` is provisional.
- It is reached from the menu's "write a new workflow" and "change a workflow", and directly by the plugin command with `new` or `change <name>`. Both command forms are provisional and C10's to name.
- How the menu hands the conversation to the skill (for example `$.prompt.submit`) belongs to C05, which owns the menu's widget.
- It runs in the person's own session. Nothing in it is a step of a run.

### The flow of a new workflow

1. **Description.** Claude asks what the workflow should do, in a few sentences. Its first question is open: "What should this workflow do, from start to finish?"
2. **Draft.** Claude drafts the whole workflow and shows it as a numbered list. Each line names a node, who does it, and where each ending goes:
   ```
   Workflow: fix-and-test   (input: task, text)
   1. implement   Claude, in the background   done → test
   2. test        command: npm test            green → approve; red → implement (at most 3 times in a row, then → give-up)
   3. approve     you decide                   approved → done; rejected → implement
   4. done        end, completed
   5. give-up     end, failed
   Defaults: Claude steps 3 tries, 1 h each; your decision has no time limit.
   ```
   Under the list, each Claude step gets one line saying what its instructions will tell it (R3).
3. **Questions on the draft.** Claude asks only what the description left open and what changes the outcome (see the next section), a few at a time, in plain words.
4. **Corrections.** The person corrects the draft in plain words ("tests should go back to implement only twice", "I want to watch implement"). Claude redraws the list after each correction. On request it shows a step's full instructions, or a picture of the graph rendered with Graphviz `dot` when it is installed.
5. **Save.** When the person says save, Claude:
   1. runs `spore init` if there is no project folder (C08);
   2. writes the workflow folder (see "Files");
   3. runs `spore validate <folder>` and then `claude-spore check <folder>`;
   4. fixes every error it caused and checks again. If an error still stands after three rounds, it stops, shows the error in plain words, and asks the person;
   5. never commits, stages or pushes.
6. **Ending.** It prints the folder path and the defaults it filled in, and makes one offer: start a run now. Accepting runs the menu's own start flow (C08), which asks for the inputs. The new workflow is already in the menu, because the menu lists every folder under `.spore/workflows/`.

### What the interview asks and what it fills in

Claude asks, unless the description already answered it:

| Topic | Plain question | Written as |
|---|---|---|
| The nodes and their order | (from the description) | `workflow.dot` nodes and edges |
| Who does each step | "Is this a command, Claude, or a person?" | the C07 mapping: `command`, a Claude wrapper `command`, `await-signal` or `external` |
| Claude steps: watch or not | "Do you want to watch this step and type into it?" | interactive (async `claude-spore start`) or background (`claude-spore run`); background unless the person says otherwise |
| How each step can end, and where each ending goes | "What can come out of this step, and what happens next in each case?" | action `outcomes`, `exitOutcomes`, edges with `on` |
| Loops | "How many times in a row may it go back before a person is asked?" | `loopLimits`, an `exhausted` arrow; 3 is proposed |
| Approvals and manual work | "Who approves, and what can they answer?" | `await-signal` with the answers as edges; `external` with `actor: "person"` for manual work |
| What a person's answer carries | "Should a rejection say why?" | an output schema on that signal outcome, e.g. a `reason` string |
| The inputs a run starts with | "What does a run need to know when it starts?" | `inputSchema` with a `description` per property (C08) |
| Steps that push, deploy or send messages | "If this step is cut off halfway and run again, is that safe?" | `effect: "idempotent"` on yes, `"unsafe"` on no (C07 R11, C03) |
| Two runs at once | "Will you run more than one of these at a time in this checkout?" | on yes, it proposes a first step that makes a worktree (C08 L3) |

Claude fills in, and lists in the ending:
- the retries, time limits and effect classes of each kind (C07 R11–R13);
- the node parameters `format`, `schema`, `model` and `tools`. `skill` is written only when the person names a skill (C14);
- an output schema for every Claude outcome. The default is `{summary: string}`, extended where the outcome carries data, such as the list of changes on "needs changes" (C12 R5);
- the instructions file of each Claude step, explaining what each ending means (C14);
- `maxVisitsPerNode`, raised only when a loop limit needs it (W12: the limit plus one must be at most `maxVisitsPerNode`). Where a "keep going" path could pass the safety net, it says so (C13 risk);
- `dialect="runspore/0.2"` and `format: "runspore.workflow/0.2"`.

### Files

```
.spore/workflows/<name>/
  workflow.dot        graph id = <name>; dialect, start, kind, on; label for display
  settings.json       format, limits, inputSchema, actions, nodes (outputSchemas, loopLimits, input)
  steps/<node>.md     instructions of each Claude step, plain prose, no frontmatter
```

- `<name>` is lowercase words joined by hyphens. Claude proposes it from the description, and the person can change it. If the folder exists, Claude proposes another name; it never overwrites a workflow in "new".
- Output schemas are written inline in `settings.json`. Nothing else is written, and the interview leaves no draft or scratch files behind.

### Changing a workflow

1. Claude reads the folder and shows the workflow as the same numbered list, then asks what to change.
2. It edits the files in place and keeps everything not asked to change: DOT comments, hand-written instructions, render attributes, unrelated settings.
3. It checks and fixes as on save, then shows the new list and only the defaults it added.
4. If `spore list --unfinished` shows runs of this workflow, it says they keep the old version and only new runs get the change (C02: "updates affect new runs only").
5. A workflow folder that does not build when opened is still shown. Claude names the errors first and offers to fix them as the first change.

### `spore validate`: two new rules (kernel 0.2)

Both are package rules checked with W01–W13. A violation is `workflow.invalid` and names the node. They apply to `runspore.workflow/0.2`; 0.1 packages are unchanged.
- **W14. Every node is reachable from `start`.** Following edges from the start node reaches every node.
- **W15. Every node can reach an end.** From every node, some path of edges reaches a `complete` or `fail` node. A loop with no way out fails this rule. A loop guarded by `loopLimits` passes, because W12 already requires its `exhausted` arrow.

## Verified facts

- The menu lists "each folder under `.spore/workflows/`, shown by name, always offered", and builds nothing ahead of time (c08-entry-menu.md, Plugin side). A new folder therefore appears at once.
- C02 R7's build errors already catch: a settings entry with no node, an activity node with no settings, a declared outcome with no edge, duplicate `on`, an outgoing edge from an end node, and an escaping file reference. Unreachable nodes and missing ends were left as "candidates for the compiler" by the carry-over (W10 row), and nothing added them until now.
- `spore validate` builds in memory and reports errors with file and line (C02). `claude-spore check` checks the node parameters and requires a schema on every Claude outcome (C12, C14).
- The old skills-repo map has no authoring decisions; the carry-over (§3) lists C09 as new ground.
- Claude Code's mod API offers `$.ui.ask`, `$.command.register` and `$.prompt.submit` (docs/research/claude-code-mods.md). None of them is needed by the interview itself, which is an ordinary skill conversation.

## Changes outside C09

- **C08.** The menu gains "change a workflow" beside "write a new workflow". The workflow is shown by its name, which equals its folder name.
- **C05.** How the menu hands off to the interview skill depends on C05's menu widget.
- **C10.** The skill's name, the command forms (`new`, `change <name>`), and whether "authoring a workflow" ships as a `references/` file.
- **C11.** The reference loop can be written by this interview as its first test. The draft in the flow above is that loop.
- **Spec split.** `kernel.md` 0.2 gets W14 and W15. `spec/claude-code.md` describes the interview.

## Risks

- **An agent step's instructions may be weak, and nobody reads them before the first run (R3, Max's call).** The first run shows it, and a change affects only new runs. Mitigation: the one-line summary per step, and full text on request.
- **Editing in place can lose hand formatting.** Claude rewrites `settings.json` as JSON, so key order or spacing may change. DOT comments are kept by editing text, not regenerating it.
- **A wrong default can pass unnoticed.** The ending lists every filled-in default, so the person sees them once.
- **W14 and W15 reject a graph with a dead end that worked by accident.** They apply to 0.2 only, so existing 0.1 workflows are untouched.

## Deferred

- **Starter workflows (Q1 option C).** This reopens when a few real workflows exist and share a shape worth offering.

## Open threads

None.
