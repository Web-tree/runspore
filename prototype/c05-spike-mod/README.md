# C05 spike mod (throwaway)

The mod that measured what Claude Code 2.1.296 does where the mods docs are silent, for
[C05 — A spike mod: measure what the docs leave open](https://github.com/Web-tree/runspore/issues/10).
Findings: `docs/research/c05-spike-mod.md` on `main`.

Load it for one session with `claude --plugin-dir prototype/c05-spike-mod`, then run `/spike` for the
list of tests. Every hook writes one JSON file per event to `/tmp/c05-spike-log/` (create it first).
`spore` must be on `PATH` for the `session.start` timing. Never merged: this branch is the primary source.
