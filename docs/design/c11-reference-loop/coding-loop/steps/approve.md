The tests pass. Decide whether the change is done.

Look at the change in the working tree (`git diff`) and at the implement step's summary. In Claude Code, Decide in the workflow menu shows the summary. In a terminal, it is the last `implement` result in `spore events <run>`.

Answer one of:

- `approved`: the change is done and the run completes.
  `spore signal <run> approval --outcome approved`
- `rejected`: the change goes back to Claude. Say why; Claude reads your reason.
  `spore signal <run> approval --outcome rejected --data '{"reason": "…"}'`

A rejection without a reason is refused.
