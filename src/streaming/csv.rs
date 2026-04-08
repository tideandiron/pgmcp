// src/streaming/csv.rs
//
// CSV row encoder for pgmcp.
//
// Encodes tokio-postgres rows to RFC 4180 CSV bytes. Produces:
//   - A header row (column names) as the first line.
//   - One data row per tokio-postgres Row, with fields separated by commas.
//   - Lines terminated by CRLF (\r\n) per RFC 4180.
//
// # Quoting rules (RFC 4180)
//
// A field is quoted with double-quotes if it contains:
//   - A comma `,`
//   - A double-quote `"`
//   - A carriage return `\r`
//   - A newline `\n`
//
// A double-quote inside a quoted field is escaped as `""`.
//
// # NULL handling
//
// NULL columns produce an empty field (two consecutive commas for interior
// fields, or an empty field at row start/end).
//
// # Type rendering
//
// The CSV encoder renders values as their most natural string representation:
//   - Integers and floats: decimal notation
//   - Booleans: `true` / `false`
//   - Strings: UTF-8 text (quoted if necessary)
//   - UUIDs: hyphenated lowercase hex
//   - Timestamps: ISO 8601 / RFC 3339
//   - JSON/JSONB: raw JSON text
//   - All others: serde_json fallback to string representation

#![allow(dead_code)]

use std::io::Write as _;

use tokio_postgres::Row;

use crate::pg::types::{
    OID_BOOL, OID_BPCHAR, OID_FLOAT4, OID_FLOAT8, OID_INT2, OID_INT4, OID_INT8, OID_JSON,
    OID_JSONB, OID_NAME, OID_NUMERIC, OID_TEXT, OID_TIMESTAMP, OID_TIMESTAMPTZ, OID_UUID,
    OID_VARCHAR,
};

// ── CsvEncoder ────────────────────────────────────────────────────────────────

/// Stateless CSV row encoder.
///
/// Call [`encode_rows`] to encode a header row plus all data rows.
pub struct CsvEncoder;

impl CsvEncoder {
    /// Encode rows to CSV bytes with a header line.
    ///
    /// # Output
    ///
    /// ```text
    /// col1,col2,col3\r\n
    /// val1,val2,val3\r\n
    /// ...
    /// ```
    ///
    /// Returns an empty `Vec<u8>` if `rows` is empty AND there are no columns to
    /// produce a header from. If rows are non-empty, the header is always produced.
    pub fn encode_rows(rows: &[Row]) -> Vec<u8> {
        if rows.is_empty() {
            return Vec::new();
        }
        let mut buf = Vec::with_capacity(rows.len() * 64);

        // Header row.
        let columns = rows[0].columns();
        let mut first = true;
        for col in columns {
            if !first {
                buf.push(b',');
            }
            first = false;
            write_csv_field(col.name(), &mut buf);
        }
        buf.extend_from_slice(b"\r\n");

        // Data rows.
        for row in rows {
            Self::encode_row(row, &mut buf);
        }

        buf
    }

    /// Encode a header line from column names, returning the bytes.
    ///
    /// Useful when the caller wants to produce the header before any rows
    /// are available (streaming pre-header pattern).
    pub fn encode_header(columns: &[tokio_postgres::Column]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(columns.len() * 16);
        let mut first = true;
        for col in columns {
            if !first {
                buf.push(b',');
            }
            first = false;
            write_csv_field(col.name(), &mut buf);
        }
        buf.extend_from_slice(b"\r\n");
        buf
    }

    /// Encode a single data row as CSV, appending to `buf`.
    ///
    /// Does NOT write a CRLF prefix; the line terminator is written at the end.
    pub fn encode_row(row: &Row, buf: &mut Vec<u8>) {
        let columns = row.columns();
        let mut first = true;
        for (i, col) in columns.iter().enumerate() {
            if !first {
                buf.push(b',');
            }
            first = false;
            encode_csv_value(row, i, col.type_().oid(), buf);
        }
        buf.extend_from_slice(b"\r\n");
    }
}

// ── encode_csv_value ──────────────────────────────────────────────────────────

