# Workflow folders and the DOT dialect `runspore/0.2`

An author writes a workflow as a **workflow folder**: the graph in `workflow.dot`, the
structured, non-topology part of the package in `settings.json`, and the files the
settings name. A **build** turns the folder into the package of
[kernel.md](kernel.md) §2 and the files it declares, sealed together in one workflow
artifact ([artifact.md](artifact.md)). `spore build` writes the artifact to disk;
`spore validate`, `start` and `run` build a folder in memory ([cli.md](cli.md)). The
folder is never read again by a run, which executes its stored package.

The build is the library `runspore-build`, shared by the CLI and the host. It reads
nothing outside the folder and needs no Graphviz installation.

## 1. The folder

| Path | Required | Holds |
| --- | --- | --- |
| `workflow.dot` | yes | Topology, node kinds, routing, the workflow name and the start node (§2) |
| `settings.json` | yes | Limits, actions, and the non-routing fields of each node (§4) |
| Any other path | no | Instruction, asset and schema files, read only when `settings.json` names them |

The file names are fixed. The entry point is the folder or its `workflow.dot`; any
other `.dot` file is refused. A file the settings do not name is ignored, so a stray
file never joins a workflow.

## 2. Grammar

The dialect is a strict subset of the Graphviz DOT language. Accepted:

- one `digraph <id> { … }`, the id a bare or quoted ID;
- node statements `a [attr=value, …];` and edge statements `a -> b [attr=value];`;
- chained edges `a -> b -> c [on="x"];`, where the attribute list applies to every
  edge of the chain;
- graph attributes as `name = value;` or `graph [name=value];`;
- `node [...]` and `edge [...]` defaults, holding render attributes only;
- IDs as Graphviz spells them: bare (`[A-Za-z_][A-Za-z0-9_]*`), numerals, and quoted
  strings with `\"` escapes, `\` line continuations and `+` concatenation;
- comments `// …`, `/* … */`, and lines starting with `#`;
- `;` and `,` separators where Graphviz allows them.

Rejected, each with its line: `strict`, undirected graphs (`graph`, `--`),
subgraphs (`subgraph` or a bare `{`), ports (`a:p`), HTML strings (`<…>`), an
attribute without a value, and anything after the closing `}`.

Keywords (`digraph`, `graph`, `node`, `edge`, `subgraph`, `strict`) are matched
without regard to case, as in Graphviz.

## 3. Attributes

### Attributes with meaning

| Attribute | On | Required | Value |
| --- | --- | --- | --- |
| `dialect` | graph | yes | `runspore/0.2` |
| `start` | graph | yes | The ID of a node in the graph |
| `kind` | node | no, default `activity` | `activity`, `await-signal`, `complete` or `fail` |
| `on` | edge | no, default `ok` | One outcome: `[a-z0-9][a-z0-9-]{0,63}`, `failed` or `exhausted` |

`dialect` is checked before any other rule: a missing or unknown dialect is the only
problem the build reports, since nothing after it can be read. `runspore/0.2` is the
only dialect this version reads; a folder marked `runspore/0.1` is refused with the fix
(change the dialect line, and `format` in `settings.json`).

`label` is render-only and never carries meaning. An edge routes one outcome; to
route two outcomes to one node, draw two edges. `on="failed"` routes a failure of the
step; it is never declared in an action's `outcomes`. `on="exhausted"` is where the run
goes when one of the node's loop limits (`loopLimits`, §4) is used up; it is never
declared either, and only a node with `loopLimits` may draw it.

A node may appear in several statements. It is created by its first mention, in a
node or an edge statement, and gets `kind` from whichever statement sets it; two
different `kind` values for one node are an error.

### Render attributes

These are accepted anywhere an attribute list is and ignored, so the file still
renders in any DOT viewer: `label`, `xlabel`, `shape`, `style`, `color`, `fillcolor`,
`fontcolor`, `fontname`, `fontsize`, `penwidth`, `peripheries`, `width`, `height`,
`fixedsize`, `arrowhead`, `arrowtail`, `dir`, `weight`, `constraint`, `tooltip`,
`rankdir`, `splines`, `nodesep`, `ranksep`, `bgcolor`. Adding a name to this list is a
compatible change.

Any other attribute name, an attribute with meaning in the wrong place (for example
`kind` on an edge, or anything but render attributes in `node [...]` and
`edge [...]`), and an attribute given twice in one list are errors with their line.

