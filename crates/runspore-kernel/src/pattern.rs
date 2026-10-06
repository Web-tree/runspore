//! The identifier patterns of `spec/kernel.md`, matched byte by byte. Every pattern
//! is ASCII-only, so a byte length is a character length.

fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn dotted_word(byte: u8) -> bool {
    word(byte) || byte == b'.'
}

fn lower(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn lower_dash(byte: u8) -> bool {
    lower(byte) || byte == b'-'
}

fn lower_dotted(byte: u8) -> bool {
    lower_dash(byte) || byte == b'.'
}

fn all(text: &str, max: usize, allowed: fn(u8) -> bool) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(allowed)
}

fn starts_lower(text: &str) -> bool {
    text.bytes().next().is_some_and(lower)
}

/// `[A-Za-z0-9_.-]{1,128}`: workflow names, tenants, run IDs.
pub(crate) fn is_name(text: &str) -> bool {
    all(text, 128, dotted_word)
}

/// `[A-Za-z0-9_-]{1,64}`: node and action IDs.
pub(crate) fn is_id(text: &str) -> bool {
    all(text, 64, word)
}

/// `[A-Za-z0-9_.-]{1,64}`: signal names.
pub(crate) fn is_signal(text: &str) -> bool {
    all(text, 64, dotted_word)
}

/// `[a-z0-9][a-z0-9-]{0,63}`: outcome names.
pub(crate) fn is_outcome(text: &str) -> bool {
    all(text, 64, lower_dash) && starts_lower(text)
}

/// `[a-z0-9][a-z0-9.-]{0,63}`: error codes of fail nodes.
pub(crate) fn is_error_code(text: &str) -> bool {
    all(text, 64, lower_dotted) && starts_lower(text)
}