fn encode_csv_value(row: &Row, i: usize, oid: u32, buf: &mut Vec<u8>) {
    match oid {
        OID_BOOL => {
            match row.try_get::<_, Option<bool>>(i) {
                Ok(Some(v)) => buf.extend_from_slice(if v { b"true" } else { b"false" }),
                Ok(None) | Err(_) => {} // empty field
            }
        }

        OID_INT2 => {
            if let Ok(Some(v)) = row.try_get::<_, Option<i16>>(i) {
                let _ = write!(buf, "{v}");
            }
        }

        OID_INT4 => {
            if let Ok(Some(v)) = row.try_get::<_, Option<i32>>(i) {
                let _ = write!(buf, "{v}");
            }
        }

        OID_INT8 => {
            if let Ok(Some(v)) = row.try_get::<_, Option<i64>>(i) {
                let _ = write!(buf, "{v}");
            }
        }

        OID_FLOAT4 => {
            if let Ok(Some(v)) = row.try_get::<_, Option<f32>>(i) {
                if v.is_finite() {
                    let mut tmp = ryu::Buffer::new();
                    buf.extend_from_slice(tmp.format(v).as_bytes());
                }
                // NaN/Inf → empty field
            }
        }

        OID_FLOAT8 => {
            if let Ok(Some(v)) = row.try_get::<_, Option<f64>>(i) {
                if v.is_finite() {
                    let mut tmp = ryu::Buffer::new();
                    buf.extend_from_slice(tmp.format(v).as_bytes());
                }
            }
        }

        OID_TEXT | OID_VARCHAR | OID_NAME | OID_BPCHAR | OID_NUMERIC => {
            if let Ok(Some(s)) = row.try_get::<_, Option<&str>>(i) {
                write_csv_field(s, buf);
            }
        }

        OID_JSON | OID_JSONB => {
            if let Ok(Some(s)) = row.try_get::<_, Option<&str>>(i) {
                write_csv_field(s, buf);
            }
        }

        OID_UUID => {
            if let Ok(Some(u)) = row.try_get::<_, Option<uuid::Uuid>>(i) {
                let s = u.hyphenated().to_string();
                write_csv_field(&s, buf);
            }
        }

        OID_TIMESTAMPTZ => {
            if let Ok(Some(dt)) = row.try_get::<_, Option<time::OffsetDateTime>>(i) {
                if let Ok(s) = dt.format(&time::format_description::well_known::Rfc3339) {
                    write_csv_field(&s, buf);
                }
            }
        }

        OID_TIMESTAMP => {
            if let Ok(Some(dt)) = row.try_get::<_, Option<time::PrimitiveDateTime>>(i) {
                let format = time::format_description::parse(
                    "[year]-[month]-[day]T[hour]:[minute]:[second]",
                )
                .unwrap_or_default();
                if let Ok(s) = dt.format(&format) {
                    write_csv_field(&s, buf);
                }
            }
        }

        _ => {
            // Fallback: serde_json value → string representation.
            if let Ok(Some(v)) = row.try_get::<_, Option<serde_json::Value>>(i) {
                let s = v.to_string();
                write_csv_field(&s, buf);
            }
        }
    }
}

// ── write_csv_field ───────────────────────────────────────────────────────────

/// Write a string as an RFC 4180 CSV field.
///
/// Quotes the field if it contains a comma, double-quote, CR, or LF.
/// Doubles any embedded double-quote characters.
pub fn write_csv_field(s: &str, buf: &mut Vec<u8>) {
    let needs_quoting = s
        .bytes()
        .any(|b| b == b',' || b == b'"' || b == b'\r' || b == b'\n');

    if needs_quoting {
        buf.push(b'"');
        for byte in s.bytes() {
            if byte == b'"' {
                buf.extend_from_slice(b"\"\"");
            } else {
                buf.push(byte);
            }
        }
        buf.push(b'"');
    } else {
        buf.extend_from_slice(s.as_bytes());
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> String {
        let mut buf = Vec::new();
        write_csv_field(s, &mut buf);
        String::from_utf8(buf).unwrap()
    }

    // ── write_csv_field tests ─────────────────────────────────────────────

    #[test]
    fn csv_field_plain_text_no_quoting() {
        assert_eq!(field("hello"), "hello");
    }

    #[test]
    fn csv_field_empty_string_no_quoting() {
        assert_eq!(field(""), "");
    }

    #[test]
    fn csv_field_with_comma_is_quoted() {
        assert_eq!(field("hello, world"), r#""hello, world""#);
    }

    #[test]
    fn csv_field_with_double_quote_is_quoted_and_doubled() {
        assert_eq!(field(r#"say "hi""#), r#""say ""hi""""#);
    }

    #[test]
    fn csv_field_with_newline_is_quoted() {
        let s = "line1\nline2";
        let result = field(s);
        assert!(result.starts_with('"') && result.ends_with('"'));
        assert!(result.contains("line1\nline2"));
    }

    #[test]
    fn csv_field_with_carriage_return_is_quoted() {
        let s = "a\rb";
        let result = field(s);
        assert!(result.starts_with('"') && result.ends_with('"'));
    }

    #[test]
    fn csv_field_numeric_string_no_quoting() {
        assert_eq!(field("12345"), "12345");
    }

    #[test]
    fn csv_field_json_value_is_quoted() {
        // JSON contains double quotes and colons.
        let s = r#"{"key": "value"}"#;
        let result = field(s);
        assert!(
            result.starts_with('"') && result.ends_with('"'),
            "JSON should be quoted: {result}"
        );
        // Double-quotes inside should be doubled.
        assert!(result.contains(r#""""#), "inner quotes should be doubled: {result}");
    }

    #[test]
    fn csv_field_only_double_quote() {
        // Single " → needs quoting → open-quote + escaped-" + close-quote = """"
        // That is 4 double-quote characters.
        let result = field("\"");
        assert_eq!(result, "\"\"\"\"");
    }

    // ── CsvEncoder::encode_rows tests ─────────────────────────────────────

    #[test]
    fn encode_rows_empty_produces_empty_bytes() {
        let rows: Vec<Row> = vec![];
        let encoded = CsvEncoder::encode_rows(&rows);
        assert!(encoded.is_empty(), "empty rows should produce empty CSV");
    }

    // Note: Tests requiring actual Row objects need an integration test
    // with a live database. The unit tests above cover the encoding logic.

    #[test]
    fn csv_field_backslash_no_quoting() {
        // Backslashes don't need quoting in CSV (only RFC 4180 specials do).
        assert_eq!(field(r"C:\path\file"), r"C:\path\file");
    }

    #[test]
    fn csv_field_unicode_no_quoting() {
        assert_eq!(field("こんにちは"), "こんにちは");
    }
}
