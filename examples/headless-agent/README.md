# headless-agent

A background agent step on spore 0.1: `test` runs `./test.sh`, `implement` asks
Claude (through `claude -p`) to fix the code, and the loop repeats until the tests
pass, then waits for an `approval` signal. Findings and the wrapper's rules are in
[docs/research/c04-headless-agent-step.md](../../docs/research/c04-headless-agent-step.md).

Needs `claude` (Claude Code 2.1.296 or later) and `python3` on `PATH`. Each run spends
a few cents of model usage.

```sh
cargo build --release -p runspore-cli
export PATH="$PWD/target/release:$PATH"
demo=$(mktemp -d) && cp -R examples/headless-agent/. "$demo" && cd "$demo"
key="demo-$(date +%s)"   # a reused key resumes an old Claude session; see the findings
spore run workflow.json --key "$key"          # exits 3, waiting for approval
spore signal <runId> approval --outcome approved
spore run workflow.json --key "$key"          # exits 0
```

Run `spore` from this folder: action `cwd` values resolve against the directory spore
was started from.

| File | Role |
| --- | --- |
| `workflow.json` | The 0.1 workflow |
| `claude-step` | The wrapper: runs `claude -p`, resumes the session on a retry, emits canonical JSON |
| `instructions/implement.md` | The step's prose, with its Claude settings in frontmatter |
| `instructions/implement.schema.json` | The step's `--json-schema` |
| `project/` | The code under test, with a bug in `add.sh` |
