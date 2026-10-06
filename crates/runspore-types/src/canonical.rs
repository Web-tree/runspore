//! Canonical JSON: RFC 8785 (JCS) over a narrowed value domain.
//!
//! Domain: null, booleans, integers within ±(2^53 − 1), strings, arrays, objects.
//! Rejected: non-integer numbers, out-of-range integers, duplicate object keys,
//! nesting deeper than [`MAX_DEPTH`]. Any zero-valued number token (`-0`, `0.0`)
//! normalizes to `0`. Strings are preserved without Unicode normalization.
//!
//! Encoding: no whitespace, object keys sorted by UTF-16 code units, minimal
//! string escapes with lowercase hex.

use std::fmt;

use serde::de::{DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use serde_json::{Map, Number, Value};

pub const MAX_SAFE_INT: i64 = 9_007_199_254_740_991;
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanonError {
    #[error("invalid JSON: {0}")]
    Syntax(String),
    #[error("duplicate object key {0:?}")]
    DuplicateKey(String),
    #[error("non-integer number")]
    NonInteger,
    #[error("integer outside ±(2^53 − 1)")]
    IntegerOutOfRange,
    #[error("nesting deeper than {MAX_DEPTH}")]
    TooDeep,
    #[error("input is valid but not in canonical form")]
    NotCanonical,
    #[error("value does not match the expected shape: {0}")]
    Shape(String),
}

impl CanonError {
    /// Stable machine-readable code, used in kernel failures and store errors.
    pub fn code(&self) -> &'static str {
        match self {
            CanonError::Syntax(_) => "canonical.syntax",
            CanonError::DuplicateKey(_) => "canonical.duplicate-key",
            CanonError::NonInteger => "canonical.non-integer",
            CanonError::IntegerOutOfRange => "canonical.integer-out-of-range",
            CanonError::TooDeep => "canonical.too-deep",
            CanonError::NotCanonical => "canonical.not-canonical",
            CanonError::Shape(_) => "canonical.shape",
        }
    }
}

/// Parses any JSON text whose values fit the domain. Whitespace and key order are free.
pub fn parse(bytes: &[u8]) -> Result<Value, CanonError> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = Domain { depth: 0 }
        .deserialize(&mut de)
        .map_err(unwrap_marker)?;
    de.end().map_err(unwrap_marker)?;
    Ok(value)
}

/// Parses JSON that must already be the exact canonical encoding of its value.
pub fn parse_canonical(bytes: &[u8]) -> Result<Value, CanonError> {
    let value = parse(bytes)?;
    if to_vec(&value)? != bytes {
        return Err(CanonError::NotCanonical);
    }
    Ok(value)
}

/// Canonical encoding of a value. Fails if the value is outside the domain.
pub fn to_vec(value: &Value) -> Result<Vec<u8>, CanonError> {
    let mut out = Vec::new();
    write_value(value, 0, &mut out)?;
    Ok(out)
}

/// Canonical encoding of any serializable type.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonError> {
    let value = serde_json::to_value(value).map_err(|e| CanonError::Shape(e.to_string()))?;
    to_vec(&value)
}

/// Decodes a typed value from bytes that must be canonical.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CanonError> {
    let value = parse_canonical(bytes)?;
    serde_json::from_value(value).map_err(|e| CanonError::Shape(e.to_string()))
}

fn write_value(value: &Value, depth: usize, out: &mut Vec<u8>) -> Result<(), CanonError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(integer(n)?.to_string().as_bytes()),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            if depth >= MAX_DEPTH {
                return Err(CanonError::TooDeep);
            }
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_value(item, depth + 1, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            if depth >= MAX_DEPTH {
                return Err(CanonError::TooDeep);
            }
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push(b'{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_value(item, depth + 1, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn integer(n: &Number) -> Result<i64, CanonError> {
    let v = if let Some(v) = n.as_i64() {
        v
    } else if n.as_u64().is_some() {
        return Err(CanonError::IntegerOutOfRange);
    } else {
        match n.as_f64() {
            Some(0.0) => 0,
            _ => return Err(CanonError::NonInteger),
        }
    };
    if !(-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&v) {
        return Err(CanonError::IntegerOutOfRange);
    }
    Ok(v)
}

fn write_string(s: &str, out: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for &b in s.as_bytes() {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            b if b < 0x20 => {
                out.extend_from_slice(b"\\u00");
                out.push(HEX[(b >> 4) as usize]);
                out.push(HEX[(b & 0x0f) as usize]);
            }
            b => out.push(b),
        }
    }
    out.push(b'"');
}

// Domain errors travel through serde's error type as a marker prefix and are
// recovered here, so the caller sees the precise variant.
const MARKER: &str = "\u{1}runspore:";

fn marker<E: serde::de::Error>(err: &CanonError) -> E {
    let body = match err {
        CanonError::DuplicateKey(k) => format!("dup:{k}"),
        CanonError::NonInteger => "nonint".to_string(),
        CanonError::IntegerOutOfRange => "range".to_string(),
        CanonError::TooDeep => "deep".to_string(),
        other => format!("other:{other}"),
    };
    E::custom(format!("{MARKER}{body}\u{1}"))
}

fn unwrap_marker(err: serde_json::Error) -> CanonError {
    let text = err.to_string();
    let Some(start) = text.find(MARKER) else {
        return CanonError::Syntax(text);
    };
    let rest = &text[start + MARKER.len()..];
    let body = rest.split('\u{1}').next().unwrap_or("");
    if let Some(key) = body.strip_prefix("dup:") {
        CanonError::DuplicateKey(key.to_string())
    } else {
        match body {
            "nonint" => CanonError::NonInteger,
            "range" => CanonError::IntegerOutOfRange,
            "deep" => CanonError::TooDeep,
            other => CanonError::Syntax(other.to_string()),
        }
    }
}