### Names and limits

The graph id is the workflow `name` and must match `[A-Za-z0-9_.-]{1,128}`; it is
required. Node IDs match `[A-Za-z0-9_-]{1,64}`. A graph has at most 256 nodes.

## 4. Settings

`settings.json` is JSON in the package's own shape minus topology:

| Field | Required | Meaning |
| --- | --- | --- |
| `$schema` | no | The address of the settings' JSON Schema, for editors; build-only |
| `format` | yes | `runspore.workflow/0.2`; must agree with the dialect |
| `limits` | no | The package's `limits` |
| `actions` | no | The package's `actions`, each optionally with `assets` |
| `nodes` | no | Per node ID, the node's non-routing fields |
| `inputSchema` | no | The package's `inputSchema`: the shape a run's input must have ([schema.md](schema.md) §4) |
| `onIntervention` | no | The package's `onIntervention`: the action spore runs each time a run needs intervention or ends failed ([host.md](host.md) §2, Telling a person) |

`name`, `start`, a node's `kind` and its `outcomes` come from `workflow.dot` and are
not allowed here. Any other top-level field is an error.

The shape of `settings.json` is the JSON Schema (2020-12) generated from the build's
own Rust types, published at
`https://raw.githubusercontent.com/Web-tree/runspore/main/spec/settings.schema.json`
([settings.schema.json](settings.schema.json)). `settings.json` may name it as
`"$schema"` so that an editor completes and checks it; the build checks every file
against the same schema whether or not it does. An action takes only the fields the
schema lists (`kind`, `effect`, `outcomes`, `retry`, `assets`, `priority`, and per kind: for
`command`, `argv`, `cwd`, `env`, `timeoutMs`, `output`, `exitOutcomes` and `async`; for
`native`, `function`; for `external` (0.2), `actor` and `async`), so a misspelt field
is a problem rather than ignored. An action whose `effect` is `pure`, `read-only` or
`idempotent` and that sets no `retry.maxAttempts` gets `"maxAttempts": 3` in the
package, so a try lost to a crash runs again on its own, as Temporal's default retry
policy retries an activity that sets none; an `unsafe` action keeps the kernel's
default of one try (kernel.md §2), and `"maxAttempts": 1` asks for one try. As in
the kernel, the same tries also cover a retryable failure, such as a command's exit
code that `exitOutcomes` does not list (kernel.md §7.3). `async` takes `timeoutMs` and
`leaseMs`, positive decimal strings, and `probe`, `watch`, `stop` and `channels`; an action's `leaseMs`
must be smaller than its `timeoutMs` (default `"3600000"`), else
`action.lease-too-long` (host.md §2, Steps outside the worker). `probe`, `watch` and
`stop` are each `{"command": {"argv", "cwd", "env", "timeoutMs"}}`, `argv` required,
or `{"component": {…}}`, and `channels` is a non-empty list of result channels
(host.md §3, Async commands).
**0.2:** `priority` is an integer from -1000 to 1000, default 0: which due step a
worker or an actor takes first, a higher priority before a lower one and, among
equals, the one due longest (host.md §2, Dispatch). It changes no result, only the
order; give the steps that finish a run's work a higher priority than those that
start new work, so that finished work does not wait behind it.
**0.2:** each action is also checked against the capability set of the `spore` that
builds it (host.md §2, The handshake): a `probe`, `watch` or `stop` in the `component`
form, which no `spore` offers yet, is `action.component-unsupported`; `channels` that
name no result channel it offers (this `spore` offers `store`) are
`action.no-result-channel`.

The fields a node may carry depend on its kind in the DOT:

| Kind | Fields |
| --- | --- |
| `activity` | `action` (required), `input`, `params`, `instructions`, `outputSchemas`, `loopLimits` |
| `await-signal` | `signal` (required), `instructions`, `outputSchemas`, `loopLimits` |
| `complete` | `output` |
| `fail` | `error` (required) |

An `activity`, `await-signal` or `fail` node therefore needs an entry in `nodes`; a
`complete` node may omit it.

`instructions` (on a node) and `assets` (a list, on an action) are relative paths to
files inside the folder. Both are build-only: the build checks them, records the
files they name in the artifact's manifest, and leaves them out of the package.

