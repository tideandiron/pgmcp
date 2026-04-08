// src/tools/query_events.rs
//
// Response construction helpers for the query tool.
//
// This module provides typed structures and helper functions for building the
// JSON responses returned by the `query` tool handler. All response variants
// are serialized to `Content::text(json_string)` at the call site.
//
// Response variants:
//  - `DryRunResponse`   — returned when dry_run: true (no SQL executed)
//  - `QueryResponse`    — returned after a successful query (JSON or CSV data)
//
// Response schema (QueryResponse):
//  {
//    "columns":           [{name, type}, ...],   // column metadata
//    "rows":              [...] | "csv...",       // result data (format-dependent)
//    "row_count":         N,                      // exact number of rows returned
//    "truncated":         bool,                   // true when LIMIT was hit exactly
//    "format":            "json"|"json_compact"|"csv",
//    "sql_executed":      "...",                  // SQL after LIMIT injection
//    "limit_injected":    bool,
//    "execution_time_ms": N.N,
//    "plan":              null | {...}
//  }
//
// For MVP: responses are returned as a single JSON blob in `CallToolResult`.
// True SSE progress events require client-side streaming support; the
// architecture is compatible with adding it later.

use serde_json::Value;

use crate::error::McpError;

// ── OutputFormat ──────────────────────────────────────────────────────────────

/// Requested output format for query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    /// JSON array of objects: `[{"col": val, ...}, ...]`
    Json,
    /// Compact JSON (no extra whitespace, same structure as Json).
    JsonCompact,
    /// RFC 4180 CSV with header row.
    Csv,
}

impl OutputFormat {
    /// Parse a format string from the tool parameter.
    ///
    /// # Errors
    ///
    /// Returns `McpError::param_invalid` for unknown format values.
    pub(crate) fn from_str(s: &str) -> Result<Self, McpError> {
        match s {
            "json" => Ok(Self::Json),
            "json_compact" => Ok(Self::JsonCompact),
            "csv" => Ok(Self::Csv),
            other => Err(McpError::param_invalid(
                "format",
                format!("unknown format '{other}'; must be one of: json, json_compact, csv"),
            )),
        }
    }

    /// Returns the format name as used in response metadata.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::JsonCompact => "json_compact",
            Self::Csv => "csv",
        }
    }
}

// ── ColumnInfo ────────────────────────────────────────────────────────────────

/// Column metadata included in every query response.
#[derive(Debug, Clone)]
pub(crate) struct ColumnInfo {
    /// Column name from the result set.
    pub name: String,
    /// PostgreSQL type name (e.g., "int4", "text", "timestamptz").
    pub pg_type: String,
}

impl ColumnInfo {
    pub(crate) fn new(name: impl Into<String>, pg_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pg_type: pg_type.into(),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "type": self.pg_type,
        })
    }
}

// ── DryRunResponse ────────────────────────────────────────────────────────────

/// Response returned when `dry_run: true`.
///
/// Contains the parsed statement analysis without executing any SQL.
#[derive(Debug)]
pub(crate) struct DryRunResponse {
    /// The statement kind detected by the parser (e.g., "Select", "Insert").
    pub statement_kind: String,
    /// The SQL after LIMIT injection (for SELECT statements).
    pub sql_analyzed: String,
    /// Whether all guardrails passed.
    pub guardrails_passed: bool,
    /// Whether a LIMIT clause was injected.
    pub limit_injected: bool,
    /// The guardrail error message, if any.
    pub guardrail_error: Option<String>,
}

impl DryRunResponse {
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "dry_run": true,
            "statement_kind": self.statement_kind,
            "sql_analyzed": self.sql_analyzed,
            "guardrails_passed": self.guardrails_passed,
            "limit_injected": self.limit_injected,
            "guardrail_error": self.guardrail_error,
            "row_count": null,
        })
    }

    pub(crate) fn to_json_string(&self) -> String {
        self.to_json().to_string()
    }
}

// ── QueryResponse ─────────────────────────────────────────────────────────────

/// Response returned after successful query execution.
#[derive(Debug)]
pub(crate) struct QueryResponse {
    /// Column metadata from the result set.
    pub columns: Vec<ColumnInfo>,
    /// Encoded rows: JSON array bytes (for JSON format) or CSV bytes (for CSV).
    pub rows_bytes: Vec<u8>,
    /// Number of rows in the result set. Always accurate — counts actual rows
    /// returned, not an estimate.
    pub row_count: usize,
    /// True when `row_count == limit`, meaning the result set was cut off by
    /// the LIMIT. The query may have matched more rows than were returned.
    /// An agent should re-query with a larger limit or add WHERE filters to
    /// retrieve the full set.
    pub truncated: bool,
    /// Output format used.
    pub format: OutputFormat,
    /// The SQL actually executed (after LIMIT injection).
    pub sql_executed: String,
    /// Whether a LIMIT was injected.
    pub limit_injected: bool,
    /// End-to-end execution time in milliseconds.
    pub execution_time_ms: f64,
    /// Optional EXPLAIN plan (when `explain: true`).
    pub plan: Option<Value>,
}

