// src/tools/table_stats.rs
//
// table_stats tool — returns runtime statistics for a user table.
//
// Cache-first: looks up stats from SchemaCache when populated; falls back to
// a live pg_catalog query when the cache has no entry for the requested table.
//
// Parameters:
//   table  (string, required)  — table name
//   schema (string, optional)  — schema name; defaults to "public"
//
// Queries pg_stat_user_tables (via the view alias s), pg_class (for relation
// sizes), and pg_statio_user_tables (for cache hit ratio). Zero rows returned
// from pg_stat_user_tables means the table does not exist or is not a user
// table → returns McpError::table_not_found.
//
// TIMESTAMPTZ columns (last_vacuum, last_autovacuum, last_analyze,
// last_autoanalyze) are retrieved as Option<time::OffsetDateTime> and formatted
// as RFC 3339 strings, or null when the timestamp is NULL.
//
// Returns a JSON object:
//   {
//     "table":                    string,
//     "schema":                   string,
//     "row_estimate":             i64,
//     "sizes": {
//       "total":                  i64,
//       "table":                  i64,
//       "indexes":                i64,
//       "toast":                  i64
//     },
//     "cache_hit_ratio":          f64,   -- 0.0–1.0
//     "seq_scans":                i64,
//     "idx_scans":                i64,   -- 0 when NULL in catalog
//     "live_tuples":              i64,
//     "dead_tuples":              i64,
//     "last_vacuum":              string | null,  -- RFC 3339
//     "last_autovacuum":          string | null,
//     "last_analyze":             string | null,
//     "last_autoanalyze":         string | null,
//     "modifications_since_analyze": i64
//   }
//
// SQL matches src/pg/queries/table_stats.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, pg::cache::CachedTableStats, server::context::ToolContext};

/// Format an `Option<time::OffsetDateTime>` as an RFC 3339 string, or return
/// `Value::Null` when the timestamp is absent.
///
/// Uses `time::format_description::well_known::Rfc3339` which produces output
/// like `"2024-01-15T10:30:00+00:00"`.
///
/// # Errors
///
/// Returns [`McpError::internal`] if formatting fails (should not happen for
/// valid `OffsetDateTime` values from Postgres).
fn format_timestamp(ts: Option<time::OffsetDateTime>) -> Result<Value, McpError> {
    match ts {
        None => Ok(Value::Null),
        Some(dt) => {
            let s = dt
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|e| McpError::internal(format!("timestamp format error: {e}")))?;
            Ok(Value::String(s))
        }
    }
}

/// Build the JSON response body from a [`CachedTableStats`] entry.
///
/// Used by both the cache-hit path and (if needed) as a shared builder.
fn build_stats_json(stats: &CachedTableStats) -> Result<Value, McpError> {
    // Timestamps in the cache are already formatted as Option<String> (RFC 3339).
    let last_vacuum = stats
        .last_vacuum
        .as_deref()
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    let last_autovacuum = stats
        .last_autovacuum
        .as_deref()
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    let last_analyze = stats
        .last_analyze
        .as_deref()
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    let last_autoanalyze = stats
        .last_autoanalyze
        .as_deref()
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);

    Ok(serde_json::json!({
        "table":  stats.table,
        "schema": stats.schema,
        "row_estimate": stats.row_estimate,
        "sizes": {
            "total":   stats.total_bytes,
            "table":   stats.table_bytes,
            "indexes": stats.index_bytes,
            "toast":   stats.toast_bytes,
        },
        "cache_hit_ratio":             stats.cache_hit_ratio,
        "seq_scans":                   stats.seq_scans,
        "idx_scans":                   stats.idx_scans,
        "live_tuples":                 stats.live_tuples,
        "dead_tuples":                 stats.dead_tuples,
        "last_vacuum":                 last_vacuum,
        "last_autovacuum":             last_autovacuum,
        "last_analyze":                last_analyze,
        "last_autoanalyze":            last_autoanalyze,
        "modifications_since_analyze": stats.modifications_since_analyze,
    }))
}

