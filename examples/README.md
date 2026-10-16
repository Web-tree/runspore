# Examples

Workflow folders for the `spore` binary. Each folder holds `workflow.dot` (the steps
and the arrows between them) and `settings.json` (what each step runs); `spore` reads
the folder, builds its artifact in memory and records the artifact with the run. The
commands below were run in this order against a release build; each output block is
what they printed. Run IDs are random, so yours differ, and `...` marks lines left
out.

## Setup

From the repository root, build the binary and work in a scratch directory with copies
of the examples. The database is `./runspore.db` there.

```sh
cargo build --release -p runspore-cli
export PATH="$PWD/target/release:$PATH"
demo=$(mktemp -d) && cp -R examples/hello examples/review-loop examples/unsafe-deploy "$demo" && cd "$demo"
```

## hello/

Two `read-only` steps of two seconds each; `shout` receives the output of `greet`. Each
step may take up to three attempts.

```sh
spore validate hello
spore run hello --key hello-1
```

```text
hello is valid
2026-10-16T04:32:20Z  hello-1  step greet started, attempt 1
2026-10-16T04:32:22Z  hello-1  greet returned ok -> shout
2026-10-16T04:32:22Z  hello-1  step shout started, attempt 1
2026-10-16T04:32:25Z  hello-1  shout returned ok -> done
2026-10-16T04:32:25Z  hello-1  completed at done
run        run_cc275ce70f71ce5c2325b8b74b91f77e
start key  hello-1
status     completed
revision   3
package    sha256:73dcd360fa0dd84ab10b63262bb15267cab451f627f397dff1b31536ea71511a
artifact   sha256:5eac69a8d6d15f5634a5cf42510a25f64e42f43089288f6b83cc1bad86b8f057
end        done, from shout on ok
result     {"output":{"message":"hello"}}
```

The package digest is the run's workflow; the artifact digest is the file it came from.

### Interrupt and resume

On SIGINT (Ctrl-C), SIGTERM or SIGHUP the process stops claiming steps, gives the
steps in flight a short grace period, stops the commands still running, and exits.

```sh
spore run hello --key hello-2 & sleep 3; kill -INT $!; wait $!
spore run hello --key hello-2
spore events run_0422d62dc8f51dcf726f870079ad6404
```

```text
2026-10-16T04:32:43Z  hello-2  step greet started, attempt 1
2026-10-16T04:32:45Z  hello-2  greet returned ok -> shout
2026-10-16T04:32:45Z  hello-2  step shout started, attempt 1
run        run_0422d62dc8f51dcf726f870079ad6404
start key  hello-2
status     running
...
position   shout (shout/1, from greet on ok)
invocation inv_5700be57c1cc1ae94dad95f15118899a attempt 1 of 3 scheduled
...
2026-10-16T04:33:00Z  hello-2  shout returned ok -> done
2026-10-16T04:33:00Z  hello-2  completed at done
run        run_0422d62dc8f51dcf726f870079ad6404
...
result     {"output":{"message":"hello"}}
   1  2026-10-16T04:32:43Z  run.started            rev 1      greet  start
   2  2026-10-16T04:32:45Z  activity.result        rev 2      greet  ok -> shout: {"greeting":"hello"}  att:inv_e7ef7b1ffab7ea31b1b2df8d1664da66.1
   3  2026-10-16T04:33:00Z  activity.result        rev 3      shout  ok -> done: {"message":"hello"}; completed  att:inv_5700be57c1cc1ae94dad95f15118899a.1
```

`greet`'s result was recorded before the interrupt, so the second `run` ran only
`shout`: one `activity.result` per step.

### Kill and resume

SIGKILL leaves the step in flight claimed, and the guard in front of its command stops
the command. The next `run` waits until the step's lease (10 seconds by default) runs
out, then retries it, because a `read-only` step is safe to repeat. Recorded steps do
not run again.

```sh
spore run hello --key hello-3 & sleep 3; kill -KILL $!; wait $!; echo "exit $?"
spore run hello --key hello-3
```

```text
2026-10-16T04:33:07Z  hello-3  step greet started, attempt 1
2026-10-16T04:33:09Z  hello-3  greet returned ok -> shout
2026-10-16T04:33:09Z  hello-3  step shout started, attempt 1
exit 137
2026-10-16T04:33:19Z  hello-3  step shout attempt 1 failed: attempt.expired: the attempt's lease ran out
2026-10-16T04:33:19Z  hello-3  step shout started, attempt 2
2026-10-16T04:33:21Z  hello-3  shout returned ok -> done
2026-10-16T04:33:21Z  hello-3  completed at done
run        run_0be544f12f0c92d34656ef907b87a55a
start key  hello-3
status     completed
revision   4
...
```

The first attempt of `shout` expired and the kernel scheduled a second. `greet` ran
once.

## review-loop/

`check` fails until `rework` has run, then the run waits for an `approval` signal. `run`
exits 3 while the run waits and prints the commands that answer it. `signal` can come
from any process; `spore continue` (or a `spore worker` that is already running) goes
on from there.