impl QueryResponse {
    /// Serialize to a JSON string for the `CallToolResult` content.
    ///
    /// # Output schema
    ///
    /// ```json
    /// {
    ///   "columns":           [{"name": "...", "type": "..."}, ...],
    ///   "rows":              [...] | "csv...",
    ///   "row_count":         N,
    ///   "truncated":         false,
    ///   "format":            "json",
    ///   "sql_executed":      "SELECT ...",
    ///   "limit_injected":    true,
    ///   "execution_time_ms": 1.5,
    ///   "plan":              null
    /// }
    /// ```
    pub(crate) fn to_json_string(&self) -> String {
        let columns: Vec<Value> = self.columns.iter().map(ColumnInfo::to_json).collect();

        let rows_value = match self.format {
            OutputFormat::Json | OutputFormat::JsonCompact => {
                // Parse the raw bytes back as JSON so they embed cleanly.
                // Both json and json_compact embed the same structure; json_compact
                // produces a more compact outer envelope (no extra whitespace).
                serde_json::from_slice(&self.rows_bytes).unwrap_or(Value::Null)
            }
            OutputFormat::Csv => {
                // CSV is returned as a raw string. The column header is the
                // first line; subsequent lines are data rows in RFC 4180 format.
                Value::String(String::from_utf8_lossy(&self.rows_bytes).into_owned())
            }
        };

        let obj = serde_json::json!({
            "columns": columns,
            "rows": rows_value,
            "row_count": self.row_count,
            "truncated": self.truncated,
            "format": self.format.as_str(),
            "sql_executed": self.sql_executed,
            "limit_injected": self.limit_injected,
            "execution_time_ms": self.execution_time_ms,
            "plan": self.plan,
        });

        // json_compact omits extra whitespace in the outer envelope. serde_json's
        // default to_string() already produces compact JSON, so both paths use
        // the same serializer.
        obj.to_string()
    }
}

// ── error_response ────────────────────────────────────────────────────────────

