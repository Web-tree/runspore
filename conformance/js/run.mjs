// Replays every compiled trace listed in build/export/manifest.json through the
// jco-transpiled kernel component and requires the exact expected bytes.
//
// Runs unchanged under `node run.mjs` and `bun run.mjs`. Holds no workflow or
// canonical-JSON logic: request bytes are the UTF-8 of the compiled strings,
// and results are compared byte for byte with the UTF-8 of the expected
// strings. 64-bit fields go from decimal strings straight to BigInt.
//
// Each transition runs in a fresh instance of the component, as in the Rust
// host; the compiled core module is shared, since it holds no state.

import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { instantiate } from './build/transpiled/kernel.js';

const here = dirname(fileURLToPath(import.meta.url));
const exportDir = join(here, 'build', 'export');
const transpiledDir = join(here, 'build', 'transpiled');
const encoder = new TextEncoder();
const decoder = new TextDecoder();

const runtime = globalThis.Bun
  ? `bun ${globalThis.Bun.version}`
  : `node ${process.versions.node}`;

class Mismatch extends Error {}

function fail(trace, file, index, step, field, expected, actual) {
  throw new Mismatch(
    `${runtime}: ${trace.name} (${file}) step ${index} event ${step.event.eventId}: ${field} differs\n` +
      excerpt(expected, actual),
  );
}

/** Both values around the first differing character, at most 160 characters each. */
function excerpt(expected, actual) {
  const a = String(expected);
  const b = String(actual);
  let at = 0;
  while (at < a.length && at < b.length && a[at] === b[at]) at += 1;
  const from = Math.max(0, at - 60);
  const cut = (s) => (from > 0 ? '…' : '') + s.slice(from, from + 160) + (s.length > from + 160 ? '…' : '');
  return `  first difference at character ${at}\n  expected: ${cut(a)}\n  actual:   ${cut(b)}`;
}

function sameBytes(bytes, expected) {
  const want = encoder.encode(expected);
  if (bytes.length !== want.length) return false;
  for (let i = 0; i < want.length; i += 1) if (bytes[i] !== want[i]) return false;
  return true;
}

/** A u64 from its decimal string; a JSON number here would already have lost precision. */
function u64(value, what) {
  if (typeof value !== 'string' || !/^(0|[1-9][0-9]*)$/.test(value)) {
    throw new Error(`${what} must be a decimal string, found ${JSON.stringify(value)}`);
  }
  return BigInt(value);
}

function describe(result) {
  if (result.failure) return `failure ${JSON.stringify(result.failure)}`;
  const d = result.decision;
  return `decision ${JSON.stringify({
    snapshot: decoder.decode(d.snapshot),
    snapshotDigest: d.snapshotDigest,
    commands: d.commands.map((c) => ({ ...c, payload: decoder.decode(c.payload) })),
    diagnostics: d.diagnostics.map((g) => ({ ...g, nodeId: g.nodeId ?? null, details: decoder.decode(g.details) })),
  })}`;
}

async function transition(core, request) {
  const instance = await instantiate(() => core, {});
  try {
    return { decision: instance.reducer.transition(request) };
  } catch (error) {
    const failure = error?.payload;
    if (failure && typeof failure.kind === 'string' && typeof failure.code === 'string') {
      return { failure };
    }
    throw error;
  }
}

