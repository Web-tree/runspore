# Runspore

Runspore is an embedded durable workflow runtime designed around a deterministic Rust/WebAssembly graph kernel and replaceable transactional storage.

**Status: semantics 0.2.** One binary, `spore`, with a SQLite store and a Rust host that runs the kernel through Wasmtime. A workflow is a folder holding `workflow.dot` and `settings.json`; `spore build` seals it into a workflow artifact. A step can be done by an actor outside the worker or by a job that outlives it, a step that gives up pauses the run for a person, and a run can be cancelled. Node and Bun are verified at kernel level only; there is no JS host. The supported setup is one machine, one OS user and one database per checkout, with no authorization beyond file permissions. Not implemented yet: timers, fork and join, the `workflow` node type, compensation, `reconcilable` effects, a YAML frontend, PostgreSQL, a JS host or SDK, browser and Cloudflare profiles, secrets, telemetry and the rest listed in [spec/README.md](spec/README.md). Website: [runspore.com](https://runspore.com).

```text
event + state → deterministic kernel → new state + commands
```

Hosts atomically persist the transition and dispatch commands as activities. Non-deterministic outcomes are recorded; ambiguous external effects require idempotency or reconciliation.

## Install

On macOS or Linux (arm64 or x86_64):

```sh
curl -fsSL https://raw.githubusercontent.com/Web-tree/runspore/main/install.sh | sh
```

`install.sh` downloads the newest release, checks that `SHA256SUMS` is signed by the release key below and that the archive matches it, checks the build-provenance attestation when `gh` is signed in, and puts `spore` in `~/.local/bin`. The release key:

```text
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGy3mjvtp10UZgB9Y8u3KNBOh66DHPo3bdbZEszh9b1k runspore-release
```

When the line reads `PENDING`, no key has been made yet: there is no signed release and `install.sh` refuses to run, so build from source. Assets, formats and how to check them by hand: [spec/release.md](spec/release.md). To build from source:

```sh
cargo install --git https://github.com/Web-tree/runspore --locked runspore-cli
```

## Releases

release-please keeps a release PR open that bumps the version and adds the merged changes to `CHANGELOG.md`. Merging it cuts the release: release-please tags `v<version>`, and the same workflow run builds, signs, attests and uploads the assets. Every push to `main` builds the four archives too, without publishing them. PR titles are conventional commits (`feat(R4): …`, `fix: …`), since release-please reads them after the squash merge.

Before the first release, a repository admin runs `scripts/new-release-key.sh` once and merges the key it pins; the release PR cannot be merged before that. Once the 0.2 build runs a `spore` whose `ticket` workflow writes conventional PR titles, set the repository variable `PR_TITLE_STRICT` to `true` so a wrong title fails its check instead of only warning. Details: [spec/release.md §8](spec/release.md#8-the-release-cycle).

## Quickstart

From the repository root, build the binary and work in a scratch directory with copies of the [example](examples/README.md) workflow folders:

```sh
cargo build --release -p runspore-cli
export PATH="$PWD/target/release:$PATH"
demo=$(mktemp -d) && cp -R examples/hello examples/review-loop examples/unsafe-deploy "$demo" && cd "$demo"
```

Run a workflow of two steps to completion. `spore` reads the folder, builds its artifact in memory and records it with the run. The database is `~/.spore/db/<folder>/runspore.db`, outside the folder; in a git repository it is `~/.spore/db/<repo key>/runspore.db`, shared by every worktree and clone, and `spore list --json` shows it. Run IDs are random, so yours differ.

```sh
spore run hello --key hello-1
```

```text
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

Kill the process during the second step and run the same key again. The first step does not run again; the second is retried after its lease (10 seconds by default) runs out.

```sh
spore run hello --key hello-3 & sleep 3; kill -KILL $!; wait $!
spore run hello --key hello-3
```

```text
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

A run that waits for a signal exits 3 and prints the commands that answer it. Deliver the signal from any process, then `spore continue`:

```sh
spore run review-loop --key review-1; echo "exit $?"
spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome approved --id approve-1
spore continue run_04f51c4db6397978f2958d917e176b95
```

```text
...
run        run_04f51c4db6397978f2958d917e176b95
start key  review-1
status     waiting
...
waiting    for signal approval since 2026-10-16T04:33:26Z, for 0s
outcomes   approved
           rejected
answer     spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome approved
           spore signal run_04f51c4db6397978f2958d917e176b95 approval --outcome rejected
exit 3
signal approval applied (message ID approve-1)
2026-10-16T04:33:32Z  review-1  approve returned approved -> done
2026-10-16T04:33:32Z  review-1  completed at done
run run_04f51c4db6397978f2958d917e176b95 completed at done
```

An `unsafe` step interrupted after it started is never repeated on its own. The run parks in `needs-intervention` (exit 4) until an operator resolves it:

```sh
spore run unsafe-deploy --key deploy-1 & sleep 1; kill -KILL $!; wait $!
spore run unsafe-deploy --key deploy-1; echo "exit $?"
spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --complete ok --id fix-1
spore continue run_e9b5a9c8589b644e0ddb6e1238f1cc9c
cat deploy.log
```

```text
2026-10-16T04:33:48Z  deploy-1  step deploy attempt 1 failed: attempt.expired: the attempt's lease ran out
2026-10-16T04:33:48Z  deploy-1  needs intervention at deploy: the outcome of attempt 1 is unknown
...
needs      a person: step deploy lost its last attempt's lease with no result since 2026-10-16T04:33:48Z, for 0s
resolve    spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --retry
           spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --complete ok
           spore resolve run_e9b5a9c8589b644e0ddb6e1238f1cc9c --fail <message>
exit 4
resolution applied (request ID fix-1)
2026-10-16T04:33:56Z  deploy-1  deploy returned ok -> done
2026-10-16T04:33:56Z  deploy-1  completed at done
run run_e9b5a9c8589b644e0ddb6e1238f1cc9c completed at done
deployed inv_d05e1c780633ee634db3bb6504984c13
```

A run you no longer want is cancelled; its history stays, and a step in progress is stopped:

```sh
spore start review-loop --key review-2
spore cancel run_1cfdd0c834eff27d0dc75ec2bb686f4b --reason "not needed"
```

```text
created run run_1cfdd0c834eff27d0dc75ec2bb686f4b (start key review-2)
run run_1cfdd0c834eff27d0dc75ec2bb686f4b cancelled (request ID cancel_74de1eca15fc8496)
```

Inside a repository, `spore init` makes the project folder `.spore/`, which holds its workflows and its run database, and every `spore` call from any subfolder uses that database. Commands, exit codes and JSON output: [spec/cli.md](spec/cli.md).

## Guides

- [Watch a run, and get told when it needs you](docs/guide/watch-a-run.md): `spore worker` prints each change of each run, `spore list` shows the runs as a tree, `spore status` says what a run waits for and how to move it on, and `onIntervention` sends a notification when a run stops for a person or fails.
- [A Claude step with plain spore](docs/guide/claude-step.md): a workflow step done by Claude Code, with nothing installed but `spore` and `claude`.

## Spore Workflow Language (SWL)

The independent [Spore Workflow Language specification](https://github.com/Web-tree/spore-workflow-language) is stewarded under WebTree → Runspore → SWL. Read its [layered specification on the Runspore website](https://runspore.com/docs) or directly in the public SWL repository. It defines interoperable workflow interfaces, notation, execution, invocation, and optional state/storage contracts without prescribing this runtime.

**Compatibility note:** SWL is a working draft. Runspore's currently supported authoring dialect and package are implementation-specific; the separate SWL definition and notation are not yet claimed as directly executable by `spore`. A documented adapter and shared conformance tests are required before making that claim.

## Documentation

Read the [architecture index](docs/README.md) and [delivery plan](docs/17-delivery-plan.md). `docs/` is the target architecture: Rust, Node, Bun, browser and Cloudflare profiles; typed graphs; effects; SQLite and PostgreSQL adapters; security; OpenTelemetry. [spec/](spec/README.md) is what `spore` does today, and wins where the two differ. The vocabulary is in [CONTEXT.md](CONTEXT.md). The Claude Code integration is the `ml-workflow` plugin in the `max-skills` repository, specified there.

## Contract seeds

The [contracts](contracts/README.md) directory includes the reducer and telemetry WIT drafts, native async store interface, example activity binding, and 22 conformance scenario definitions. [Validation status](contracts/VALIDATION.txt) records what has and has not been checked.

## Evidence

- The kernel gives byte-identical results natively, through Wasmtime, in Node and in Bun: 76 golden traces plus generated corpora. Run `conformance/js/check.sh`.
- The SQLite store passes the adapter-independent conformance suite and a crash test at every store failpoint.
- The `spore` binary, aborted at each of 19 failpoints and killed at 40 points through a run, recovers: no completed step runs again, and an interrupted `unsafe` step parks the run until an operator resolves it. Run `cargo test -p runspore-cli`.

Not yet done: a soak test, a security review, any target beyond one host with one SQLite file.

## Project context

This is a general workflow substrate developed under WebTree. The [skills workflow epic](https://github.com/Max-Levitskiy/skills/issues/18) is an optional integration profile rather than the runtime's governing format.

## License

MIT. See [LICENSE](LICENSE). Third-party dependencies keep their own licences; all current Rust dependencies are permissive (MIT, Apache-2.0, BSD, Zlib, Unlicense).
