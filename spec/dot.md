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

`dialect` is checked before any other rule: a missing or unknown dialect is the first
error the build reports. `runspore/0.2` is the only dialect this version reads; a
folder marked `runspore/0.1` is refused with the fix (change the dialect line, and
`format` in `settings.json`).

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
| `format` | yes | `runspore.workflow/0.2`; must agree with the dialect |
| `limits` | no | The package's `limits` |
| `actions` | no | The package's `actions`, each optionally with `assets` |
| `nodes` | no | Per node ID, the node's non-routing fields |

`name`, `start`, a node's `kind` and its `outcomes` come from `workflow.dot` and are
not allowed here. Any other top-level field is an error.

The fields a node may carry depend on its kind in the DOT:

| Kind | Fields |
| --- | --- |
| `activity` | `action` (required), `input`, `instructions`, `outputSchemas`, `loopLimits` |
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

An `activity` node's `input` may hold the marker `{"$outputSchemas": {}}` wherever a
mapping is evaluated: as the input itself, or inside its plain objects and arrays,
but not inside a `$literal` or a `$get`, whose contents are data. The build replaces
each marker with `{"$literal": <the node's outputSchemas>}`, the schemas in package
form, so a program the step runs can read its own schemas without the workflow
writing them twice. A marker on a node with no `outputSchemas`, or one with another
key or a value other than `{}`, is an error.

`loopLimits` goes into the package as it is: an object from an outcome the node
routes to how many times in a row it may route it, for example `{"red": 3}` on the
step that judges a loop. The next `red` takes the node's `exhausted` edge, which the
node must draw (kernel.md §2, W12). The number is not drawn in the DOT; a renderer
may show it on the edge from the settings.

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
   each schema file as a `source` entry. Read each `outputSchemas` file, inline the
   `$ref`s and check every schema against the subset at its place in the package
   (schema.md §1); then replace each `$outputSchemas` marker, checking that the copy
   still fits the package's depth limit where it lands.
6. Produce the package: `format`, `name` and `start` from the files above, `limits`
   and `actions` from the settings, and each node as its DOT `kind`, its settings
   fields, and, for `activity` and `await-signal`, `outcomes` mapping each edge's
   `on` to its target. The package then passes the kernel's validation (kernel.md
   §2) like any other, including W12 (loop limits), W14 (every node is reachable
   from `start`) and W15 (an end is reachable from every node).
7. Seal the artifact: the canonical package bytes as `main`, `workflow.dot` and
   `settings.json` as `source` entries, and the entries of step 5, each with its
   bytes, its blob digest and `executable` when the file had an execute bit
   (artifact.md §2). The same folder always gives the same manifest and the same
   package.
8. With `--composed` only: nest the sealed artifact and the kernel component in the
   composed artifact (artifact.md §7). The artifact inside is the one step 7 sealed,
   byte for byte.

## 6. Errors

A build error is reported with exit code 2 and names the file and line:
`<folder>/workflow.dot:<line>: <message>` or `<folder>/settings.json:<line>:
<message>`. An error that concerns both files (a node in one and not the other, a
format that disagrees with the dialect) names both. An error found by the kernel's
validation after the build names the folder.

| Code | Cause |
| --- | --- |
| `workflow.folder` | The entry is neither a folder nor its `workflow.dot`, or a fixed file is missing |
| `dot.dialect` | `dialect` missing or unknown, or `format` in `settings.json` disagrees with it |
| `dot.syntax` | Outside the grammar of §2 |
| `dot.attribute` | An unknown or misplaced attribute, one given twice, or a bad value |
| `dot.graph` | A rule of §3 Names and limits or of §5 step 2; an outcome with no edge, or an edge for an outcome the action does not declare (§5 step 4) |
| `settings.invalid` | `settings.json` is not JSON or not the shape of §4, or a node names an action `actions` does not define |
| `settings.mismatch` | A node is in one file and missing from the other |
| `settings.reference` | A file reference that is absolute, escapes the folder, names no file, or names `workflow.dot` or `settings.json` |
| `settings.invalid` | Also: a `$outputSchemas` marker on a node with no `outputSchemas`, or not written as `{"$outputSchemas": {}}` |
| `workflow.invalid-schema` | A schema file that is not JSON in the canonical domain, or a schema outside the subset, too deep, too large or with a bad `$ref` ([schema.md](schema.md)). The message names `nodes.<id>.outputSchemas.<outcome>`, the schema file when there is one, and the JSON Pointer of the keyword |
