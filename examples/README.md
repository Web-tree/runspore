# Examples

Three workflows for the `runspore` binary. The commands below were run in this order,
in one shell, against a release build; each output block is what they printed. Run and
invocation IDs are derived from the start key, so yours are the same.

## Setup

From the repository root, build the binary and work in a scratch directory with copies
of the examples. The database is `./runspore.db` there.

```sh
cargo build --release -p runspore-cli
export PATH="$PWD/target/release:$PATH"
demo=$(mktemp -d) && cp examples/*.json "$demo" && cd "$demo"
```

## hello.json

Two `read-only` steps of two seconds each; `shout` receives the output of `greet`. Each
step may take up to three attempts.

```sh
runspore validate hello.json
runspore run hello.json --key hello-1
```

```text
hello.json is valid
run        run_1ed55d2b7e22541f12a4d93233ea1f12
start key  hello-1
status     completed
revision   3
result     {"output":{"message":"hello"}}
```

### Interrupt and resume

On SIGINT (Ctrl-C) or SIGTERM the process stops claiming steps, gives the steps in
flight up to `--grace-ms` (10 seconds by default) to finish, records their results, and
exits 130.

```sh
runspore run hello.json --key hello-2 & sleep 1; kill -INT $!; wait $!; echo "exit $?"
runspore run hello.json --key hello-2
runspore events run_02dba8c4bbfa7115e5ac84527ad928f4
```

```text
run        run_02dba8c4bbfa7115e5ac84527ad928f4
start key  hello-2
status     running
revision   2
position   shout (shout/1)
invocation inv_d4aefc08b535626e4827120fbd1cf5b7 attempt 1 scheduled
exit 130
run        run_02dba8c4bbfa7115e5ac84527ad928f4
start key  hello-2
status     completed
revision   3
result     {"output":{"message":"hello"}}
   1  run.started            rev 1      start
   2  activity.result        rev 2      att:inv_8a54ca10ce093bcba43fef226ef14aea.1
   3  activity.result        rev 3      att:inv_d4aefc08b535626e4827120fbd1cf5b7.1
```

`greet` finished during the grace period and its result was recorded. The second `run`
ran only `shout`: one `activity.result` per step.

### Kill and resume

SIGKILL leaves the step in flight claimed. The next `run` waits until its lease (10
seconds by default) runs out, then retries it, because a `read-only` step is safe to
repeat. Recorded steps do not run again.

```sh
runspore run hello.json --key hello-3 & sleep 3; kill -KILL $!; wait $!; echo "exit $?"
runspore run hello.json --key hello-3
runspore events run_59be2f5a2cd74bff8b99b6ca86bc5aa9
```

```text
exit 137
run        run_59be2f5a2cd74bff8b99b6ca86bc5aa9
start key  hello-3
status     completed
revision   4
result     {"output":{"message":"hello"}}
   1  run.started            rev 1      start
   2  activity.result        rev 2      att:inv_e89f5ddc7ccd2a203c2d3e88507d55e0.1
   3  activity.result        rev 3      att:inv_eba64bfa12dcfe3b5641d08d62e662b9.1  activity.retry-scheduled
   4  activity.result        rev 4      att:inv_eba64bfa12dcfe3b5641d08d62e662b9.2
```

Event 3 is the expired first attempt of `shout`; the kernel scheduled a retry. Event 4
is the second attempt. `greet` ran once.

## review-loop.json

`check` fails until `rework` has run, then the run waits for an `approval` signal. `run`
exits 3 while the run waits. `signal` can come from any process; the next `run` (or a
`runspore worker` that is already running) continues from there.

```sh
runspore run review-loop.json --key review-1; echo "exit $?"
runspore events run_698d808071a554a19eaf1cc0c578af57
runspore signal run_698d808071a554a19eaf1cc0c578af57 approval --outcome approved --id approve-1
runspore signal run_698d808071a554a19eaf1cc0c578af57 approval --outcome approved --id approve-1
runspore run review-loop.json --key review-1
```

```text
run        run_698d808071a554a19eaf1cc0c578af57
start key  review-1
status     waiting
revision   4
position   approve (approve/1)
exit 3
   1  run.started            rev 1      start
   2  activity.result        rev 2      att:inv_138162c8309df8cf48cbe8a4c6bf064b.1
   3  activity.result        rev 3      att:inv_47a35a3724b312bec12d500ed3eaceab.1
   4  activity.result        rev 4      att:inv_cf426245203a83a7b53a7b54f5c442db.1
signal approval applied (message ID approve-1)
signal approval duplicate (message ID approve-1)
run        run_698d808071a554a19eaf1cc0c578af57
start key  review-1
status     completed
revision   5
result     {"output":{"approval":"approved","check":"checks pass\n"}}
```

The events are `check` (red), `rework`, `check` (ok). Repeating a message ID is a
no-op. `--outcome rejected` would end the run in the `fail` node, and `run` would exit 1.

## unsafe-deploy.json

One `unsafe` step that appends to `deploy.log` and then sleeps for three seconds. Kill
the process while it runs:

```sh
runspore run unsafe-deploy.json --key deploy-1 & sleep 1; kill -KILL $!; wait $!; echo "exit $?"
cat deploy.log
runspore run unsafe-deploy.json --key deploy-1; echo "exit $?"
```

```text
exit 137
deployed inv_365cb523af61e7de1ded53399d6ee056
run        run_d3f771834c9dca1abe857c4307ef9b6c
start key  deploy-1
status     needs-intervention
revision   2
position   deploy (deploy/1)
invocation inv_365cb523af61e7de1ded53399d6ee056 attempt 1 unknown
exit 4
```

The step started and its result was never recorded. Runspore cannot tell whether the
deploy took effect, and an `unsafe` step is never repeated on its own. After the lease
ran out, the run parked in `needs-intervention` and `run` exited 4. An operator answers
the invocation: `--complete <outcome>` when the effect happened, `--retry` to run it
again as attempt 2, `--fail <message>` to fail the run.

```sh
runspore resolve run_d3f771834c9dca1abe857c4307ef9b6c inv_365cb523af61e7de1ded53399d6ee056 --complete ok --id fix-1
runspore run unsafe-deploy.json --key deploy-1
cat deploy.log
runspore list
```

```text
resolution applied (request ID fix-1)
run        run_d3f771834c9dca1abe857c4307ef9b6c
start key  deploy-1
status     completed
revision   3
result     {"output":{"deployed":"ok"}}
deployed inv_365cb523af61e7de1ded53399d6ee056
run_02dba8c4bbfa7115e5ac84527ad928f4  completed           hello-2
run_1ed55d2b7e22541f12a4d93233ea1f12  completed           hello-1
run_59be2f5a2cd74bff8b99b6ca86bc5aa9  completed           hello-3
run_698d808071a554a19eaf1cc0c578af57  completed           review-1
run_d3f771834c9dca1abe857c4307ef9b6c  completed           deploy-1
```

The deploy ran once. Exit codes and the JSON output (`--json`) are listed in
[spec/cli.md](../spec/cli.md).
