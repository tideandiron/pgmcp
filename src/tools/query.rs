// src/tools/query.rs
//
// The primary query tool — executes SQL and returns results.
//
// Execution pipeline:
//   1. Extract and validate parameters
//   2. Parse SQL via sql/parser.rs → ParsedStatement
//   3. Run guardrails via sql/guardrails.rs
//   4. For SELECT: inject LIMIT via sql/limit.rs
//   5. If dry_run: return parse+guardrail analysis, no execution
//   6. If explain: prepend EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)
//   7. Acquire connection from pool
//   8. Set statement_timeout via SET LOCAL
//   9. If transaction: BEGIN
//  10. Execute query, collect rows
//  11. Serialize rows through json/csv encoder using BatchSizer
//  12. If transaction: ROLLBACK
//  13. Release connection
//  14. Return CallToolResult with metadata
//
// Design invariants:
// - No SQL executes without passing through SQL Analysis + Guardrails (step 2–3).
// - Pool connections are not held across uncontrolled await points.
// - All PG errors are wrapped as McpError before propagating.
// - dry_run never executes the user's SQL.

use std::time::{Duration, Instant};

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{
    error::McpError,
    pg::types::pg_type_name,
    server::context::ToolContext,
    sql::{guardrails::GuardrailConfig, limit::inject_limit, parser::StatementKind},
    streaming::{BatchSizer, csv::CsvEncoder, json::JsonEncoder},
    tools::query_events::{ColumnInfo, DryRunResponse, OutputFormat, QueryResponse},
};

// ── QueryParams ───────────────────────────────────────────────────────────────

/// Parsed and validated query tool parameters.
#[derive(Debug)]
struct QueryParams {
    /// The SQL statement to execute.
    sql: String,
    /// Intent string for logging (not used for query modification).
    intent: String,
    /// Maximum rows to return (injected as LIMIT if absent).
    limit: u32,
    /// Statement timeout in seconds.
    timeout_seconds: u64,
    /// Output format.
    format: OutputFormat,
    /// If true, wrap in BEGIN/ROLLBACK.
    transaction: bool,
    /// If true, analyze without executing.
    dry_run: bool,
    /// If true, prepend EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON).
    explain: bool,
}

/// Default limit when none is specified.
const DEFAULT_LIMIT: u32 = 100;
/// Maximum allowed limit.
const MAX_LIMIT: u32 = 10_000;
/// Default statement timeout in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

impl QueryParams {
    /// Extract and validate parameters from the MCP tool call arguments.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::param_invalid`] if `sql` is missing or empty, if
    /// `format` is unrecognized, or if `limit` is out of range.
    fn from_args(args: Option<&Map<String, serde_json::Value>>) -> Result<Self, McpError> {
        let args =
            args.ok_or_else(|| McpError::param_invalid("sql", "sql parameter is required"))?;

        // sql (required)
        let sql = args.get("sql").and_then(|v| v.as_str()).ok_or_else(|| {
            McpError::param_invalid("sql", "sql parameter is required and must be a string")
        })?;
        let sql = sql.trim().to_string();
        if sql.is_empty() {
            return Err(McpError::param_invalid("sql", "sql must not be empty"));
        }

        // intent (optional, logging only)
        let intent = args
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // limit (optional, default 100)
        let limit = match args.get("limit") {
            None => DEFAULT_LIMIT,
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| {
                    McpError::param_invalid("limit", "limit must be a positive integer")
                })?;
                if n == 0 {
                    return Err(McpError::param_invalid(
                        "limit",
                        format!("limit must be at least 1; got {n}"),
                    ));
                }
                if n > u64::from(MAX_LIMIT) {
                    return Err(McpError::param_invalid(
                        "limit",
                        format!("limit must not exceed {MAX_LIMIT}; got {n}"),
                    ));
                }
                n as u32
            }
        };

        // timeout_seconds (optional, default 30)
        let timeout_seconds = match args.get("timeout_seconds") {
            None => DEFAULT_TIMEOUT_SECONDS,
            Some(v) => {
                let n = v
                    .as_f64()
                    .map(|f| f as u64)
                    .or_else(|| v.as_u64())
                    .ok_or_else(|| {
                        McpError::param_invalid(
                            "timeout_seconds",
                            "timeout_seconds must be a positive number",
                        )
                    })?;
                n.max(1) // floor at 1 second
            }
        };

        // format (optional, default "json")
        let format = match args.get("format") {
            None => OutputFormat::Json,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| McpError::param_invalid("format", "format must be a string"))?;
                OutputFormat::from_str(s)?
            }
        };

        // transaction (optional, default false)
        let transaction = args
            .get("transaction")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // dry_run (optional, default false)
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // explain (optional, default false)
        let explain = args
            .get("explain")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(QueryParams {
            sql,
            intent,
            limit,
            timeout_seconds,
            format,
            transaction,
            dry_run,
            explain,
        })
    }
}

