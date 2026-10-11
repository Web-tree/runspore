The tests failed four times in a row: Claude went back to fix them three times and the next run was still red. Decide what happens next.

The last test output is in the `test` result of `spore events <run>`, and Decide in the workflow menu shows it.

Answer one of the following. In a terminal, pass this step's activation ID as the signal ID (`position.activationId` in `spore status <run> --json`, for example `tests_stuck/1`), so that sending it twice is a no-op.

- `keep-going`: Claude tries again in the same conversation, with the last test output, and the count of red runs starts again from zero. Send `{}`, or advice for Claude as a hint.
  `spore signal <run> tests-stuck --outcome keep-going --data '{"hint": "…"}' --id <activationId>`
- `stop`: the run ends as failed. The edits stay in the working tree.
  `spore signal <run> tests-stuck --outcome stop --id <activationId>`

A run may visit `implement` at most 16 times in all, counting every return to it, rejections at approval included. One round of "keep going" can take up to four of them. When none are left, the run ends as failed, and nothing can catch that. Each visit of `implement` appears as `implement/<n>` in `spore events <run>`; the highest `<n>` is the count so far.
