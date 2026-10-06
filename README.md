# Runspore

Runspore is an embedded durable workflow runtime designed around a deterministic Rust/WebAssembly graph kernel and replaceable transactional storage.

**Status: architecture and draft contracts.** The runtime is not implemented. Package and domain availability for the name have not been established.

```text
event + state → deterministic kernel → new state + commands
```

Hosts atomically persist the transition and dispatch commands as activities. Non-deterministic outcomes are recorded; ambiguous external effects require idempotency or reconciliation.

## Documentation

Read the [architecture index](docs/README.md) and [delivery plan](docs/17-delivery-plan.md). The design covers Rust, Node, Bun, browser and Cloudflare profiles; typed graphs; effects; SQLite and PostgreSQL adapters; security; OpenTelemetry; and Claude Code / skills integrations.

## Contract seeds

The [contracts](contracts/README.md) directory includes the reducer and telemetry WIT drafts, native async store interface, example activity binding, and 22 conformance scenario definitions. [Validation status](contracts/VALIDATION.txt) records what has and has not been checked.

## First implementation milestone

Prove one graph executes identically through Wasmtime, Node, and Bun; implement atomic SQLite transitions; inject failures around commits and external effects; and verify recovery before expanding to shared workers or edge profiles.

## Project context

This is a general workflow substrate developed under WebTree. The [skills workflow epic](https://github.com/Max-Levitskiy/skills/issues/18) is an optional integration profile rather than the runtime's governing format.

## License status

The architecture proposes dual MIT / Apache-2.0 licensing for original work, subject to dependency review. No license has been applied in this initial documentation commit; licensing must be finalized before distributing runtime code.