`outputSchemas` is an object keyed by outcome ([schema.md](schema.md)). Each value is
a schema written inline, or a string: a file reference to a JSON file holding one.
Either may use the authoring form, `$defs` and `$ref` (schema.md §2). The build puts
every schema into the package in its package form: read from its file, every `$ref`
inlined, checked against the subset. The package never holds a path or a `$ref`;
the manifest records each schema file as a `source` entry.

`inputSchema` is written the same way: one schema inline, or a file reference to a
JSON file holding one, either in the authoring form. The build puts it into the
package in its package form, checked against the subset at its place (the second
level) and for an object root (schema.md §4). A run started from the artifact has its
input checked against it; a folder without one accepts any input.

An `activity` node's `params` is how it sets up its action, apart from `input`, the
data the action gets: one action defined once in `settings.json` can serve many nodes,
each with its own settings. It is a mapping like `input` (kernel.md §2, §4), checked
the same way, with the same operators (`$get`, `$literal`, `$if`, `$uuid`, `$json`,
`$join`), and goes into the package as written. On any other kind of node it is
`node.field-not-allowed`, whose fix removes it.

```json
"implement": {
  "action": "coder",
  "params": {
    "tools": ["Read", "Edit", "Bash"],
    "schema": {"$resultSchema": {}},
    "instructions": "Make the change `task` asks for, then answer with a short summary."
  },
  "input": {"task": {"$get": ["input", "task"]}}
}
```

An `activity` node's `input` and `params` may hold the marker `{"$outputSchemas": {}}`
wherever a mapping is evaluated: as the field itself, inside its plain objects and
arrays, or as an operand of `$if`, `$uuid`, `$json` or `$join`, but not inside a
`$literal` or a `$get`, whose contents are data. The build replaces each marker with
`{"$literal": <the node's outputSchemas>}`, the schemas in package form, so a program
the step runs can read its own schemas without the workflow writing them twice.

`{"$resultSchema": {}}` is a marker in the same places. The build replaces it with
`{"$literal": <schema>}`, the schema of a result-mode answer for this node
([host.md](host.md) §2): an object `{"outcome", "output"}` with both required and no
other key. `outcome` is an `enum` of the outcomes the node's action declares. `output`
is the outcome's `outputSchemas` entry when the action declares one outcome, and
otherwise the `anyOf` of every outcome's entry, with `{}` (anything) for an outcome
with no entry. The top level is an object, never an `anyOf`: model APIs that take a
schema for structured output (a `--json-schema` flag, strict structured outputs)
refuse `anyOf`, `oneOf` and `allOf` at the top level. The schema does not tie an
output to its outcome; the host checks the output against its own outcome's schema
after the answer (host.md §2). A program given this schema can only answer with a
declared outcome, and the schema is one runspore itself can check answers against. For
an action with outcomes `ok` and `red` and a schema only for `ok`:

```json
{"type": "object",
 "properties": {"outcome": {"enum": ["ok", "red"]},
                "output": {"anyOf": [<ok's schema>, {}]}},
 "required": ["outcome", "output"], "additionalProperties": false}
```

Either marker on a node with no `outputSchemas` (`input.marker-no-schemas`), with
another key or a value other than `{}` (`input.marker-shape`), or whose copy would nest
deeper than the package allows (`input.marker-too-deep`) is an error.

`loopLimits` goes into the package as it is: an object from an outcome the node
routes to how many times in a row it may route it, for example `{"red": 3}` on the
step that judges a loop. The next `red` takes the node's `exhausted` edge, which the
node must draw (kernel.md §2, W12). The number is not drawn in the DOT; a renderer
may show it on the edge from the settings.

An item of a command's `argv`, and of an async `probe`'s, `watch`'s or `stop`'s
`command.argv`, is a string or a mapping that gives one before each try (host.md §3).
Its mappings are checked as a step's are (W10), except that a `$get` path starts with
`input`, `params`, `run` or `step` (`mapping.path-root` otherwise), and the markers
are not allowed. An `argv[0]` that is a mapping is not looked up on the machine.
A `$get` path into `input` or `params` that is evaluated whenever its item is (not
under `$literal`, not in a branch of `$if`, without `default`) is also checked against
each step that runs the action: when the step's own `input` or `params` surely gives
nothing at that path, the step would fail with `mapping.missing-path` on every try, so
the build reports `mapping.missing-path` naming the step and the path. An absent
`input` gives `null` and absent `params` give `{}`; a part that a `$get` or `$if` of
the step gives is known only at run time and is not checked. A missing key of the
step's `input` gets a `needs-review` fix that maps it from the run's input.

