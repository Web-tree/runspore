CREATE TABLE packages (
    digest TEXT PRIMARY KEY,
    body BLOB NOT NULL
);
CREATE TABLE runs (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    start_key TEXT NOT NULL,
    package_digest TEXT NOT NULL REFERENCES packages (digest),
    started_digest TEXT NOT NULL,
    status TEXT NOT NULL,
    revision INTEGER NOT NULL,
    applied_sequence INTEGER NOT NULL,
    next_sequence INTEGER NOT NULL,
    last_accepted_ms INTEGER NOT NULL,
    snapshot BLOB,
    snapshot_digest TEXT,
    fence INTEGER NOT NULL,
    quarantine_code TEXT,
    quarantine_details TEXT,
    quarantine_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (tenant, run_id),
    UNIQUE (tenant, start_key)
);
CREATE TABLE events (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    event_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    body BLOB NOT NULL,
    body_digest TEXT NOT NULL,
    accepted_at_ms INTEGER NOT NULL,
    consumed_revision INTEGER,
    PRIMARY KEY (tenant, run_id, sequence),
    UNIQUE (tenant, run_id, event_id),
    FOREIGN KEY (tenant, run_id) REFERENCES runs (tenant, run_id)
);
CREATE TABLE transitions (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    consumed_sequence INTEGER NOT NULL,
    decision_digest TEXT NOT NULL,
    snapshot_digest TEXT NOT NULL,
    diagnostics TEXT NOT NULL,
    committed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (tenant, run_id, revision),
    UNIQUE (tenant, run_id, consumed_sequence),
    FOREIGN KEY (tenant, run_id) REFERENCES runs (tenant, run_id)
);
CREATE TABLE commands (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    activation_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (tenant, run_id, command_id),
    FOREIGN KEY (tenant, run_id, revision) REFERENCES transitions (tenant, run_id, revision)
);
CREATE TABLE invocations (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    action_id TEXT NOT NULL,
    effect_key TEXT NOT NULL,
    input BLOB NOT NULL,
    input_digest TEXT NOT NULL,
    status TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    not_before_ms INTEGER NOT NULL,
    PRIMARY KEY (tenant, run_id, invocation_id),
    FOREIGN KEY (tenant, run_id) REFERENCES runs (tenant, run_id)
);
CREATE INDEX invocations_due ON invocations (status, not_before_ms, invocation_id);
CREATE TABLE attempts (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    owner TEXT NOT NULL,
    fence INTEGER NOT NULL,
    lease_until_ms INTEGER NOT NULL,
    status TEXT NOT NULL,
    result BLOB,
    PRIMARY KEY (tenant, run_id, attempt_id),
    UNIQUE (tenant, run_id, invocation_id, attempt_number),
    UNIQUE (tenant, run_id, fence),
    FOREIGN KEY (tenant, run_id, invocation_id) REFERENCES invocations (tenant, run_id, invocation_id)
);
CREATE INDEX attempts_due ON attempts (status, lease_until_ms, attempt_id);
CREATE TABLE receipts (
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    operation TEXT NOT NULL,
    value BLOB NOT NULL,
    PRIMARY KEY (tenant, run_id, request_id)
);
CREATE TABLE audit (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant TEXT NOT NULL,
    run_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    evidence_digest TEXT NOT NULL,
    evidence BLOB NOT NULL,
    at_ms INTEGER NOT NULL
);
