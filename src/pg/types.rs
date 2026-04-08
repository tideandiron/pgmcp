// src/pg/types.rs
//
// PostgreSQL OID constants and type-name helpers.
//
// OIDs are stable across all Postgres versions in the supported range (14–17).
// They come from `pg_catalog.pg_type` and are hardcoded here to avoid a
// catalog round-trip in the hot encoding path.
//
// Usage in the streaming encoder:
//   let oid = row.columns()[i].type_().oid();
//   match oid {
//       OID_INT4 => encode_int4(row, i, buf),
//       OID_TEXT => encode_text(row, i, buf),
//       _        => encode_fallback(row, i, buf),
//   }

#![allow(dead_code)]

// ── Boolean ───────────────────────────────────────────────────────────────────
/// `bool`
pub(crate) const OID_BOOL: u32 = 16;

// ── Integer types ─────────────────────────────────────────────────────────────
/// `smallint` / `int2`
pub(crate) const OID_INT2: u32 = 21;
/// `integer` / `int4`
pub(crate) const OID_INT4: u32 = 23;
/// `bigint` / `int8`
pub(crate) const OID_INT8: u32 = 20;

// ── Floating-point types ──────────────────────────────────────────────────────
/// `real` / `float4`
pub(crate) const OID_FLOAT4: u32 = 700;
/// `double precision` / `float8`
pub(crate) const OID_FLOAT8: u32 = 701;

// ── Numeric ───────────────────────────────────────────────────────────────────
/// `numeric` / `decimal`
pub(crate) const OID_NUMERIC: u32 = 1700;

// ── Text types ────────────────────────────────────────────────────────────────
/// `text`
pub(crate) const OID_TEXT: u32 = 25;
/// `character varying` / `varchar`
pub(crate) const OID_VARCHAR: u32 = 1043;
/// `name` (internal catalog string, max 63 bytes)
pub(crate) const OID_NAME: u32 = 19;
/// `character` / `char` / `bpchar`
pub(crate) const OID_BPCHAR: u32 = 1042;

// ── JSON types ────────────────────────────────────────────────────────────────
/// `json`
pub(crate) const OID_JSON: u32 = 114;
/// `jsonb`
pub(crate) const OID_JSONB: u32 = 3802;

// ── UUID ──────────────────────────────────────────────────────────────────────
/// `uuid`
pub(crate) const OID_UUID: u32 = 2950;

// ── Date/time types ───────────────────────────────────────────────────────────
/// `timestamp with time zone`
pub(crate) const OID_TIMESTAMPTZ: u32 = 1184;
/// `timestamp without time zone`
pub(crate) const OID_TIMESTAMP: u32 = 1114;
/// `date`
pub(crate) const OID_DATE: u32 = 1082;
/// `time without time zone`
pub(crate) const OID_TIME: u32 = 1083;
/// `time with time zone`
pub(crate) const OID_TIMETZ: u32 = 1266;
/// `interval`
pub(crate) const OID_INTERVAL: u32 = 1186;

// ── Binary ────────────────────────────────────────────────────────────────────
/// `bytea`
pub(crate) const OID_BYTEA: u32 = 17;

// ── Object identifier types ───────────────────────────────────────────────────
/// `oid`
pub(crate) const OID_OID: u32 = 26;
/// `regclass`
pub(crate) const OID_REGCLASS: u32 = 2205;

// ── Array types (most common) ─────────────────────────────────────────────────
/// `integer[]`
pub(crate) const OID_INT4ARRAY: u32 = 1007;
/// `bigint[]`
pub(crate) const OID_INT8ARRAY: u32 = 1016;
/// `text[]`
pub(crate) const OID_TEXTARRAY: u32 = 1009;
/// `boolean[]`
pub(crate) const OID_BOOLARRAY: u32 = 1000;
/// `double precision[]`
pub(crate) const OID_FLOAT8ARRAY: u32 = 1022;

// ── pg_type_name ──────────────────────────────────────────────────────────────

