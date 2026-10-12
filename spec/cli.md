# CLI 0.1

One binary, `spore`, the Runspore command line. It embeds the kernel component, the SQLite store, and the
engine. It never listens on a network.

## Global options

| Option | Default | Meaning |
| --- | --- | --- |
| `--db <path>` | `$RUNSPORE_DB`, else `./runspore.db` | Database file |
| `--json` | off | One JSON object on stdout instead of text |
| `--native-kernel` | off | Call the kernel natively instead of through Wasmtime; for debugging |
| `--lease-ms`, `--heartbeat-ms`, `--poll-ms`, `--grace-ms` | engine defaults | Worker tuning; mainly for tests |
| `--stop-grace-ms` | 5000 | Time a stopped command's process group gets between SIGTERM and SIGKILL (host.md §3, `stop_grace_ms`); used by `worker` and `run` |

## Commands

| Command | Does |
| --- | --- |
| `validate <workflow>` | Builds the workflow and checks it and its actions without touching the database |
| `start <workflow> [--input <json> \| --input-file <path>] [--key <startKey>]` | Creates a run and returns its run ID. A missing key is generated; the same key always names the same run in this database |
| `run <workflow> [--input …] [--key …]` | `start`, then works until the run is terminal or parked |
| `worker [--run <runId>] [--until-parked]` | Works on all runs until interrupted. With `--run --until-parked`, on one run until it is terminal or parked |
| `status <runId>` | Status, position and the arrow into it, result and the end node, quarantine reason, outstanding invocation |
| `list` | All runs with status |
| `events <runId>` | The run's events in order, with the revision that consumed each and its diagnostics |
| `signal <runId> <name> [--outcome <o>] [--data <json>] [--id <messageId>]` | Delivers a signal. A missing ID is generated; repeating an ID is a no-op |
| `resolve <runId> [<invocationId>] (--complete <outcome> [--output <json>] \| --retry \| --fail <message>) [--id <requestId>]` | Answers a stuck step, or an invocation whose outcome is unknown. Without an invocation ID it answers the run's outstanding invocation; a run with none exits 2 (`run.no-invocation`) |
| `release <runId>` | Lifts a quarantine |

A `<workflow>` is a workflow folder or its `workflow.dot`, which is built in memory
as [dot.md](dot.md) describes, or a JSON workflow document. JSON documents are still
accepted until the cut-over to 0.2. A build error exits 2 and names the file and line
(dot.md §6).

`run` and `worker --until-parked` are the same loop; `run` on an existing key resumes it.
Run IDs are random (host §2): `start` and `run` print the run ID the engine returned,
and the same key in another database names another run.
`worker --run` and `--until-parked` are only valid together. `release` on a run that is
not quarantined succeeds and changes nothing.

A step of a 0.2 workflow that gives up with no `failed` route is stuck (kernel.md
§7.4): the run stops with exit 4 and waits for a person. `resolve --retry` runs a new
try of the same invocation, with the same effect key, even past the action's
`maxAttempts`. `--complete <outcome>` continues as if the step had returned that
outcome. `--fail <message>` ends the run as failed with the step's kept error; the
message is recorded in the event only, and the run never pauses on that step again.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. For `run` and `worker --until-parked`: the run completed |
| 1 | The run failed |
| 2 | Usage or validation error |
| 3 | The run is waiting for a signal |
| 4 | The run needs intervention: a stuck step, or an unsafe step with an unknown outcome |
| 5 | The run is quarantined |
| 10 | Store or internal error |
| 130 | Interrupted by SIGINT, SIGTERM or SIGHUP after a graceful shutdown |

## JSON output

Every object carries `"format": "runspore.cli/0.1"`. Counters and timestamps are
decimal strings. `status` output:

```json
{"format": "runspore.cli/0.1", "runId": "run_…", "status": "needs-intervention",
 "revision": "4", "position": {"nodeId": "deploy", "activationId": "deploy/1", "visit": 1,
                               "from": {"nodeId": "test", "outcome": "ok"}},
 "invocation": {"invocationId": "inv_…", "attempt": 3, "maxAttempts": 3, "state": "stuck",
                "error": {"code": "command.exit", "message": "exit code 1", "nodeId": "deploy",
                          "details": {…}}},
 "result": null, "end": null, "quarantine": null}
```

`position.from` is the arrow the run took into its position: the node whose result
routed there and the outcome it routed on, the same arrow a mapping reads as `from`
(kernel.md §4). It is `null` at the start node's first activation. `end` is `null`
unless the run is `completed` or `failed`; then it is `{"nodeId", "from"}`: the node
the run ended at and the arrow into it, for example
`{"nodeId": "done", "from": {"nodeId": "approve", "outcome": "approved"}}`. The node is
the complete or fail node the run reached, or the node named by the error that ended
it (`result.error.nodeId`): a step that failed with no `failed` arrow, or a node it
could not enter or whose mapping did not resolve. `from` is `null` when a run ends at
its start node.

The kernel's state holds no arrow. `status` finds both by replaying the run's consumed
events through the kernel: for each transition that moved the run, it starts at the
node the run was parked at and follows the route of the result each node then holds,
until it reaches the new node. The cost is one transition per event of the run.

`invocation` is the run's outstanding invocation, as the kernel state holds it
(kernel.md §3), plus `maxAttempts` from its action's retry policy (1 when it has
none). `state` is `scheduled`, `unknown` or `stuck`; at a stuck step `error` is the
kept error. `run` and `worker --until-parked` print the same object.

Text output is for people and is not a stable interface.

## Interrupts

On SIGINT, SIGTERM or SIGHUP the worker stops claiming, gives in-flight activities a
short grace period, stops the commands still running with the stop sequence (host.md
§3), and exits. Closing the terminal (SIGHUP) therefore shuts down like Ctrl-C. Work it
leaves behind is picked up by the next worker.

A worker that dies without a graceful shutdown (SIGKILL, a crash) still stops its
commands: each one's guard stops its process group. The guard is a hidden mode of
`spore` itself; it is not a command.
