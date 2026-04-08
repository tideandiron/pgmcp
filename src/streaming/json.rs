// src/streaming/json.rs
//
// JSON row encoder with OID-specific fast paths.
// Items here are consumed by the query tool handler (feat/018).
// Dead-code lint fires until the query tool integrates this layer.
//
// Encodes tokio-postgres rows directly to JSON bytes without routing through
// serde_json::Value (which requires heap allocation for every field). Each
// fast path reads the column value at the native Rust type, formats it
// directly into a pre-allocated Vec<u8> write buffer.
//
// # Output format
//
// A single row is encoded as a JSON object:
//   {"col1": value1, "col2": value2, ...}
//
// A result set is encoded as a JSON array of objects:
//   [{"col1": v1, ...}, {"col1": v2, ...}, ...]
//
// The encoder writes the opening `[`, then one object per row separated by
// `,`, then the closing `]`.
//
// # Null handling
//
// All columns are extracted as Option<T>. A None value (NULL in Postgres)
// is encoded as the literal `null` regardless of the column OID.
//
// # Fallback
//
// For OIDs without a specific fast path, the encoder extracts the value as
// `serde_json::Value` via tokio-postgres's built-in serde support and
// delegates to `serde_json::to_writer`. This allocates but is correct for
// all types.

#![allow(dead_code)]

use tokio_postgres::Row;

use std::io::Write as _;

use crate::pg::types::{
    OID_BOOL, OID_BPCHAR, OID_FLOAT4, OID_FLOAT8, OID_INT2, OID_INT4, OID_INT8, OID_JSON,
    OID_JSONB, OID_NAME, OID_NUMERIC, OID_TEXT, OID_TIMESTAMP, OID_TIMESTAMPTZ, OID_UUID,
    OID_VARCHAR,
};

// ── JsonEncoder ───────────────────────────────────────────────────────────────

/// Stateless JSON row encoder.
///
/// Call [`encode_rows`] to encode a slice of rows into a JSON array.
/// Call [`encode_row`] to encode a single row into a JSON object, appending
/// to an existing buffer.
pub struct JsonEncoder;

impl JsonEncoder {
    /// Encode a slice of rows as a JSON array into a new `Vec<u8>`.
    ///
    /// # Output
    ///
    /// `[{"col": val, ...}, ...]`
    ///
    /// An empty slice produces `[]`.
    pub fn encode_rows(rows: &[Row]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(rows.len() * 64 + 2);
        buf.push(b'[');
        let mut first = true;
        for row in rows {
            if !first {
                buf.push(b',');
            }
            first = false;
            Self::encode_row(row, &mut buf);
        }
        buf.push(b']');
        buf
    }

    /// Encode a single row as a JSON object, appending to `buf`.
    ///
    /// # Output
    ///
    /// `{"col1": value1, "col2": value2}`
    pub fn encode_row(row: &Row, buf: &mut Vec<u8>) {
        buf.push(b'{');
        let columns = row.columns();
        let mut first = true;
        for (i, col) in columns.iter().enumerate() {
            if !first {
                buf.push(b',');
            }
            first = false;

            // Write the column name as a JSON string key.
            write_json_string(col.name(), buf);
            buf.push(b':');

            // Dispatch to the appropriate encoder based on OID.
            let oid = col.type_().oid();
            encode_value(row, i, oid, buf);
        }
        buf.push(b'}');
    }
}

// ── encode_value ──────────────────────────────────────────────────────────────

