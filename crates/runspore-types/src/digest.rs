//! Domain-separated, length-delimited SHA-256 digests and derived identities.
//!
//! `hash(tag, parts) = SHA-256("runspore/v1/" ‖ tag ‖ 0x00 ‖ Σ (u64be(len(part)) ‖ part))`
//! rendered as `sha256:<64 lowercase hex>`. Parts are never concatenated without
//! their length, so no two distinct part lists share an encoding.

use sha2::{Digest as _, Sha256};

use crate::reducer::Decision;

/// `sha256:<64 lowercase hex>`.
pub type Digest = String;

const PREFIX: &[u8] = b"runspore/v1/";

pub fn hash(tag: &str, parts: &[&[u8]]) -> Digest {
    format!("sha256:{}", hash_hex(tag, parts))
}

fn hash_hex(tag: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PREFIX);
    hasher.update(tag.as_bytes());
    hasher.update([0u8]);
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

/// Digest of a workflow package: its canonical document bytes.
pub fn package(canonical_workflow: &[u8]) -> Digest {
    hash("package", &[canonical_workflow])
}

/// Digest of a run snapshot: its canonical state bytes.
pub fn state(canonical_state: &[u8]) -> Digest {
    hash("state", &[canonical_state])
}

/// Digest of any canonical event, command, evidence, or request body.
pub fn body(canonical_body: &[u8]) -> Digest {
    hash("body", &[canonical_body])
}

/// Digest of a mutation request: operation name plus canonical request body.
pub fn request(operation: &str, canonical_body: &[u8]) -> Digest {
    hash("request", &[operation.as_bytes(), canonical_body])
}

/// Digest of a complete kernel decision, in command and diagnostic order.
pub fn decision(decision: &Decision) -> Digest {
    let mut owned: Vec<Vec<u8>> = vec![
        decision.snapshot_digest.as_bytes().to_vec(),
        (decision.commands.len() as u64).to_be_bytes().to_vec(),
    ];
    for command in &decision.commands {
        owned.push(command.command_id.as_bytes().to_vec());
        owned.push(command.activation_id.as_bytes().to_vec());
        owned.push(command.kind.as_bytes().to_vec());
        owned.push(command.payload.clone());
    }
    owned.push((decision.diagnostics.len() as u64).to_be_bytes().to_vec());
    for diagnostic in &decision.diagnostics {
        owned.push(diagnostic.code.as_bytes().to_vec());
        owned.push(diagnostic.node_id.clone().unwrap_or_default().into_bytes());
        owned.push(vec![diagnostic.node_id.is_some() as u8]);
        owned.push(diagnostic.details.clone());
    }
    let parts: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
    hash("decision", &parts)
}

/// Derived identities. Anything that leaves the run (commands, invocations,
/// effect keys) is a truncated hash; activation IDs stay readable and run-local.
pub mod ids {
    use super::hash_hex;

    /// `<nodeId>/<visit>`; `visit` counts entries into the node, starting at 1.
    pub fn activation(node_id: &str, visit: u32) -> String {
        format!("{node_id}/{visit}")
    }

    /// Stable across physical attempts of one logical invocation.
    pub fn invocation(tenant: &str, run_id: &str, activation_id: &str) -> String {
        let hex = hash_hex(
            "invocation",
            &[
                tenant.as_bytes(),
                run_id.as_bytes(),
                activation_id.as_bytes(),
            ],
        );
        format!("inv_{}", &hex[..32])
    }

    /// `ordinal` is 1 for the schedule command and `n` for the retry that authorizes attempt `n`.
    pub fn command(tenant: &str, run_id: &str, activation_id: &str, ordinal: u32) -> String {
        let hex = hash_hex(
            "command",
            &[
                tenant.as_bytes(),
                run_id.as_bytes(),
                activation_id.as_bytes(),
                ordinal.to_string().as_bytes(),
            ],
        );
        format!("cmd_{}", &hex[..32])
    }

    /// The idempotency key handed to activity implementations.
    pub fn effect_key(invocation_id: &str) -> String {
        invocation_id.to_string()
    }

    /// One physical attempt of an invocation.
    pub fn attempt(invocation_id: &str, attempt: u32) -> String {
        format!("{invocation_id}.{attempt}")
    }

    /// Run identity derived from a client start key.
    pub fn run_from_start_key(tenant: &str, start_key: &str) -> String {
        let hex = hash_hex("run", &[tenant.as_bytes(), start_key.as_bytes()]);
        format!("run_{}", &hex[..32])
    }

    pub const EVENT_STARTED: &str = "start";

    pub fn event_for_attempt(attempt_id: &str) -> String {
        format!("att:{attempt_id}")
    }

    pub fn event_for_signal(message_id: &str) -> String {
        format!("sig:{message_id}")
    }

    pub fn event_for_resolution(request_id: &str) -> String {
        format!("res:{request_id}")
    }

    /// Commit request identity: deterministic, so competing coordinators that
    /// compute the same decision collapse into one receipt.
    pub fn commit_request(resulting_revision: u64, event_id: &str) -> String {
        format!("commit/{resulting_revision}/{event_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_delimiting_separates_part_boundaries() {
        assert_ne!(hash("t", &[b"ab", b"c"]), hash("t", &[b"a", b"bc"]));
        assert_ne!(hash("t", &[b"abc"]), hash("t", &[b"abc", b""]));
        assert_ne!(hash("a", &[b"x"]), hash("b", &[b"x"]));
    }

    #[test]
    fn known_vector() {
        // Computed independently (Python hashlib) over
        // b"runspore/v1/body\x00" + u64be(2) + b"{}", so the framing is pinned.
        assert_eq!(
            hash("body", &[b"{}"]),
            "sha256:7f6fcb6872012cddbdbf6a1f895f1b687a70de28298b97d4a42961bb171fab45"
        );
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = ids::invocation("default", "run_1", "build/1");
        assert_eq!(a, ids::invocation("default", "run_1", "build/1"));
        assert_ne!(a, ids::invocation("default", "run_1", "build/2"));
        assert_ne!(a, ids::invocation("default", "run_2", "build/1"));
        assert!(a.starts_with("inv_") && a.len() == 36);
        assert_ne!(
            ids::command("default", "run_1", "build/1", 1),
            ids::command("default", "run_1", "build/1", 2)
        );
        assert_eq!(ids::attempt(&a, 2), format!("{a}.2"));
    }
}