```sh
spore run review-loop --key review-1; echo "exit $?"
spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome approved --id approve-1
spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome approved --id approve-1
spore continue run_04f51c4db6397978f2958d917e176b95
```

```text
2026-10-16T04:33:26Z  review-1  step check started, attempt 1
2026-10-16T04:33:26Z  review-1  check returned red -> rework
2026-10-16T04:33:26Z  review-1  step rework started, attempt 1
2026-10-16T04:33:26Z  review-1  rework returned ok -> check
2026-10-16T04:33:26Z  review-1  step check started, attempt 1
2026-10-16T04:33:26Z  review-1  check returned ok -> approve
2026-10-16T04:33:26Z  review-1  waiting at approve for signal approval
run        run_04f51c4db6397978f2958d917e176b95
start key  review-1
status     waiting
revision   4
...
position   approve (approve/1, from check on ok)
waiting    for signal approval since 2026-10-16T04:33:26Z, for 0s
outcomes   approved
           rejected
answer     spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome approved
           spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome rejected
exit 3
signal approval applied (message ID approve-1)
signal approval duplicate (message ID approve-1)
2026-10-16T04:33:32Z  review-1  approve returned approved -> done
2026-10-16T04:33:32Z  review-1  completed at done
run run_04f51c4db6397978f2958d917e176b95 completed at done
```

Repeating a message ID is a no-op. `--outcome rejected` would end the run in the `fail`
node, and `continue` would exit 1.

### Cancel

A run nobody wants any more is cancelled. It ends `cancelled`, its history is kept, and
a step in progress is stopped.

```sh
spore start review-loop --key review-2
spore cancel run_1cfdd0c834eff27d0dc75ec2bb686f4b --reason "not needed"
spore list
```

```text
created run run_1cfdd0c834eff27d0dc75ec2bb686f4b (start key review-2)
run run_1cfdd0c834eff27d0dc75ec2bb686f4b cancelled (request ID cancel_74de1eca15fc8496)
...
run_04f51c4db6397978f2958d917e176b95  completed           -                  -  review-1
run_1cfdd0c834eff27d0dc75ec2bb686f4b  cancelled           -                  -  review-2
```

## unsafe-deploy/

One `unsafe` step that appends to `deploy.log` and then sleeps for three seconds. Kill
the process while it runs:

```sh
spore run unsafe-deploy --key deploy-1 & sleep 1; kill -KILL $!; wait $!; echo "exit $?"
cat deploy.log
spore run unsafe-deploy --key deploy-1; echo "exit $?"
```

```text
2026-10-16T04:33:38Z  deploy-1  step deploy started, attempt 1
exit 137
deployed inv_d05e1c780633ee634db3bb6504984c13
2026-10-16T04:33:48Z  deploy-1  step deploy attempt 1 failed: attempt.expired: the attempt's lease ran out
2026-10-16T04:33:48Z  deploy-1  needs intervention at deploy: the outcome of attempt 1 is unknown
run        run_e9b5a9c8589b644e0ddb6e1238f1cc9c
start key  deploy-1
status     needs-intervention
revision   2
...
position   deploy (deploy/1)
invocation inv_d05e1c780633ee634db3bb6504984c13 attempt 1 of 1 unknown
process    group 16998 on Max-M5.local
needs      a person: step deploy lost its last attempt's lease with no result since 2026-10-16T04:33:48Z, for 0s
resolve    spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --retry
           spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --complete ok
           spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --fail <message>
exit 4
```

The step started and its result was never recorded. Runspore cannot tell whether the
deploy took effect, and an `unsafe` step is never repeated on its own. After the lease
ran out, the run parked in `needs-intervention` and `run` exited 4. An operator answers
it: `--complete <outcome>` when the effect happened, `--retry` to run it again as
attempt 2, `--fail <message>` to fail the run.

```sh
spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --complete ok --id fix-1
spore continue run_e9b5a9c8589b644e0ddb6e1238f1cc9c
cat deploy.log
```

```text
resolution applied (request ID fix-1)
2026-10-16T04:33:56Z  deploy-1  deploy returned ok -> done
2026-10-16T04:33:56Z  deploy-1  completed at done
run run_e9b5a9c8589b644e0ddb6e1238f1cc9c completed at done
deployed inv_d05e1c780633ee634db3bb6504984c13
```

The deploy ran once. Exit codes and the JSON output (`--json`) are listed in
[spec/cli.md](../spec/cli.md).

## coding-loop/

The reference coding loop as a workflow folder: Claude implements through a plain
`claude -p` step, `npm test` runs, and a person approves. It needs `claude` on `PATH`;
[docs/guide/claude-step.md](../docs/guide/claude-step.md) walks through it.

## watch-a-run/, headless-agent/

`watch-a-run/` holds the two workflows of
[docs/guide/watch-a-run.md](../docs/guide/watch-a-run.md). `headless-agent/` is the
superseded 0.1 research example behind C04; its README says what replaced it.