#[derive(Clone, Copy)]
struct Domain {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Domain {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Domain {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON value in the Runspore canonical domain")
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        if !(-MAX_SAFE_INT..=MAX_SAFE_INT).contains(&v) {
            return Err(marker(&CanonError::IntegerOutOfRange));
        }
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        if v > MAX_SAFE_INT as u64 {
            return Err(marker(&CanonError::IntegerOutOfRange));
        }
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        if v == 0.0 {
            return Ok(Value::Number(0.into()));
        }
        // serde_json reports integers beyond u64/i64 as floats.
        if v.is_finite() && v.fract() == 0.0 && v.abs() > MAX_SAFE_INT as f64 {
            return Err(marker(&CanonError::IntegerOutOfRange));
        }
        Err(marker(&CanonError::NonInteger))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        if self.depth >= MAX_DEPTH {
            return Err(marker(&CanonError::TooDeep));
        }
        let child = Domain {
            depth: self.depth + 1,
        };
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(child)? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        if self.depth >= MAX_DEPTH {
            return Err(marker(&CanonError::TooDeep));
        }
        let child = Domain {
            depth: self.depth + 1,
        };
        let mut map = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            let value = access.next_value_seed(child)?;
            if map.contains_key(&key) {
                return Err(marker(&CanonError::DuplicateKey(key)));
            }
            map.insert(key, value);
        }
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(text: &str) -> String {
        String::from_utf8(to_vec(&parse(text.as_bytes()).unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn sorts_keys_and_strips_whitespace() {
        assert_eq!(
            canon(r#" { "b" : 1, "a" : [ true , null ] } "#),
            r#"{"a":[true,null],"b":1}"#
        );
    }

    #[test]
    fn sorts_keys_by_utf16_code_units_not_utf8_bytes() {
        // U+1F600 is the surrogate pair D83D DE00 in UTF-16, which sorts before
        // U+FB33 there, but after it in UTF-8 byte order and in code point order.
        let text = "{\"\u{fb33}\":1,\"\u{1f600}\":2}";
        assert_eq!(canon(text), "{\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn rfc8785_key_order_example() {
        let text = r#"{"\u20ac":"Euro Sign","\r":"Carriage Return","\ufb33":"Hebrew Letter Dalet With Dagesh","1":"One","\ud83d\ude00":"Emoji: Grinning Face","\u0080":"Control","\u00f6":"Latin Small Letter O With Diaeresis"}"#;
        let value = parse(text.as_bytes()).unwrap();
        let out = String::from_utf8(to_vec(&value).unwrap()).unwrap();
        let order: Vec<usize> = [
            "Carriage", "One", "Control", "Latin", "Euro", "Emoji", "Hebrew",
        ]
        .iter()
        .map(|needle| out.find(needle).unwrap())
        .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{out}");
    }

    #[test]
    fn escapes_minimally_with_lowercase_hex() {
        let value = Value::String("a\"b\\c\u{8}\t\n\u{c}\r\u{1f}\u{7f}é\u{2028}".to_string());
        let out = String::from_utf8(to_vec(&value).unwrap()).unwrap();
        assert_eq!(out, "\"a\\\"b\\\\c\\b\\t\\n\\f\\r\\u001f\u{7f}é\u{2028}\"");
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        assert_eq!(
            parse(br#"{"a":1,"a":2}"#),
            Err(CanonError::DuplicateKey("a".into()))
        );
        assert_eq!(
            parse(br#"[{"x":{"k":1,"k":1}}]"#),
            Err(CanonError::DuplicateKey("k".into()))
        );
    }

    #[test]
    fn rejects_non_integers_and_out_of_range() {
        assert_eq!(parse(b"1.5"), Err(CanonError::NonInteger));
        assert_eq!(parse(b"1e-3"), Err(CanonError::NonInteger));
        assert_eq!(
            parse(b"9007199254740992"),
            Err(CanonError::IntegerOutOfRange)
        );
        assert_eq!(
            parse(b"-9007199254740992"),
            Err(CanonError::IntegerOutOfRange)
        );
        assert_eq!(
            parse(b"18446744073709551616"),
            Err(CanonError::IntegerOutOfRange)
        );
        assert!(parse(b"9007199254740991").is_ok());
        assert!(parse(b"-9007199254740991").is_ok());
    }

    #[test]
    fn normalizes_negative_zero() {
        assert_eq!(canon("-0"), "0");
        assert_eq!(canon("[-0]"), "[0]");
    }

    #[test]
    fn rejects_trailing_data_and_bad_syntax() {
        assert!(matches!(parse(b"1 2"), Err(CanonError::Syntax(_))));
        assert!(matches!(parse(b"{"), Err(CanonError::Syntax(_))));
        assert!(matches!(parse(b"\"\\ud800\""), Err(CanonError::Syntax(_))));
    }

    #[test]
    fn enforces_depth_limit() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse(ok.as_bytes()).is_ok());
        assert_eq!(parse(deep.as_bytes()), Err(CanonError::TooDeep));
    }

    #[test]
    fn parse_canonical_requires_exact_bytes() {
        assert!(parse_canonical(br#"{"a":1,"b":2}"#).is_ok());
        assert_eq!(
            parse_canonical(br#"{"b":2,"a":1}"#),
            Err(CanonError::NotCanonical)
        );
        assert_eq!(
            parse_canonical(br#"{"a": 1}"#),
            Err(CanonError::NotCanonical)
        );
    }

    #[test]
    fn encode_rejects_floats_from_typed_values() {
        assert_eq!(encode(&1.5f64), Err(CanonError::NonInteger));
        assert_eq!(encode(&u64::MAX), Err(CanonError::IntegerOutOfRange));
    }
}
