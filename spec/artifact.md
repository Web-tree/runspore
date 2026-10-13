# Workflow artifacts `runspore.artifact/0.1`

A build ([dot.md](dot.md) §5) turns a workflow folder into one immutable file, the
**workflow artifact**: a WebAssembly component that holds the package and every file
the workflow declares. A run is started from an artifact and pins it; the folder is
never read again. `spore build` writes the artifact to disk, and `start` and `run`
from a folder build it in memory and store it ([cli.md](cli.md)).

The artifact is data. The shared kernel component runs the package inside it; nothing
in the artifact is a kernel and nothing in it runs a step.

## 1. The `runspore:artifact` world

`contracts/wit/artifact/artifact.wit`:

```wit
package runspore:artifact@0.1.0;

interface reader {
  /// Canonical JSON manifest, format "runspore.artifact/0.1".
  manifest: func() -> list<u8>;
  /// Bytes of the blob with this digest: the package (the manifest's `main`) or the
  /// file of a manifest entry. None when the artifact holds no such blob.
  blob: func(digest: string) -> option<list<u8>>;
}

world artifact {
  export reader;
}
```

The world imports nothing. An artifact exports exactly `runspore:artifact/reader@0.1.0`.

## 2. The manifest

```json
{
  "format": "runspore.artifact/0.1",
  "dialect": "runspore/0.2",
  "name": "deploy",
  "main": "sha256:<package digest>",
  "entries": [
    {"path": "assets/check.sh", "digest": "sha256:…", "kind": "asset",
     "owner": {"action": "build"}, "executable": true},
    {"path": "instructions/review.md", "digest": "sha256:…", "kind": "instructions",
     "owner": {"node": "review"}},
    {"path": "schemas/review.json", "digest": "sha256:…", "kind": "source"},
    {"path": "settings.json", "digest": "sha256:…", "kind": "source"},
    {"path": "workflow.dot", "digest": "sha256:…", "kind": "source"}
  ]
}
```

| Field | Value |
| --- | --- |
| `format` | `runspore.artifact/0.1` |
| `dialect` | The DOT dialect the folder was written in: `runspore/0.2` |
| `name` | The workflow name, the package's `name` |
| `main` | `digest::package` of the canonical package bytes (kernel.md §1) |
| `entries` | One per file and owner, sorted by `path`, then owner (none, then nodes, then actions, each by ID) |

An entry:

| Field | Value |
| --- | --- |
| `path` | Folder-relative path of the file, with `/` separators (§3) |
| `digest` | `digest::hash("blob", [file bytes])` |
| `kind` | `source`, `instructions` or `asset` |
| `owner` | `{"node": "<node ID>"}` on `instructions`, `{"action": "<action ID>"}` on `asset`; absent on `source` |
| `executable` | `true` when the source file had any execute bit; absent otherwise |

The kinds:

- `source`: `workflow.dot`, `settings.json`, and each schema file the settings name.
  They make a run's pinned sources complete, so `spore inspect --run` can show what a
  run was started from.
- `instructions`: the file a node's `instructions` names. Its owner is that node.
- `asset`: a file an action's `assets` lists, and a probe, watch or stop program the
  action names as `./<path>` (dot.md §4). Its owner is that action.

A file that several owners name, for example one instructions file shared by two
nodes, is an entry of each, with the same path and digest; its bytes are stored once.
The manifest bytes are canonical JSON (kernel.md §1), so the same folder always gives
the same manifest.

## 3. Entry paths

An entry path is valid when it is non-empty and relative, uses `/` as its only
separator, and contains no backslash, NUL, empty segment, `.` or `..` segment. A path
appears at most once per owner, and every entry at one path has the same digest. A
manifest that breaks any of these rules is `artifact.invalid-path`.

The build only writes valid paths. A host still checks every path of an artifact it
loads (§5), since a prebuilt artifact may come from anywhere, and checks again, before
it writes an entry to disk, that the resolved file stays inside the folder it is
writing.

## 4. Digests

| Digest | Of | Role |
| --- | --- | --- |
| Package digest | The canonical package bytes | The run's identity, as in 0.1. The kernel sees only this |
| Artifact digest | `digest::hash("artifact", [artifact file bytes])` | Provenance: the file the run was started from |
| Blob digest | `digest::hash("blob", [file bytes])` | Addresses one entry's bytes inside the artifact |

The artifact digest depends on the component encoder, so two builds of one folder by
different `spore` versions may differ in artifact digest while the package digest is
the same. A start key that names a run started from another artifact of the same
package names the same run.

## 5. Loading an artifact

A host loads an artifact before it validates or starts from it:

1. Instantiate the component in the same sandbox as the kernel (host.md §1): no
   imports, bounded memory, a fuel watchdog, no nondeterministic Wasm features. A
   component that does not export the `reader` interface, that traps, or that runs out
   of fuel or memory is `artifact.invalid`.
2. Call `manifest()`. The bytes must be canonical JSON of the shape of §2;
   `format` must be `runspore.artifact/0.1` and `dialect` `runspore/0.2`
   (`artifact.unsupported` otherwise). Every entry path must pass §3.
3. Call `blob(main)` and check that `digest::package` of the bytes equals `main`
   (`artifact.digest-mismatch`; an absent blob is `artifact.invalid`).
4. Validate the package as any other (host.md §2) and start: the store keeps the
   package and the artifact content-addressed, and the run records both digests
   (store.md §2 `create_run`). Every later turn reads the package by its digest.

An artifact is never trusted for anything but bytes: it runs only inside the sandbox,
and only `manifest()` and `blob()` are called.

## 6. Errors

| Code | Cause |
| --- | --- |
| `artifact.invalid` | Not a component of the `runspore:artifact` world, a trap or watchdog stop, a manifest that is not the shape of §2, or no blob for `main` |
| `artifact.unsupported` | A manifest `format` or `dialect` this version does not read |
| `artifact.invalid-path` | An entry path that breaks §3 |
| `artifact.digest-mismatch` | `blob(main)` does not hash to `main` |

Each exits 2 in the CLI.