// ── handle ────────────────────────────────────────────────────────────────────

/// Handle a `query` tool call.
///
/// This is the primary data-access tool in pgmcp. It accepts a single SQL
/// statement, runs it through the SQL analysis + guardrail pipeline, and
/// returns the result rows in the requested format.
///
/// # Errors
///
/// - [`McpError::param_invalid`] — missing or invalid parameters
/// - [`McpError::sql_parse_error`] — SQL that cannot be parsed
/// - [`McpError::guardrail_violation`] — SQL blocked by policy
/// - [`McpError::pg_pool_timeout`] — connection acquisition timeout
/// - [`McpError::pg_query_failed`] — SQL execution error from Postgres
/// - [`McpError::internal`] — unexpected serialization errors
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    // Step 1: Extract and validate parameters.
    let params = QueryParams::from_args(args.as_ref())?;

    tracing::debug!(
        sql = %params.sql,
        intent = %params.intent,
        limit = params.limit,
        dry_run = params.dry_run,
        explain = params.explain,
        "query tool invoked"
    );

    // Step 2: Parse SQL.
    let parsed = crate::sql::parser::parse_statement(&params.sql)?;

    // Step 3: Run guardrails.
    let guardrail_config = GuardrailConfig {
        block_ddl: ctx.config.guardrails.block_ddl,
        block_copy_program: ctx.config.guardrails.block_copy_program,
        block_session_set: ctx.config.guardrails.block_session_set,
    };

    let guardrail_result = crate::sql::guardrails::check(&parsed, &guardrail_config);

    // Step 4: For SELECT, inject LIMIT.
    let (sql_after_limit, limit_injected) =
        if parsed.kind == StatementKind::Select && guardrail_result.is_ok() {
            match inject_limit(&params.sql, params.limit, MAX_LIMIT) {
                Ok(pair) => pair,
                Err(e) => {
                    // parse error during limit injection — unexpected, but handle it
                    return Err(e);
                }
            }
        } else {
            (params.sql.clone(), false)
        };

    // Step 5: dry_run — return analysis without executing.
    if params.dry_run {
        let statement_kind = format!("{:?}", parsed.kind);
        let (guardrails_passed, guardrail_error) = match &guardrail_result {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.message().to_string())),
        };

        let resp = DryRunResponse {
            statement_kind,
            sql_analyzed: sql_after_limit,
            guardrails_passed,
            limit_injected,
            guardrail_error,
        };
        return Ok(CallToolResult::success(vec![Content::text(
            resp.to_json_string(),
        )]));
    }

    // Propagate guardrail error now that dry_run is handled.
    guardrail_result?;

    // Step 6: If explain=true, prepend EXPLAIN clause.
    let final_sql = if params.explain {
        format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql_after_limit}")
    } else {
        sql_after_limit.clone()
    };

    // Step 7: Acquire connection from pool.
    let acquire_timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(acquire_timeout).await?;

    let start = Instant::now();

    // Step 8: Set statement_timeout.
    let timeout_ms = params.timeout_seconds * 1000;
    client
        .execute(
            &format!("SET LOCAL statement_timeout = '{timeout_ms}ms'"),
            &[],
        )
        .await
        .map_err(McpError::from)?;

    // Step 9: If transaction=true, begin an explicit transaction.
    if params.transaction {
        client.execute("BEGIN", &[]).await.map_err(McpError::from)?;
    }

    // Step 10: Execute query and collect rows.
    // Note: execute_with_rollback handles both the execute and the ROLLBACK.
    let (rows, plan_value) =
        execute_query(&client, &final_sql, params.explain, params.transaction).await?;

    let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Step 11: Serialize rows.
    let columns: Vec<ColumnInfo> = if rows.is_empty() {
        vec![]
    } else {
        rows[0]
            .columns()
            .iter()
            .map(|c| ColumnInfo::new(c.name(), pg_type_name(c.type_().oid())))
            .collect()
    };

    let row_count = rows.len();

    let rows_bytes = match params.format {
        OutputFormat::Json | OutputFormat::JsonCompact => JsonEncoder::encode_rows(&rows),
        OutputFormat::Csv => CsvEncoder::encode_rows(&rows),
    };

    // BatchSizer is used to report the batch metrics.
    let mut sizer = BatchSizer::new();
    sizer.record(row_count, rows_bytes.len());

    // Step 14: Build and return the response.
    // `truncated` is true when the result set hit the injected LIMIT exactly,
    // indicating that more rows may exist beyond what was returned.
    let truncated = limit_injected && row_count == params.limit as usize;
    let resp = QueryResponse {
        columns,
        rows_bytes,
        row_count,
        truncated,
        format: params.format,
        sql_executed: sql_after_limit,
        limit_injected,
        execution_time_ms,
        plan: plan_value,
    };

    Ok(CallToolResult::success(vec![Content::text(
        resp.to_json_string(),
    )]))
}

