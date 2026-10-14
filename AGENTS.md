# AGENTS.md

## Agent skills

### Issue tracker

Issues live in GitHub Issues on Web-tree/runspore, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default label names: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: one `CONTEXT.md` and `docs/adr/` at the repo root, plus the existing decision docs in `docs/` and `spec/`. See `docs/agents/domain.md`.

## Workarounds are gap signals

Stop and check for a spore gap when one question about a run, a workflow or the build takes you:

- outside `spore`: a script, `sqlite3`, Python, `jq` over `--json` output, or reading files under `.spore/`;
- more than two `spore` calls;
- or telling a person to do one of these.

Finish the task with the workaround. Then write the gap down: the question, the commands it took, and the one command or output that should have answered it. Add it to the open checkpoint's `changes`, or file an issue. Fix it as a general spore capability, copied from how Temporal or n8n solve it, not as a script or a trick in one workflow.
