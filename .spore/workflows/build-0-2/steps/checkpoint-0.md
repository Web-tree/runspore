# Checkpoint 0: Switch the build to spore 0.2

Comes after the fast lane (R1, then R3): spore now reads a workflow folder
(`workflow.dot` + `settings.json`), so it can run this build without `to-0.1`.

**Try:** move the build onto spore from `main`, from the orchestrator folder:

1. Stop the build with Ctrl-C.
2. `work/build-0-2/bin/answer 0 go`
3. `work/build-0-2/bin/build --upgrade`

`--upgrade` checks out `origin/main`, installs its spore, and starts the run again
on it, in a database of its own: the fast lane and this checkpoint pass at once,
and wave 1 starts on the new spore. It prints the commit it came from;
`build --upgrade <that commit>` goes back.

**Feedback asked for:** Did the folders validate and start as they are? Was
anything in `bin/build` still needed only because of 0.1?

Answer from the orchestrator folder:

- `work/build-0-2/bin/answer 0 go` starts wave 1 (on spore 0.1 if you skip step 3).
- `work/build-0-2/bin/answer 0 changes "<change>" ["skills: <change>"]...` makes one ticket per change (in runspore, or in the skills repo with the `skills:` prefix), builds them, then asks you again.