// ── execute_query ─────────────────────────────────────────────────────────────

/// Execute the final SQL and return collected rows plus an optional EXPLAIN plan.
///
/// Handles:
/// - Normal query execution
/// - EXPLAIN extraction (the plan is embedded in the EXPLAIN rows)
/// - ROLLBACK after transaction wrapping
///
/// # Errors
///
/// Returns `McpError::pg_query_failed` on any Postgres execution error.
async fn execute_query(
    client: &deadpool_postgres::Client,
    sql: &str,
    is_explain: bool,
    is_transaction: bool,
) -> Result<(Vec<tokio_postgres::Row>, Option<serde_json::Value>), McpError> {
    // Execute the query.
    let rows = client.query(sql, &[]).await.map_err(McpError::from)?;

    // Roll back the transaction if requested (after successful execution).
    if is_transaction {
        // Best-effort ROLLBACK; ignore errors (connection will be recycled).
        let _ = client.execute("ROLLBACK", &[]).await;
    }

    // Extract EXPLAIN plan if this was an EXPLAIN query.
    if is_explain {
        let plan = extract_explain_plan(&rows);
        return Ok((vec![], plan));
    }

    Ok((rows, None))
}

// ── extract_explain_plan ──────────────────────────────────────────────────────

/// Extract the JSON plan from EXPLAIN (FORMAT JSON) rows.
///
/// Postgres returns EXPLAIN JSON as a single row with one column containing
/// the plan as a JSON string.
fn extract_explain_plan(rows: &[tokio_postgres::Row]) -> Option<serde_json::Value> {
    let row = rows.first()?;
    // EXPLAIN FORMAT JSON returns a text column with the JSON plan.
    let plan_str: &str = row.try_get(0).ok()?;
    serde_json::from_str(plan_str).ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(json: &str) -> Option<Map<String, serde_json::Value>> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.as_object().cloned()
    }

    // ── Parameter extraction tests ────────────────────────────────────────

    #[test]
    fn extract_params_sql_required_when_missing() {
        let err = QueryParams::from_args(None).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn extract_params_sql_required_when_empty_args() {
        let args = make_args("{}");
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
        assert!(err.message().contains("sql"));
    }

    #[test]
    fn extract_params_sql_required_when_empty_string() {
        let args = make_args(r#"{"sql": ""}"#);
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn extract_params_sql_required_whitespace_only() {
        let args = make_args(r#"{"sql": "   "}"#);
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn extract_params_defaults() {
        let args = make_args(r#"{"sql": "SELECT 1"}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.sql, "SELECT 1");
        assert_eq!(p.limit, DEFAULT_LIMIT);
        assert_eq!(p.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
        assert_eq!(p.format, OutputFormat::Json);
        assert!(!p.transaction);
        assert!(!p.dry_run);
        assert!(!p.explain);
        assert_eq!(p.intent, "");
    }

    #[test]
    fn extract_params_limit_max_exceeded() {
        let args = make_args(r#"{"sql": "SELECT 1", "limit": 10001}"#);
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
        assert!(err.message().contains("limit"));
    }

    #[test]
    fn extract_params_limit_zero_is_invalid() {
        let args = make_args(r#"{"sql": "SELECT 1", "limit": 0}"#);
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn extract_params_limit_max_is_valid() {
        let args = make_args(r#"{"sql": "SELECT 1", "limit": 10000}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.limit, MAX_LIMIT);
    }

    #[test]
    fn extract_params_limit_1_is_valid() {
        let args = make_args(r#"{"sql": "SELECT 1", "limit": 1}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.limit, 1);
    }

    #[test]
    fn extract_params_format_csv() {
        let args = make_args(r#"{"sql": "SELECT 1", "format": "csv"}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.format, OutputFormat::Csv);
    }

    #[test]
    fn extract_params_format_json_compact() {
        let args = make_args(r#"{"sql": "SELECT 1", "format": "json_compact"}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.format, OutputFormat::JsonCompact);
    }

    #[test]
    fn extract_params_format_invalid() {
        let args = make_args(r#"{"sql": "SELECT 1", "format": "xml"}"#);
        let err = QueryParams::from_args(args.as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
        assert!(err.message().contains("format"));
    }

    #[test]
    fn extract_params_timeout_default() {
        let args = make_args(r#"{"sql": "SELECT 1"}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn extract_params_timeout_custom() {
        let args = make_args(r#"{"sql": "SELECT 1", "timeout_seconds": 60}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.timeout_seconds, 60);
    }

    #[test]
    fn extract_params_dry_run_true() {
        let args = make_args(r#"{"sql": "SELECT 1", "dry_run": true}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert!(p.dry_run);
    }

    #[test]
    fn extract_params_explain_true() {
        let args = make_args(r#"{"sql": "SELECT 1", "explain": true}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert!(p.explain);
    }

    #[test]
    fn extract_params_transaction_true() {
        let args = make_args(r#"{"sql": "SELECT 1", "transaction": true}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert!(p.transaction);
    }

    #[test]
    fn extract_params_intent_is_captured() {
        let args = make_args(r#"{"sql": "SELECT 1", "intent": "list all users for audit"}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.intent, "list all users for audit");
    }

    #[test]
    fn extract_params_sql_is_trimmed() {
        let args = make_args(r#"{"sql": "  SELECT 1  "}"#);
        let p = QueryParams::from_args(args.as_ref()).unwrap();
        assert_eq!(p.sql, "SELECT 1");
    }

    // ── extract_explain_plan tests ────────────────────────────────────────

    #[test]
    fn extract_plan_from_empty_rows_returns_none() {
        let rows: Vec<tokio_postgres::Row> = vec![];
        let plan = extract_explain_plan(&rows);
        assert!(plan.is_none());
    }

    // Full integration tests for the handle() function require a live DB
    // and are in tests/query_tool.rs.
}
