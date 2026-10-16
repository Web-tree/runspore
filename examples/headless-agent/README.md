# headless-agent

**Superseded; kept as the record behind C04.** This was a background Claude step on
spore 0.1: `test` ran `./test.sh`, `implement` asked Claude (through `claude -p`) to fix
the code, and the loop repeated until the tests passed, then waited for an `approval`
signal. Findings and the wrapper's rules are in
[docs/research/c04-headless-agent-step.md](../../docs/research/c04-headless-agent-step.md).

spore 0.2 reads only workflow folders (`workflow.dot` and `settings.json`) and the
artifacts built from them, and refuses `workflow.json` with `workflow.folder`. The same
loop on 0.2 is [examples/coding-loop/](../coding-loop/), with its Claude settings as
node parameters instead of frontmatter;
[docs/guide/claude-step.md](../../docs/guide/claude-step.md) walks through it.

| File | Role |
| --- | --- |
| `workflow.json` | The 0.1 workflow document |
| `claude-step` | The wrapper: runs `claude -p`, resumes the session on a retry, emits canonical JSON |
| `instructions/implement.md` | The step's prose, with its Claude settings in frontmatter |
| `instructions/implement.schema.json` | The step's `--json-schema` |
| `project/` | The code under test, with a bug in `add.sh` |
