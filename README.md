# Runspore

Runspore is an embedded durable workflow runtime designed around a deterministic Rust/WebAssembly graph kernel and replaceable transactional storage.

**Status: MVP implemented.** One binary, `spore`, with a SQLite store and a Rust host that runs the kernel through Wasmtime. Node and Bun are verified at kernel level only; there is no JS host. Not implemented yet: timers, fork and join, child workflows, compensation, cancellation, `reconcilable` effects, input and output schemas, a compiler or YAML/DOT frontends, PostgreSQL, a JS host or SDK, browser and Cloudflare profiles, secrets, telemetry and the rest listed in [spec/README.md](spec/README.md). Website: [runspore.com](https://runspore.com).

```text
event + state → deterministic kernel → new state + commands
```

Hosts atomically persist the transition and dispatch commands as activities. Non-deterministic outcomes are recorded; ambiguous external effects require idempotency or reconciliation.

## Quickstart

From the repository root, build the binary and work in a scratch directory with copies of the [examples](examples/README.md):

```sh
cargo build --release -p runspore-cli
export PATH="$PWD/target/release:$PATH"
demo=$(mktemp -d) && cp examples/*.json "$demo" && cd "$demo"
```

Run a workflow of two steps to completion. The database is `./runspore.db`.

```sh
spore run hello.json --key hello-1
```

```text
run        run_1ed55d2b7e22541f12a4d93233ea1f12
start key  hello-1
status     completed
revision   3
result     {"output":{"message":"hello"}}
```

Kill the process during the second step and run the same key again. The first step does not run again; the second is retried after its lease (10 seconds by default) runs out.

```sh
spore run hello.json --key hello-3 & sleep 3; kill -KILL $!; wait $!
spore run hello.json --key hello-3
```

```text
run        run_59be2f5a2cd74bff8b99b6ca86bc5aa9
start key  hello-3
status     completed
revision   4
result     {"output":{"message":"hello"}}
```

A run that waits for a signal exits 3. Deliver the signal from any process and continue:

```sh
spore run review-loop.json --key review-1; echo "exit $?"
spore signal run_698d808071a554a19eaf1cc0c578af57 approval --outcome approved --id approve-1
spore run review-loop.json --key review-1
```

```text
run        run_698d808071a554a19eaf1cc0c578af57
start key  review-1
status     waiting
revision   4
position   approve (approve/1)
exit 3
signal approval applied (message ID approve-1)
run        run_698d808071a554a19eaf1cc0c578af57
start key  review-1
status     completed
revision   5
result     {"output":{"approval":"approved","check":"checks pass\n"}}
```

An `unsafe` step interrupted after it started is never repeated on its own. The run parks in `needs-intervention` (exit 4) until an operator resolves the invocation:

```sh
spore run unsafe-deploy.json --key deploy-1 & sleep 1; kill -KILL $!; wait $!
spore run unsafe-deploy.json --key deploy-1; echo "exit $?"
spore resolve run_d3f771834c9dca1abe857c4307ef9b6c inv_365cb523af61e7de1ded53399d6ee056 --complete ok --id fix-1
spore run unsafe-deploy.json --key deploy-1
cat deploy.log
```

```text
run        run_d3f771834c9dca1abe857c4307ef9b6c
start key  deploy-1
status     needs-intervention
revision   2
position   deploy (deploy/1)
invocation inv_365cb523af61e7de1ded53399d6ee056 attempt 1 unknown
exit 4
resolution applied (request ID fix-1)
run        run_d3f771834c9dca1abe857c4307ef9b6c
start key  deploy-1
status     completed
revision   3
result     {"output":{"deployed":"ok"}}
deployed inv_365cb523af61e7de1ded53399d6ee056
```

Commands, exit codes and JSON output: [spec/cli.md](spec/cli.md).

## Documentation

Read the [architecture index](docs/README.md) and [delivery plan](docs/17-delivery-plan.md). The design covers Rust, Node, Bun, browser and Cloudflare profiles; typed graphs; effects; SQLite and PostgreSQL adapters; security; OpenTelemetry; and Claude Code / skills integrations.

## Contract seeds

The [contracts](contracts/README.md) directory includes the reducer and telemetry WIT drafts, native async store interface, example activity binding, and 22 conformance scenario definitions. [Validation status](contracts/VALIDATION.txt) records what has and has not been checked.

## Evidence

- The kernel gives byte-identical results natively, through Wasmtime, in Node and in Bun: 48 golden traces plus generated corpora. Run `conformance/js/check.sh`.
- The SQLite store passes the adapter-independent conformance suite and a crash test at every store failpoint.
- The `spore` binary, aborted at each of 19 failpoints and killed at 40 points through a run, recovers: no completed step runs again, and an interrupted `unsafe` step parks the run until an operator resolves it. Run `cargo test -p runspore-cli`.

Not yet done: a soak test, a security review, any target beyond one host with one SQLite file.

## Project context

This is a general workflow substrate developed under WebTree. The [skills workflow epic](https://github.com/Max-Levitskiy/skills/issues/18) is an optional integration profile rather than the runtime's governing format.

## License

MIT. See [LICENSE](LICENSE). Third-party dependencies keep their own licences; all current Rust dependencies are permissive (MIT, Apache-2.0, BSD, Zlib, Unlicense).