/// Return a human-readable type name for an OID.
///
/// Used in column metadata returned by the query tool.
/// Returns `"unknown"` for OIDs not in the fast-path table.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pg_type_name(OID_INT4), "int4");
/// assert_eq!(pg_type_name(OID_TEXT), "text");
/// assert_eq!(pg_type_name(9999), "unknown");
/// ```
pub(crate) fn pg_type_name(oid: u32) -> &'static str {
    match oid {
        OID_BOOL => "bool",
        OID_INT2 => "int2",
        OID_INT4 => "int4",
        OID_INT8 => "int8",
        OID_FLOAT4 => "float4",
        OID_FLOAT8 => "float8",
        OID_NUMERIC => "numeric",
        OID_TEXT => "text",
        OID_VARCHAR => "varchar",
        OID_NAME => "name",
        OID_BPCHAR => "bpchar",
        OID_JSON => "json",
        OID_JSONB => "jsonb",
        OID_UUID => "uuid",
        OID_TIMESTAMPTZ => "timestamptz",
        OID_TIMESTAMP => "timestamp",
        OID_DATE => "date",
        OID_TIME => "time",
        OID_TIMETZ => "timetz",
        OID_INTERVAL => "interval",
        OID_BYTEA => "bytea",
        OID_OID => "oid",
        OID_REGCLASS => "regclass",
        OID_INT4ARRAY => "int4[]",
        OID_INT8ARRAY => "int8[]",
        OID_TEXTARRAY => "text[]",
        OID_BOOLARRAY => "bool[]",
        OID_FLOAT8ARRAY => "float8[]",
        _ => "unknown",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_type_name_int4() {
        assert_eq!(pg_type_name(OID_INT4), "int4");
    }

    #[test]
    fn pg_type_name_text() {
        assert_eq!(pg_type_name(OID_TEXT), "text");
    }

    #[test]
    fn pg_type_name_bool() {
        assert_eq!(pg_type_name(OID_BOOL), "bool");
    }

    #[test]
    fn pg_type_name_float8() {
        assert_eq!(pg_type_name(OID_FLOAT8), "float8");
    }

    #[test]
    fn pg_type_name_uuid() {
        assert_eq!(pg_type_name(OID_UUID), "uuid");
    }

    #[test]
    fn pg_type_name_timestamptz() {
        assert_eq!(pg_type_name(OID_TIMESTAMPTZ), "timestamptz");
    }

    #[test]
    fn pg_type_name_jsonb() {
        assert_eq!(pg_type_name(OID_JSONB), "jsonb");
    }

    #[test]
    fn pg_type_name_unknown_oid() {
        assert_eq!(pg_type_name(99999), "unknown");
    }

    #[test]
    fn pg_type_name_int4_array() {
        assert_eq!(pg_type_name(OID_INT4ARRAY), "int4[]");
    }

    #[test]
    fn pg_type_name_all_fast_paths_non_empty() {
        let oids = [
            OID_BOOL,
            OID_INT2,
            OID_INT4,
            OID_INT8,
            OID_FLOAT4,
            OID_FLOAT8,
            OID_NUMERIC,
            OID_TEXT,
            OID_VARCHAR,
            OID_NAME,
            OID_BPCHAR,
            OID_JSON,
            OID_JSONB,
            OID_UUID,
            OID_TIMESTAMPTZ,
            OID_TIMESTAMP,
            OID_DATE,
            OID_TIME,
            OID_TIMETZ,
            OID_INTERVAL,
            OID_BYTEA,
            OID_OID,
            OID_REGCLASS,
            OID_INT4ARRAY,
            OID_INT8ARRAY,
            OID_TEXTARRAY,
            OID_BOOLARRAY,
            OID_FLOAT8ARRAY,
        ];
        for oid in oids {
            let name = pg_type_name(oid);
            assert!(
                !name.is_empty() && name != "unknown",
                "OID {oid} returned empty or 'unknown' name"
            );
        }
    }
}
