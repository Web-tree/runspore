The tests failed four times in a row: Claude went back to fix them three times and the next run was still red. Decide what happens next.

The last test output is in the `test` result of `spore events <run>`, and Decide in the workflow menu shows it.

Answer one of:

- `keep-going`: Claude tries again in the same conversation, and the count of red runs starts again from zero. You may add advice for Claude.
  `spore signal <run> tests-stuck --outcome keep-going --data '{"hint": "…"}'`
- `stop`: the run ends as failed. The edits stay in the working tree.
  `spore signal <run> tests-stuck --outcome stop`

Each round of "keep going" can take up to four more visits of `implement`. A run may visit a step at most 16 times. The fourth round ends the run as failed, and nothing can catch that.
