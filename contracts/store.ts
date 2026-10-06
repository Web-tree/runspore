/** Proposed native async semantic protocol. Not an implementation.
 * Every mutation atomically stores request ID, digest, outcome and its writes.
 * Duplicate lookup precedes CAS checks; historical receipts do not renew leases.
 * Counters/timestamps are canonical decimal strings capped at 2^63-1.
 */
export type U64 = string;
export type Digest = string;
export type CanonicalJson = Uint8Array;
export type ID = string;
export interface RunKey { tenant: ID; run: ID }
export interface Mutation {
  requestId: ID;
  requestDigest: Digest;
}
export interface Receipt<T> {
  requestId: ID;
  requestDigest: Digest;
  disposition: "applied" | "duplicate";
  value: T;
}
export type StoreFailure = {
  kind: "conflict" | "stale" | "unavailable" | "unknown-commit" |
        "quota" | "incompatible" | "corrupt" | "unauthorized";
  code: string;
};
export type Result<T> = { ok: true; value: T } | { ok: false; error: StoreFailure };
export interface Capabilities {
  protocol: "0.1";
  atomicRunMutation: true;
  conditionalWrites: true;
  uniqueInsert: true;
  authoritativeReadback: true;
  orderedRecoveryScan: true;
  multiworkerClaims: boolean;
  persistentWakeup: boolean;
  failureModel: string;
  transactionScope: "database" | "run";
  maxTransactionBytes: number;
  maxRecordBytes: number;
}
export interface Event {
  id: ID;
  sequence: U64;
  acceptedAtMs: U64;
  kind: string;
  body: CanonicalJson;
  bodyDigest: Digest;
}
export interface Command {
  id: ID;
  activationId: ID;
  kind: string;
  body: CanonicalJson;
  bodyDigest: Digest;
}
export interface Turn {
  key: RunKey;
  revision: U64;
  appliedSequence: U64;
  ownershipEpoch: U64;
  dispatchGeneration: U64;
  packageDigest: Digest;
  bindingDigest: Digest;
  snapshot: CanonicalJson | null;
  snapshotDigest: Digest | null;
  nextEvent: Event;
}
export interface Commit extends Mutation {
  key: RunKey;
  expectedRevision: U64;
  expectedOwnershipEpoch: U64;
  expectedDispatchGeneration: U64;
  eventId: ID;
  eventSequence: U64;
  decisionDigest: Digest;
  snapshot: CanonicalJson;
  snapshotDigest: Digest;
  commands: Command[];
  // Only kernel-authorized cancellation changes ordinary dispatch eligibility.
  dispatchGate: "unchanged" | "close";
}
export interface CommitReceipt {
  key: RunKey;
  revision: U64;
  appliedSequence: U64;
  decisionDigest: Digest;
}
export interface AttemptRef {
  key: RunKey;
  invocationId: ID;
  attemptId: ID;
  owner: ID;
  fence: U64;
}
export interface Claim extends AttemptRef {
  dispatchGeneration: U64;
  leaseUntilMs: U64;
  operationDeadlineMs: U64;
  effectKey: ID;
  bindingDigest: Digest;
  inputDigest: Digest;
  input: CanonicalJson;
}
export interface Evidence {
  digest: Digest;
  body: CanonicalJson;
}
export interface CursorPage<T> { items: T[]; next: string | null }

export interface SemanticStore {
  capabilities(): Promise<Capabilities>;

  createRun(req: Mutation & {
    key: RunKey;
    startKey: ID;
    packageDigest: Digest;
    bindingDigest: Digest;
    started: Evidence;
  }): Promise<Result<Receipt<{ key: RunKey; startedSequence: U64 }>>>;

  appendEvent(req: Mutation & {
    key: RunKey; eventId: ID; kind: string; body: Evidence;
  }): Promise<Result<Receipt<{ sequence: U64; acceptedAtMs: U64 }>>>;

  loadTurn(key: RunKey): Promise<Result<Turn | null>>;
  commitTurn(req: Commit): Promise<Result<Receipt<CommitReceipt>>>;

  // Absent is not proof an in-flight original request aborted.
  // Receipt includes digest; caller must not invent a new operation ID.
  getReceipt(key: RunKey, requestId: ID): Promise<Result<Receipt<CanonicalJson> | null>>;

  claimAttempt(req: Mutation & {
    key: RunKey;
    invocationId: ID;
    expectedDispatchGeneration: U64;
    worker: ID;
    leaseDurationMs: U64;
  }): Promise<Result<Receipt<Claim>>>;

  // Requires running + current tuple + authority_now < lease_until.
  heartbeat(req: Mutation & {
    attempt: AttemptRef; extendMs: U64; checkpoint?: Evidence;
  }): Promise<Result<Receipt<{ leaseUntilMs: U64 }>>>;

  // Atomic: validate live fence, store evidence, terminate attempt, append event.
  finishAttempt(req: Mutation & {
    attempt: AttemptRef;
    outcome: "success" | "failure" | "unknown";
    evidence: Evidence;
  }): Promise<Result<Receipt<{ resultSequence: U64 }>>>;

  // Atomic: due predicate, fence revocation, terminal attempt, observation event.
  expireAttempt(req: Mutation & {
    attempt: AttemptRef;
  }): Promise<Result<Receipt<{ resultSequence: U64 }>>>;

  // Does not make evidence authoritative workflow output or restore a lease.
  recordLateEvidence(req: Mutation & {
    attempt: AttemptRef; evidence: Evidence;
  }): Promise<Result<Receipt<{ auditId: ID }>>>;

  // Atomic pending->fired and unique inbox event, only at/after stored deadline.
  fireTimer(req: Mutation & {
    key: RunKey; timerId: ID; generation: U64;
  }): Promise<Result<Receipt<{ resultSequence: U64 }>>>;

  scanReady(cursor: string | null, limit: number): Promise<Result<CursorPage<RunKey>>>;
  scanDue(cursor: string | null, limit: number): Promise<Result<CursorPage<{
    key: RunKey; kind: "timer" | "lease" | "outbox"; itemId: ID;
  }>>>;
}

/** Host adapters implement source outbox/destination inbox for remote dispatch,
 * child runs, and signals. Never hold a transaction over network I/O.
 * A remote accepted receipt is not an activity completion. Export/import,
 * authorization, registry initialization and blob protocols are separate APIs.
 */