/// Encode the value at column index `i` of `row` with the given OID.
fn encode_value(row: &Row, i: usize, oid: u32, buf: &mut Vec<u8>) {
    match oid {
        OID_BOOL => match row.try_get::<_, Option<bool>>(i) {
            Ok(Some(v)) => buf.extend_from_slice(if v { b"true" } else { b"false" }),
            Ok(None) => buf.extend_from_slice(b"null"),
            Err(_) => buf.extend_from_slice(b"null"),
        },

        OID_INT2 => match row.try_get::<_, Option<i16>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{v}");
            }
            Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
        },

        OID_INT4 => match row.try_get::<_, Option<i32>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{v}");
            }
            Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
        },

        OID_INT8 => match row.try_get::<_, Option<i64>>(i) {
            Ok(Some(v)) => {
                let _ = write!(buf, "{v}");
            }
            Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
        },

        OID_FLOAT4 => {
            match row.try_get::<_, Option<f32>>(i) {
                Ok(Some(v)) => {
                    if v.is_finite() {
                        let mut tmp = ryu::Buffer::new();
                        buf.extend_from_slice(tmp.format(v).as_bytes());
                    } else if v.is_nan() {
                        buf.extend_from_slice(b"null"); // JSON has no NaN
                    } else if v.is_infinite() {
                        buf.extend_from_slice(b"null"); // JSON has no ±Infinity; fall back to null
                    }
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_FLOAT8 => {
            match row.try_get::<_, Option<f64>>(i) {
                Ok(Some(v)) => {
                    if v.is_finite() {
                        let mut tmp = ryu::Buffer::new();
                        buf.extend_from_slice(tmp.format(v).as_bytes());
                    } else {
                        // JSON has no NaN / ±Infinity; use null.
                        buf.extend_from_slice(b"null");
                    }
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_TEXT | OID_VARCHAR | OID_NAME | OID_BPCHAR | OID_NUMERIC => {
            match row.try_get::<_, Option<&str>>(i) {
                Ok(Some(s)) => write_json_string(s, buf),
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_JSON | OID_JSONB => {
            // JSON/JSONB values are already valid JSON; copy them raw.
            // tokio-postgres returns JSONB with a leading version byte stripped.
            match row.try_get::<_, Option<&str>>(i) {
                Ok(Some(s)) => buf.extend_from_slice(s.as_bytes()),
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_UUID => {
            match row.try_get::<_, Option<uuid::Uuid>>(i) {
                Ok(Some(u)) => {
                    buf.push(b'"');
                    // Hyphenated format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
                    let s = u.hyphenated().to_string();
                    buf.extend_from_slice(s.as_bytes());
                    buf.push(b'"');
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_TIMESTAMPTZ => {
            match row.try_get::<_, Option<time::OffsetDateTime>>(i) {
                Ok(Some(dt)) => {
                    buf.push(b'"');
                    // RFC 3339 format: 2024-01-15T10:30:00Z
                    if let Ok(s) = dt.format(&time::format_description::well_known::Rfc3339) {
                        buf.extend_from_slice(s.as_bytes());
                    }
                    buf.push(b'"');
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        OID_TIMESTAMP => {
            match row.try_get::<_, Option<time::PrimitiveDateTime>>(i) {
                Ok(Some(dt)) => {
                    buf.push(b'"');
                    // ISO 8601 without timezone.
                    let format = time::format_description::parse(
                        "[year]-[month]-[day]T[hour]:[minute]:[second]",
                    )
                    .unwrap_or_default();
                    if let Ok(s) = dt.format(&format) {
                        buf.extend_from_slice(s.as_bytes());
                    }
                    buf.push(b'"');
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }

        _ => {
            // Fallback: use serde_json via the tokio-postgres serde integration.
            match row.try_get::<_, Option<serde_json::Value>>(i) {
                Ok(Some(v)) => {
                    if let Ok(s) = serde_json::to_string(&v) {
                        buf.extend_from_slice(s.as_bytes());
                    } else {
                        buf.extend_from_slice(b"null");
                    }
                }
                Ok(None) | Err(_) => buf.extend_from_slice(b"null"),
            }
        }
    }
}

// ── write_json_string ─────────────────────────────────────────────────────────

/// Write a Rust string as a JSON string literal into `buf`.
///
/// Applies RFC 7159 escape sequences for:
/// - `"` → `\"`
/// - `\` → `\\`
/// - Control characters (0x00–0x1F) → `\uXXXX`
///
/// All other characters are copied verbatim (UTF-8).
pub fn write_json_string(s: &str, buf: &mut Vec<u8>) {
    buf.push(b'"');
    for byte in s.bytes() {
        match byte {
            b'"' => buf.extend_from_slice(b"\\\""),
            b'\\' => buf.extend_from_slice(b"\\\\"),
            b'\n' => buf.extend_from_slice(b"\\n"),
            b'\r' => buf.extend_from_slice(b"\\r"),
            b'\t' => buf.extend_from_slice(b"\\t"),
            0x08 => buf.extend_from_slice(b"\\b"),
            0x0C => buf.extend_from_slice(b"\\f"),
            0x00..=0x1F => {
                // Other control characters → \u00XX
                buf.extend_from_slice(b"\\u00");
                let hi = (byte >> 4) & 0x0F;
                let lo = byte & 0x0F;
                buf.push(HEX_DIGITS[hi as usize]);
                buf.push(HEX_DIGITS[lo as usize]);
            }
            b => buf.push(b),
        }
    }
    buf.push(b'"');
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── write_json_string tests ───────────────────────────────────────────

    fn json_str(s: &str) -> String {
        let mut buf = Vec::new();
        write_json_string(s, &mut buf);
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn json_string_plain_text() {
        assert_eq!(json_str("hello"), r#""hello""#);
    }

    #[test]
    fn json_string_empty() {
        assert_eq!(json_str(""), r#""""#);
    }

    #[test]
    fn json_string_double_quote() {
        assert_eq!(json_str(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[test]
    fn json_string_backslash() {
        assert_eq!(json_str(r"C:\path"), r#""C:\\path""#);
    }

    #[test]
    fn json_string_newline() {
        assert_eq!(json_str("line1\nline2"), r#""line1\nline2""#);
    }

    #[test]
    fn json_string_tab() {
        assert_eq!(json_str("a\tb"), r#""a\tb""#);
    }

    #[test]
    fn json_string_carriage_return() {
        assert_eq!(json_str("a\rb"), r#""a\rb""#);
    }

    #[test]
    fn json_string_control_char_unit_separator() {
        // 0x1F = unit separator — must be \u001f
        let s = "\x1f";
        let result = json_str(s);
        assert!(
            result.contains("\\u001f"),
            "expected \\u001f, got: {result}"
        );
    }

    #[test]
    fn json_string_unicode_passthrough() {
        // Non-ASCII UTF-8 should pass through without escaping.
        assert_eq!(json_str("こんにちは"), r#""こんにちは""#);
    }

    // ── JsonEncoder::encode_rows tests ────────────────────────────────────

    #[test]
    fn encode_rows_empty_slice_produces_empty_array() {
        let rows: Vec<Row> = vec![];
        let encoded = JsonEncoder::encode_rows(&rows);
        assert_eq!(encoded, b"[]");
    }

    // Note: Tests that require actual tokio-postgres Row objects need a live
    // database connection. Those are covered in integration tests.
    // The unit tests below verify the logic that does NOT require a Row.

    #[test]
    fn write_json_string_null_byte_escapes() {
        // Null byte (0x00) must be escaped.
        let s = "\x00";
        let result = json_str(s);
        assert!(
            result.contains("\\u0000"),
            "null byte must escape to \\u0000, got: {result}"
        );
    }

    #[test]
    fn write_json_string_backspace_escapes() {
        let s = "\x08";
        let result = json_str(s);
        assert_eq!(result, r#""\b""#);
    }

    #[test]
    fn write_json_string_form_feed_escapes() {
        let s = "\x0C";
        let result = json_str(s);
        assert_eq!(result, r#""\f""#);
    }

    #[test]
    fn write_json_string_mixed_escaping() {
        let s = "He said \"hello\"\nWorld";
        let result = json_str(s);
        assert_eq!(result, r#""He said \"hello\"\nWorld""#);
    }

    #[test]
    fn write_json_string_all_basic_ascii_safe_chars_pass_through() {
        // Printable ASCII from 0x20 to 0x7E (except " and \) should pass through.
        let safe: String = (0x20u8..=0x7Eu8)
            .filter(|&b| b != b'"' && b != b'\\')
            .map(|b| b as char)
            .collect();
        let mut buf = Vec::new();
        write_json_string(&safe, &mut buf);
        let result = String::from_utf8(buf).unwrap();
        // The result is the string wrapped in quotes, with no extra escaping.
        assert!(result.starts_with('"') && result.ends_with('"'));
        let inner = &result[1..result.len() - 1];
        assert_eq!(inner, safe.as_str());
    }
}