/// Extract and validate the `table` and `schema` parameters from `args`.
///
/// # Errors
///
/// Returns [`McpError::param_invalid`] when `table` is missing or empty.
fn extract_params(args: Option<&Map<String, Value>>) -> Result<(String, String), McpError> {
    let table = args
        .and_then(|m| m.get("table"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            McpError::param_invalid("table", "required string parameter is missing or empty")
        })?
        .to_string();

    let schema = args
        .and_then(|m| m.get("schema"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("public")
        .to_string();

    Ok((table, schema))
}

/// Handle a `table_stats` tool call.
///
/// Checks the schema cache first for a matching stats entry. On a cache hit,
/// returns cached data without acquiring a connection. On a cache miss,
/// acquires a connection and executes a single query against
/// `pg_stat_user_tables`, `pg_class`, and `pg_statio_user_tables`.
/// Zero rows → [`McpError::table_not_found`].
///
/// # Parameters
///
/// - `table` (required): table name. Missing or empty → `param_invalid`.
/// - `schema` (optional): schema name, defaults to `"public"`.
///
/// # Errors
///
/// - [`McpError::param_invalid`] when `table` is missing or empty.
/// - [`McpError::table_not_found`] when the table is absent from `pg_stat_user_tables`.
/// - [`McpError::pg_pool_timeout`] when a connection cannot be acquired.
/// - [`McpError::pg_query_failed`] when the catalog query fails.
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let (table, schema) = extract_params(args.as_ref())?;

    // Cache-first: look up stats from snapshot.
    if let Some(stats) = ctx.cache.get_table_stats(&schema, &table).await {
        tracing::debug!(schema = %schema, table = %table, "table_stats: cache hit");
        let body = build_stats_json(&stats)?;
        return Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
        )]));
    }

    // Cache miss: fall through to live query.
    tracing::debug!(schema = %schema, table = %table, "table_stats: cache miss, querying pg_catalog");

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/table_stats.sql.
    let rows = client
        .query(
            "SELECT \
                s.relname, \
                s.schemaname, \
                c.reltuples::int8                                         AS row_estimate, \
                pg_total_relation_size(c.oid)                            AS total_size, \
                pg_table_size(c.oid)                                     AS table_size, \
                pg_indexes_size(c.oid)                                   AS indexes_size, \
                COALESCE( \
                    pg_total_relation_size(c.oid) \
                        - pg_table_size(c.oid) \
                        - pg_indexes_size(c.oid), \
                    0 \
                )                                                         AS toast_size, \
                s.seq_scan, \
                s.idx_scan, \
                s.n_live_tup, \
                s.n_dead_tup, \
                s.last_vacuum, \
                s.last_autovacuum, \
                s.last_analyze, \
                s.last_autoanalyze, \
                s.n_mod_since_analyze, \
                COALESCE( \
                    stio.heap_blks_hit::float8 \
                        / NULLIF(stio.heap_blks_hit + stio.heap_blks_read, 0), \
                    0.0 \
                )                                                         AS cache_hit_ratio \
             FROM pg_stat_user_tables s \
             JOIN pg_class c ON c.oid = s.relid \
             LEFT JOIN pg_statio_user_tables stio ON stio.relid = s.relid \
             WHERE s.schemaname = $1 \
               AND s.relname    = $2",
            &[&schema, &table],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection — query is done.
    drop(client);

    if rows.is_empty() {
        return Err(McpError::table_not_found(&schema, &table));
    }

    let row = &rows[0];

    // Column indices (in SELECT order):
    //  0  relname
    //  1  schemaname
    //  2  row_estimate    INT8
    //  3  total_size      INT8
    //  4  table_size      INT8
    //  5  indexes_size    INT8
    //  6  toast_size      INT8
    //  7  seq_scan        INT8
    //  8  idx_scan        Option<INT8>
    //  9  n_live_tup      INT8
    // 10  n_dead_tup      INT8
    // 11  last_vacuum     Option<TIMESTAMPTZ>
    // 12  last_autovacuum Option<TIMESTAMPTZ>
    // 13  last_analyze    Option<TIMESTAMPTZ>
    // 14  last_autoanalyze Option<TIMESTAMPTZ>
    // 15  n_mod_since_analyze INT8
    // 16  cache_hit_ratio FLOAT8

    let row_estimate: i64 = row.get(2);
    let total_size: i64 = row.get(3);
    let table_size: i64 = row.get(4);
    let indexes_size: i64 = row.get(5);
    let toast_size: i64 = row.get(6);
    let seq_scan: i64 = row.get(7);
    let idx_scan: i64 = row.get::<_, Option<i64>>(8).unwrap_or(0);
    let live_tuples: i64 = row.get(9);
    let dead_tuples: i64 = row.get(10);
    let last_vacuum: Option<time::OffsetDateTime> = row.get(11);
    let last_autovacuum: Option<time::OffsetDateTime> = row.get(12);
    let last_analyze: Option<time::OffsetDateTime> = row.get(13);
    let last_autoanalyze: Option<time::OffsetDateTime> = row.get(14);
    let modifications_since_analyze: i64 = row.get(15);
    let cache_hit_ratio: f64 = row.get(16);

    let body = serde_json::json!({
        "table":  table,
        "schema": schema,
        "row_estimate": row_estimate,
        "sizes": {
            "total":   total_size,
            "table":   table_size,
            "indexes": indexes_size,
            "toast":   toast_size,
        },
        "cache_hit_ratio":             cache_hit_ratio,
        "seq_scans":                   seq_scan,
        "idx_scans":                   idx_scan,
        "live_tuples":                 live_tuples,
        "dead_tuples":                 dead_tuples,
        "last_vacuum":                 format_timestamp(last_vacuum)?,
        "last_autovacuum":             format_timestamp(last_autovacuum)?,
        "last_analyze":                format_timestamp(last_analyze)?,
        "last_autoanalyze":            format_timestamp(last_autoanalyze)?,
        "modifications_since_analyze": modifications_since_analyze,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_params_missing_table_returns_error() {
        let result = extract_params(None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "param_invalid");
    }

    #[test]
    fn extract_params_schema_defaults_to_public() {
        let args: Map<String, Value> = serde_json::from_str(r#"{"table":"events"}"#).unwrap();
        let (table, schema) = extract_params(Some(&args)).unwrap();
        assert_eq!(table, "events");
        assert_eq!(schema, "public");
    }

    #[test]
    fn extract_params_explicit_schema() {
        let args: Map<String, Value> =
            serde_json::from_str(r#"{"table":"orders","schema":"analytics"}"#).unwrap();
        let (table, schema) = extract_params(Some(&args)).unwrap();
        assert_eq!(table, "orders");
        assert_eq!(schema, "analytics");
    }

    #[test]
    fn extract_params_empty_table_is_invalid() {
        let args: Map<String, Value> = serde_json::from_str(r#"{"table":""}"#).unwrap();
        let result = extract_params(Some(&args));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "param_invalid");
    }

    #[test]
    fn format_timestamp_none_returns_null() {
        let v = format_timestamp(None).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn format_timestamp_some_returns_rfc3339_string() {
        // UNIX epoch as OffsetDateTime.
        let dt = time::OffsetDateTime::UNIX_EPOCH;
        let v = format_timestamp(Some(dt)).unwrap();
        let s = v.as_str().expect("must be a string");
        // RFC 3339 epoch: "1970-01-01T00:00:00Z"
        assert!(
            s.starts_with("1970-01-01T00:00:00"),
            "expected RFC3339 epoch string, got: {s}"
        );
    }
}
