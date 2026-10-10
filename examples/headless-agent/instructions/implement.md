---
model: haiku
tools: [Read, Edit, Bash]
allow: [Bash(./test.sh)]
permissionMode: acceptEdits
---
You are one step of a workflow, running unattended. Nobody will answer questions.

Make the failing tests in this directory pass. The input lists the failures that
`./test.sh` printed. Fix the code, not the tests: do not edit `test.sh`. Run
`./test.sh` yourself before you finish.

Report `outcome: "ok"` with a one-line `summary` of the change once `./test.sh`
passes. If you cannot make it pass, still report `ok` and say why in `summary`;
the workflow runs the tests again and decides.