/// Build an error JSON string from an `McpError` for use in the query response.
///
/// The query tool reports errors as JSON objects rather than raw MCP error
/// responses so that agents can parse the error structure programmatically.
/// Retained for future integration into error routing paths.
#[allow(dead_code)]
pub(crate) fn error_json_string(err: &McpError) -> String {
    err.to_json().to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── OutputFormat tests ────────────────────────────────────────────────

    #[test]
    fn output_format_json_from_str() {
        let f = OutputFormat::from_str("json").unwrap();
        assert_eq!(f, OutputFormat::Json);
    }

    #[test]
    fn output_format_json_compact_from_str() {
        let f = OutputFormat::from_str("json_compact").unwrap();
        assert_eq!(f, OutputFormat::JsonCompact);
    }

    #[test]
    fn output_format_csv_from_str() {
        let f = OutputFormat::from_str("csv").unwrap();
        assert_eq!(f, OutputFormat::Csv);
    }

    #[test]
    fn output_format_invalid_returns_error() {
        let err = OutputFormat::from_str("xml").unwrap_err();
        assert_eq!(err.code(), "param_invalid");
        assert!(err.message().contains("xml"));
    }

    #[test]
    fn output_format_as_str_round_trips() {
        for (input, expected) in [
            ("json", "json"),
            ("json_compact", "json_compact"),
            ("csv", "csv"),
        ] {
            let f = OutputFormat::from_str(input).unwrap();
            assert_eq!(f.as_str(), expected);
        }
    }

    // ── ColumnInfo tests ──────────────────────────────────────────────────

    #[test]
    fn column_info_to_json_has_name_and_type() {
        let col = ColumnInfo::new("user_id", "int4");
        let json = col.to_json();
        assert_eq!(json["name"], "user_id");
        assert_eq!(json["type"], "int4");
    }

    // ── DryRunResponse tests ──────────────────────────────────────────────

    #[test]
    fn dry_run_response_to_json_structure() {
        let resp = DryRunResponse {
            statement_kind: "Select".to_string(),
            sql_analyzed: "SELECT * FROM t LIMIT 100".to_string(),
            guardrails_passed: true,
            limit_injected: true,
            guardrail_error: None,
        };
        let json = resp.to_json();
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["statement_kind"], "Select");
        assert_eq!(json["guardrails_passed"], true);
        assert_eq!(json["limit_injected"], true);
        assert!(json["guardrail_error"].is_null());
        assert!(json["row_count"].is_null());
    }

    #[test]
    fn dry_run_response_with_guardrail_error() {
        let resp = DryRunResponse {
            statement_kind: "DropTable".to_string(),
            sql_analyzed: "DROP TABLE users".to_string(),
            guardrails_passed: false,
            limit_injected: false,
            guardrail_error: Some("DDL is blocked".to_string()),
        };
        let json = resp.to_json();
        assert_eq!(json["guardrails_passed"], false);
        assert_eq!(json["guardrail_error"], "DDL is blocked");
    }

    #[test]
    fn dry_run_response_to_json_string_is_valid_json() {
        let resp = DryRunResponse {
            statement_kind: "Select".to_string(),
            sql_analyzed: "SELECT 1".to_string(),
            guardrails_passed: true,
            limit_injected: false,
            guardrail_error: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert!(parsed.is_object());
    }

    // ── QueryResponse tests ───────────────────────────────────────────────

    #[test]
    fn query_response_json_format_embeds_rows_as_array() {
        let resp = QueryResponse {
            columns: vec![ColumnInfo::new("id", "int4")],
            rows_bytes: b"[{\"id\":1},{\"id\":2}]".to_vec(),
            row_count: 2,
            truncated: false,
            format: OutputFormat::Json,
            sql_executed: "SELECT id FROM t LIMIT 100".to_string(),
            limit_injected: true,
            execution_time_ms: 5.2,
            plan: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert!(
            parsed["rows"].is_array(),
            "JSON format rows should be an array"
        );
        assert_eq!(parsed["row_count"], 2);
        assert_eq!(parsed["truncated"], false);
        assert_eq!(parsed["format"], "json");
        assert_eq!(parsed["limit_injected"], true);
        assert!(parsed["plan"].is_null());
    }

    #[test]
    fn query_response_csv_format_returns_string() {
        let resp = QueryResponse {
            columns: vec![ColumnInfo::new("id", "int4")],
            rows_bytes: b"id\r\n1\r\n2\r\n".to_vec(),
            row_count: 2,
            truncated: false,
            format: OutputFormat::Csv,
            sql_executed: "SELECT id FROM t".to_string(),
            limit_injected: false,
            execution_time_ms: 3.1,
            plan: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert!(
            parsed["rows"].is_string(),
            "CSV format rows should be a string"
        );
        assert_eq!(parsed["format"], "csv");
    }

    #[test]
    fn query_response_with_plan_includes_plan_field() {
        let plan_value = serde_json::json!([{"Plan": {"Node Type": "Seq Scan"}}]);
        let resp = QueryResponse {
            columns: vec![],
            rows_bytes: b"[]".to_vec(),
            row_count: 0,
            truncated: false,
            format: OutputFormat::Json,
            sql_executed: "SELECT 1".to_string(),
            limit_injected: false,
            execution_time_ms: 10.0,
            plan: Some(plan_value.clone()),
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert!(!parsed["plan"].is_null(), "plan should be present");
    }

    #[test]
    fn query_response_truncated_true_when_limit_hit() {
        let resp = QueryResponse {
            columns: vec![ColumnInfo::new("n", "int4")],
            rows_bytes: b"[{\"n\":1},{\"n\":2}]".to_vec(),
            row_count: 2,
            truncated: true,
            format: OutputFormat::Json,
            sql_executed: "SELECT n FROM t LIMIT 2".to_string(),
            limit_injected: true,
            execution_time_ms: 1.0,
            plan: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert_eq!(
            parsed["truncated"], true,
            "truncated must be true when limit was hit"
        );
    }

    #[test]
    fn query_response_json_compact_format_is_valid_json() {
        let resp = QueryResponse {
            columns: vec![ColumnInfo::new("id", "int4")],
            rows_bytes: b"[{\"id\":1}]".to_vec(),
            row_count: 1,
            truncated: false,
            format: OutputFormat::JsonCompact,
            sql_executed: "SELECT id FROM t LIMIT 100".to_string(),
            limit_injected: true,
            execution_time_ms: 2.0,
            plan: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert_eq!(parsed["format"], "json_compact");
        assert!(parsed["rows"].is_array());
    }

    #[test]
    fn query_response_all_required_fields_present() {
        let resp = QueryResponse {
            columns: vec![ColumnInfo::new("x", "text")],
            rows_bytes: b"[{\"x\":\"hello\"}]".to_vec(),
            row_count: 1,
            truncated: false,
            format: OutputFormat::Json,
            sql_executed: "SELECT x FROM t".to_string(),
            limit_injected: false,
            execution_time_ms: 0.5,
            plan: None,
        };
        let s = resp.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        // Verify all required fields exist in the response envelope.
        let required_fields = [
            "columns",
            "rows",
            "row_count",
            "truncated",
            "format",
            "sql_executed",
            "limit_injected",
            "execution_time_ms",
            "plan",
        ];
        for field in &required_fields {
            assert!(
                parsed.get(*field).is_some(),
                "response must contain field '{field}'"
            );
        }
    }

    #[test]
    fn error_json_string_wraps_mcp_error() {
        let err = McpError::pg_query_failed("column does not exist");
        let s = error_json_string(&err);
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("must be valid JSON");
        assert_eq!(parsed["code"], "pg_query_failed");
        assert!(parsed["message"].is_string());
        assert!(parsed["hint"].is_string());
    }
}