async function replay(file, detailsFile, timing) {
  const trace = JSON.parse(await readFile(join(exportDir, file), 'utf8'));
  const details = JSON.parse(await readFile(join(exportDir, detailsFile), 'utf8'));
  if (trace.format !== 'runspore.trace.compiled/0.1') {
    throw new Error(`${file}: unsupported format ${trace.format}`);
  }
  if (details.length !== trace.steps.length) {
    throw new Error(`${detailsFile}: ${details.length} entries for ${trace.steps.length} steps`);
  }
  const graph = encoder.encode(trace.workflow);
  let snapshot;
  for (const [index, step] of trace.steps.entries()) {
    const event = step.event;
    const request = {
      identity: step.identity ?? trace.identity,
      graph,
      snapshot: step.snapshot !== undefined ? encoder.encode(step.snapshot) : snapshot,
      inputEvent: {
        eventId: event.eventId,
        sequence: u64(event.sequence, 'sequence'),
        acceptedAtMs: u64(event.acceptedAtMs, 'acceptedAtMs'),
        kind: event.kind,
        payload: encoder.encode(event.payload),
      },
      frozenLimits: trace.limits,
    };
    const started = performance.now();
    const result = await transition(timing.core, request);
    timing.ms += performance.now() - started;
    timing.calls += 1;
    const expect = step.expect;
    const check = (field, ok, expected, actual) => {
      if (!ok) fail(trace, file, index, step, field, expected, actual);
    };
    if (expect.failure) {
      check('result', result.failure, `failure ${JSON.stringify(expect.failure)}`, describe(result));
      check('failure.kind', result.failure.kind === expect.failure.kind, expect.failure.kind, result.failure.kind);
      check('failure.code', result.failure.code === expect.failure.code, expect.failure.code, result.failure.code);
      check('failure.details', result.failure.details === details[index], details[index], result.failure.details);
      continue;
    }
    check('result', result.decision, 'a decision', describe(result));
    const d = result.decision;
    check('snapshot', sameBytes(d.snapshot, expect.snapshot), expect.snapshot, decoder.decode(d.snapshot));
    check('snapshotDigest', d.snapshotDigest === expect.snapshotDigest, expect.snapshotDigest, d.snapshotDigest);
    check('commands.length', d.commands.length === expect.commands.length, expect.commands.length, d.commands.length);
    for (const [i, want] of expect.commands.entries()) {
      const got = d.commands[i];
      for (const key of ['commandId', 'activationId', 'kind']) {
        check(`commands[${i}].${key}`, got[key] === want[key], want[key], got[key]);
      }
      check(`commands[${i}].payload`, sameBytes(got.payload, want.payload), want.payload, decoder.decode(got.payload));
    }
    check('diagnostics.length', d.diagnostics.length === expect.diagnostics.length, expect.diagnostics.length, d.diagnostics.length);
    for (const [i, want] of expect.diagnostics.entries()) {
      const got = d.diagnostics[i];
      const nodeId = got.nodeId ?? null;
      check(`diagnostics[${i}].code`, got.code === want.code, want.code, got.code);
      check(`diagnostics[${i}].nodeId`, nodeId === want.nodeId, want.nodeId, nodeId);
      check(`diagnostics[${i}].details`, sameBytes(got.details, want.details), want.details, decoder.decode(got.details));
    }
    snapshot = d.snapshot;
  }
  return trace.steps.length;
}

async function main() {
  const manifest = JSON.parse(await readFile(join(exportDir, 'manifest.json'), 'utf8'));
  const component = await readFile(join(exportDir, 'component.wasm'));
  const componentSha256 = `sha256:${createHash('sha256').update(component).digest('hex')}`;
  if (componentSha256 !== manifest.componentSha256) {
    throw new Error(`component.wasm is ${componentSha256}, manifest says ${manifest.componentSha256}`);
  }
  const stamp = (await readFile(join(transpiledDir, 'component.sha256'), 'utf8')).trim();
  if (stamp !== componentSha256) {
    throw new Error(`build/transpiled was made from ${stamp}, not ${componentSha256}: transpile again`);
  }
  const coreBytes = await readFile(join(transpiledDir, 'kernel.core.wasm'));
  const glueBytes = await readFile(join(transpiledDir, 'kernel.js'));
  const timing = { core: await WebAssembly.compile(coreBytes), ms: 0, calls: 0 };

  let steps = 0;
  for (const entry of manifest.traces) {
    steps += await replay(entry.trace, entry.failureDetails, timing);
  }
  if (manifest.traces.length !== manifest.traceCount || steps !== manifest.stepCount) {
    throw new Error(`replayed ${manifest.traces.length} traces and ${steps} steps, manifest says ${manifest.traceCount} and ${manifest.stepCount}`);
  }
  const perCall = ((timing.ms * 1000) / timing.calls).toFixed(0);
  console.log(
    `${runtime}: ${manifest.traces.length} traces, ${steps} steps passed; component ${componentSha256}; ` +
      `transpiled ${coreBytes.length} B core + ${glueBytes.length} B glue; ` +
      `${perCall} µs per transition including a fresh instance (this machine)`,
  );
}

main().catch((error) => {
  console.error(error instanceof Mismatch ? error.message : error);
  process.exit(1);
});
