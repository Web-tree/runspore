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

## Commands

| Command | Does |
| --- | --- |
| `validate <workflow.json>` | Checks the document and its actions without touching the database |
| `start <workflow.json> [--input <json> \| --input-file <path>] [--key <startKey>]` | Creates a run and returns. A missing key is generated; the same key always names the same run |
| `run <workflow.json> [--input …] [--key …]` | `start`, then works until the run is terminal or parked |
| `worker [--run <runId>] [--until-parked]` | Works on all runs until interrupted. With `--run --until-parked`, on one run until it is terminal or parked |
| `status <runId>` | Status, position, result, quarantine reason, outstanding invocation |
| `list` | All runs with status |
| `events <runId>` | The run's events in order, with the revision that consumed each and its diagnostics |
| `signal <runId> <name> [--outcome <o>] [--data <json>] [--id <messageId>]` | Delivers a signal. A missing ID is generated; repeating an ID is a no-op |
| `resolve <runId> <invocationId> (--complete <outcome> [--output <json>] \| --retry \| --fail <message>) [--id <requestId>]` | Answers an invocation whose outcome is unknown |
| `release <runId>` | Lifts a quarantine |

`run` and `worker --until-parked` are the same loop; `run` on an existing key resumes it.
`worker --run` and `--until-parked` are only valid together. `release` on a run that is
not quarantined succeeds and changes nothing.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. For `run` and `worker --until-parked`: the run completed |
| 1 | The run failed |
| 2 | Usage or validation error |
| 3 | The run is waiting for a signal |
| 4 | The run needs intervention |
| 5 | The run is quarantined |
| 10 | Store or internal error |
| 130 | Interrupted by SIGINT or SIGTERM after a graceful shutdown |

## JSON output

Every object carries `"format": "runspore.cli/0.1"`. Counters and timestamps are
decimal strings. `status` output:

```json
{"format": "runspore.cli/0.1", "runId": "run_…", "status": "needs-intervention",
 "revision": "4", "position": {"nodeId": "deploy", "activationId": "deploy/1", "visit": 1},
 "invocation": {"invocationId": "inv_…", "attempt": 1, "state": "unknown"},
 "result": null, "quarantine": null}
```

Text output is for people and is not a stable interface.

## Interrupts

On SIGINT or SIGTERM the worker stops claiming, gives in-flight activities a short
grace period, and exits. Work it leaves behind is picked up by the next worker.