An action may also name a program in the folder for its async `probe`, `watch` or
`stop` (`async.<name>.command.argv[0]`) as `./<path>`. That file is bundled as the
action's asset, whether or not `assets` lists it. A bare name is not a file
reference; it is looked up on the machine when the program runs.

A file reference must be a relative path to a regular file that stays inside the
folder once `..` and every symbolic link are resolved. An absolute path, a backslash,
an escape, or a missing file is an error naming the reference. The manifest records
the reference with its `.` and `..` segments resolved by name (`a/./b/../c.md` is
`a/c.md`), and the bytes of the file it resolves to.

A file may be named by several owners, for example as the instructions of two nodes:
it becomes an entry of each. No reference may name `workflow.dot` or `settings.json`.

`onIntervention` is `{"action": "<id>", "params": <mapping>}`, `params` optional,
and goes into the package as written. `action` names an action of `actions`
(`on-intervention.unknown-action`, with the closest name as a `needs-review` fix) that
the worker runs itself: an `external` action, which waits for an actor, is
`on-intervention.external-action`. `params` is a mapping checked as a step's are
(W10), with `$get` paths starting with `input`, `nodes` or `run`, the run's context
without `from` (`mapping.path-root` otherwise). The action itself is an ordinary
action: steps may run it too, and its `argv` mappings read the params under `params`.

```json
"onIntervention": {
  "action": "notify",
  "params": {"issue": {"$get": ["input", "ticket", "issue"]}}
}
```

## 5. The build

1. Read `workflow.dot`; check the dialect (§3), then the grammar (§2) and the
   attributes (§3).
2. Check the graph: the name, `start`, node IDs and their count; no edge leaves a
   `complete` or `fail` node; no two edges from one node share an `on`.
3. Read `settings.json`; check its shape (§4) and that `format` agrees with the
   dialect.
4. Merge: every node in `settings.json` is in the DOT, and every node the DOT needs
   settings for has them. Each `activity` node's action exists; every outcome the
   action declares has an edge, and every edge's `on` is a declared outcome,
   `failed`, or `exhausted` when the node has `loopLimits`.
5. Check and strip the file references, and collect the files they name: each
   `instructions` file as an `instructions` entry owned by its node, each asset and
   each `./` probe, watch and stop program as an `asset` entry owned by its action,
   each schema file as a `source` entry. Read each `outputSchemas` file and the
   `inputSchema` file, inline the `$ref`s and check every schema against the subset
   at its place in the package (schema.md §1), the input schema also for its object
   root (schema.md §4); then replace each `$outputSchemas` and `$resultSchema` marker,
   checking that the copy still fits the package's depth limit where it lands.
6. Produce the package: `format`, `name` and `start` from the files above, `limits`,
   `actions` (with the default `retry.maxAttempts` of §4), `inputSchema` and `onIntervention` from the settings, and each node as its DOT `kind`, its settings
   fields, and, for `activity` and `await-signal`, `outcomes` mapping each edge's
   `on` to its target. The build checks every rule of the kernel's validation
   (kernel.md §2, W01 to W15) and the host's checks of each action itself, and words
   each problem with its own code (§6). The package then passes the kernel's
   validation like any other; a refusal there is a check the build missed (§6).
7. Seal the artifact: the canonical package bytes as `main`, `workflow.dot` and
   `settings.json` as `source` entries, and the entries of step 5, each with its
   bytes, its blob digest and `executable` when the file had an execute bit
   (artifact.md §2). The same folder always gives the same manifest and the same
   package.
8. With `--composed` only: nest the sealed artifact and the kernel component in the
   composed artifact (artifact.md §7). The artifact inside is the one step 7 sealed,
   byte for byte.

## 6. Diagnostics

The build reports **every problem at once**, as diagnostics, and builds the package
only when there is none. A diagnostic is values, not text:

- a **code**, such as `action.no-effect`;
- a **place**: a JSON Pointer into `settings.json`, or a line and column of
  `workflow.dot` (or of `settings.json`, for what no pointer names, such as a key given
  twice);
- named **arguments** of five kinds: `input` (the author's value at a pointer, quoted
  from the file), `key` (the key at a pointer), `place` (a pointer, shown as
  `"actions" → "upload"`), `computed` (a value the check worked out, such as the
  closest name) and `code` (text in the author's language, such as a DOT line);
- **fixes**: concrete edits (set the value at a pointer; remove, rename or replace the
  member at a pointer; or replace a range of text), each with an applicability:
  `safe` (cannot change what the workflow means beyond fixing the problem),
  `needs-review` (probably right; the author should look) or `needs-choice` (one of
  several alternatives). The first fix is the example spore shows.

The words come from the catalogue, `crates/runspore-build/catalogue/catalogue.toml`:
one entry per code with its severity (`problem` or `warning`), a title with no
placeholders, a message, a fix, an optional why and explanation, and templates with
`{name}` placeholders and no logic. `{action}`, `{step}`, `{file}` and `{workflow}` are
filled from the place, and `{owner}` with the step or action it is in, such as
`step "build"` or `action "coder"`, or `"onIntervention"` for a place in it. Wording a template cannot express is another code. Every code
has an example folder under `catalogue/examples/`, its expected text output, and the
folder after its first fix under `catalogue/fixed/<code>/`; `spore explain <code>`
shows them, and `docs/codes/<code>.md` is generated from them. The catalogue's tests
check that every placeholder is declared and used, that every message quotes something
of the author's (or, for an absence, says so), that each example gives exactly its code
and the codes its entry lists in `also`, that its first fix gives the fixed folder,
and that every `safe` fix leaves no new problem.

Rules:

- **No follow-ons.** A check that depends on a broken part is skipped, so one mistake
  gives one message: a missing arrow is not also an unreachable step, a misspelt key
  is not also a missing one, an arrow whose outcome is not a name is not checked
  against the action. Reachability (W14, W15) is checked only when `workflow.dot` and
  the arrows have no problem.
- **The dialect stops the build.** A missing or unknown dialect is the only problem
  reported (§3). A syntax error stops the reading of `workflow.dot`; `settings.json`
  is still checked on its own.
- **No kernel wording.** No message contains the kernel's or the host's words, a rule
  number such as `W07`, or a Rust type or field error. A package the build accepted
  and the kernel refused is `spore.missed-check`: spore says it missed a check and
  asks for a bug report.
- **Typos and wrong places.** A key in the wrong place names where it belongs (a key of
  an action under `nodes`, a key of a step on an action, `name` or `start` in the
  settings). A misspelt key, step, action, start, attribute, kind or effect gets the
  closest name as a `needs-review` fix.
- **Warnings** are likely run-time mistakes that do not stop the build: an `argv[0]`
  that holds spaces and names no existing file (`command.argv-spaces`; its split is
  `safe` when the line has no quotes, `$`, globs, backslashes or other shell
  characters, else `needs-review`), and a program not found on this machine's `PATH`
  (`command.program-not-found`), since the run may happen on another machine.
- A broken diagnostic (a pointer that does not resolve, a placeholder left empty, an
  edit that does not apply) is shown as `spore.broken-message`, never dropped.

The codes, by family; [docs/codes](../docs/codes/README.md) lists each one:

| Family | What it covers |
| --- | --- |
| `dot.*` | `workflow.dot`: the dialect, its syntax, names, attributes, the start, arrows, and the arrows against the actions (including W08, W09, W12, W14, W15) |
| `settings.*` | `settings.json`: JSON, values the package cannot hold, its shape against the schema, keys that are not steps |
| `action.*`, `command.*` | Actions (W04, W07) and command actions |
| `node.*` | Steps: the fields their kind takes, their action, signal, error, loop limits and output schemas (W11, W12, W13) |
| `mapping.*`, `input.*` | Mappings (W10), their operators, `argv` paths a step does not give, and the `$outputSchemas` and `$resultSchema` markers |
| `schema.*` | Schema files, schemas outside the subset, and an `inputSchema` whose root is not `"type": "object"` ([schema.md](schema.md)) |
| `file.*` | File references (§4) |
| `on-intervention.*` | `onIntervention`: the action it names (§4) |
| `spore.*` | spore itself: a missed check, a broken message |

`workflow.folder` is not a diagnostic: an entry that is neither a folder nor its
`workflow.dot`, or a folder without one of its two files, is refused before any check,
with exit code 2. How `spore` shows diagnostics, and its exit codes, are in
[cli.md](cli.md).
