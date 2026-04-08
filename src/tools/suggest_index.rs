// src/tools/suggest_index.rs
//
// suggest_index tool — analyzes a SQL query and proposes indexes.
//
// Parameters:
//   sql    (string, required)       — SQL to analyze; must be SELECT
//   schema (string, default "public") — default schema for unqualified tables
//
// Pipeline:
//   1. Parse + guardrail the SQL (SELECT only; DDL/DML blocked)
//   2. Run EXPLAIN (FORMAT JSON) — estimation only, no ANALYZE
//   3. Walk the plan tree depth-first
//   4. Collect Seq Scan nodes on tables with large row estimates and filter conditions
//   5. For each candidate, check existing indexes on the table
//   6. Generate CREATE INDEX CONCURRENTLY suggestions
//   7. Estimate index size from row count and average width
//
// Design invariants:
// - EXPLAIN is always run WITHOUT ANALYZE to avoid side effects.
// - Uses a single connection for EXPLAIN + catalog lookups.
// - Pure plan-walking logic is in separate functions for testability.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{
    error::McpError,
    server::context::ToolContext,
    sql::{
        guardrails::GuardrailConfig,
        parser::{StatementKind, parse_statement},
    },
    tools::explain::extract_plan_json_from_rows,
};

// ── Parameters ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct SuggestParams {
    sql: String,
    schema: String,
}

impl SuggestParams {
    fn from_args(args: Option<&Map<String, Value>>) -> Result<Self, McpError> {
        let args =
            args.ok_or_else(|| McpError::param_invalid("sql", "sql parameter is required"))?;

        let sql = args
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::param_invalid("sql", "sql is required and must be a string"))?
            .trim()
            .to_string();

        if sql.is_empty() {
            return Err(McpError::param_invalid("sql", "sql must not be empty"));
        }

        let schema = args
            .get("schema")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("public")
            .to_string();

        Ok(SuggestParams { sql, schema })
    }
}

// ── handle ────────────────────────────────────────────────────────────────────

/// Handle a `suggest_index` tool call.
///
/// # Errors
///
/// - [`McpError::param_invalid`] — missing or invalid parameters
/// - [`McpError::sql_parse_error`] — SQL that cannot be parsed
/// - [`McpError::guardrail_violation`] — SQL blocked (only SELECT allowed here)
/// - [`McpError::pg_pool_timeout`] — connection acquisition timeout
/// - [`McpError::pg_query_failed`] — EXPLAIN or catalog query error
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let params = SuggestParams::from_args(args.as_ref())?;

    tracing::debug!(sql = %params.sql, schema = %params.schema, "suggest_index tool invoked");

    // Parse and guardrail; additionally require SELECT.
    let parsed = parse_statement(&params.sql)?;

    let guardrail_config = GuardrailConfig {
        block_ddl: ctx.config.guardrails.block_ddl,
        block_copy_program: ctx.config.guardrails.block_copy_program,
        block_session_set: ctx.config.guardrails.block_session_set,
    };
    crate::sql::guardrails::check(&parsed, &guardrail_config)?;

    // Restrict to SELECT only.
    if parsed.kind != StatementKind::Select {
        return Err(McpError::param_invalid(
            "sql",
            "suggest_index only supports SELECT statements",
        ));
    }

    let acquire_timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(acquire_timeout).await?;

    // Run EXPLAIN (FORMAT JSON) — no ANALYZE to avoid side effects.
    let explain_sql = format!("EXPLAIN (FORMAT JSON) {}", params.sql);
    let explain_rows = client
        .query(&explain_sql, &[])
        .await
        .map_err(McpError::from)?;

    let plan_json = extract_plan_json_from_rows(&explain_rows)?;

    // Extract top-level cost.
    let total_cost = plan_json
        .get(0)
        .and_then(|p| p.get("Plan"))
        .and_then(|p| p.get("Total Cost"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // Find Seq Scan candidates.
    let candidates = collect_seq_scan_candidates(&plan_json);

    // For each candidate, check existing indexes and build suggestions.
    let mut suggestions: Vec<Value> = Vec::new();

    for candidate in &candidates {
        // Check existing indexes on this table.
        let has_suitable_index = check_existing_index(
            &client,
            &candidate.table_name,
            &params.schema,
            &candidate.filter_columns,
        )
        .await;

        if !has_suitable_index {
            let create_sql = generate_create_index_sql(
                &candidate.table_name,
                &params.schema,
                &candidate.filter_columns,
            );
            let estimated_size = estimate_index_size(candidate.estimated_rows, candidate.avg_width);
            let impact = classify_impact(candidate.estimated_rows, total_cost);

            suggestions.push(serde_json::json!({
                "table": candidate.table_name,
                "schema": params.schema,
                "reason": format!(
                    "Sequential scan on '{}' (estimated {} rows){}",
                    candidate.table_name,
                    candidate.estimated_rows,
                    if candidate.filter_columns.is_empty() {
                        String::new()
                    } else {
                        format!(" with filter on {}", candidate.filter_columns.join(", "))
                    }
                ),
                "create_sql": create_sql,
                "estimated_index_size_bytes": estimated_size,
                "impact": impact,
                "tradeoffs": format!(
                    "Speeds up queries filtering on {} at the cost of slightly \
                     slower INSERTs and UPDATEs, and {} bytes of additional storage.",
                    if candidate.filter_columns.is_empty() {
                        "this table".to_string()
                    } else {
                        candidate.filter_columns.join(", ")
                    },
                    estimated_size
                ),
            }));
        }
    }

    drop(client);

    let body = serde_json::json!({
        "sql_analyzed": params.sql,
        "current_plan_cost": total_cost,
        "seq_scans_found": candidates.len(),
        "suggestions": suggestions,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

// ── SeqScanCandidate ──────────────────────────────────────────────────────────

/// A Seq Scan node that is a candidate for index creation.
#[derive(Debug)]
pub(crate) struct SeqScanCandidate {
    pub table_name: String,
    pub estimated_rows: i64,
    pub avg_width: i64,
    /// Column names extracted from the filter expression (best-effort).
    pub filter_columns: Vec<String>,
}

// ── collect_seq_scan_candidates ───────────────────────────────────────────────

/// Walk an EXPLAIN JSON plan tree and collect all Seq Scan nodes that are
/// candidates for index suggestions.
///
/// A node is a candidate when:
/// - Node Type is "Seq Scan"
/// - Plan Rows > 1,000 (large enough to benefit from an index)
/// - Relation Name is not empty (actual table, not a subquery result)
///
/// This function is pure (no I/O) and is separated for unit testability.
pub(crate) fn collect_seq_scan_candidates(plan_json: &Value) -> Vec<SeqScanCandidate> {
    let mut candidates = Vec::new();
    if let Some(arr) = plan_json.as_array()
        && let Some(first) = arr.first()
        && let Some(plan) = first.get("Plan")
    {
        walk_for_seq_scans(plan, &mut candidates);
    }
    candidates
}

fn walk_for_seq_scans(node: &Value, candidates: &mut Vec<SeqScanCandidate>) {
    let node_type = node.get("Node Type").and_then(|v| v.as_str()).unwrap_or("");
    let relation_name = node
        .get("Relation Name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let estimated_rows = node.get("Plan Rows").and_then(|v| v.as_i64()).unwrap_or(0);
    let avg_width = node.get("Plan Width").and_then(|v| v.as_i64()).unwrap_or(8);
    let filter = node.get("Filter").and_then(|v| v.as_str()).unwrap_or("");

    if node_type == "Seq Scan" && !relation_name.is_empty() && estimated_rows > 1_000 {
        let filter_columns = extract_filter_columns(filter);
        candidates.push(SeqScanCandidate {
            table_name: relation_name.to_string(),
            estimated_rows,
            avg_width,
            filter_columns,
        });
    }

    // Recurse into child plans.
    if let Some(children) = node.get("Plans").and_then(|v| v.as_array()) {
        for child in children {
            walk_for_seq_scans(child, candidates);
        }
    }
}

// ── extract_filter_columns ────────────────────────────────────────────────────

/// Extract column names from a Postgres filter expression (best-effort).
///
/// Postgres EXPLAIN outputs filter expressions in a human-readable SQL-like
/// form, e.g. `(status = 'open')` or `((amount > 0) AND (user_id = $1))`.
///
/// This function extracts identifiers that appear directly before comparison
/// operators (`=`, `<`, `>`, `<=`, `>=`, `<>`, `!=`, `LIKE`, `ILIKE`, `IN`).
/// It is intentionally conservative: if a column name cannot be determined
/// confidently, it returns an empty Vec rather than guessing wrong.
pub(crate) fn extract_filter_columns(filter: &str) -> Vec<String> {
    if filter.is_empty() {
        return vec![];
    }

    let mut columns = Vec::new();
    let operators = [
        " = ", " < ", " > ", " <= ", " >= ", " <> ", " != ", " LIKE ", " ILIKE ", " IN ",
    ];

    for op in &operators {
        let mut search = filter;
        while let Some(op_pos) = search.find(op) {
            // Work backwards from the operator to find the identifier.
            let before = &search[..op_pos];
            // Strip trailing whitespace and any closing parentheses.
            let before = before.trim_end().trim_end_matches(')').trim_end();
            // Extract the last identifier (word chars, optionally quoted).
            let col = extract_last_identifier(before);
            if !col.is_empty() && !columns.contains(&col) {
                columns.push(col);
            }
            search = &search[op_pos + op.len()..];
        }
    }

    columns
}

/// Extract the last SQL identifier from a string snippet.
///
/// Handles both bare identifiers (`status`) and double-quoted ones (`"Status"`).
fn extract_last_identifier(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }

    // Handle double-quoted identifier.
    if let Some(inner) = s.strip_suffix('"')
        && let Some(start) = inner.rfind('"')
    {
        let ident = &inner[start + 1..];
        return ident.to_string();
    }

    // Bare identifier: walk back from the end collecting word chars.
    let chars: Vec<char> = s.chars().collect();
    let mut end = chars.len();
    while end > 0 && (chars[end - 1].is_alphanumeric() || chars[end - 1] == '_') {
        end -= 1;
    }
    let col: String = chars[end..].iter().collect();

    // Reject SQL keywords that aren't column names.
    let col_lower = col.to_lowercase();
    if matches!(
        col_lower.as_str(),
        "and" | "or" | "not" | "true" | "false" | "null" | "any" | "all" | "is"
    ) {
        return String::new();
    }

    col
}

// ── check_existing_index ─────────────────────────────────────────────────────

/// Returns true if the table already has an index covering any of the filter columns.
///
/// This is a best-effort check: it queries pg_indexes and looks for the column
/// names in the index definition string. A more rigorous check would parse the
/// index definition, but string-contains is sufficient for the heuristic.
async fn check_existing_index(
    client: &deadpool_postgres::Client,
    table_name: &str,
    schema: &str,
    filter_columns: &[String],
) -> bool {
    let rows = client
        .query(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = $1 AND tablename = $2",
            &[&schema, &table_name],
        )
        .await
        .unwrap_or_default();

    if filter_columns.is_empty() {
        // No specific columns to check — if any non-PK index exists, call it covered.
        return rows.len() > 1; // more than just the PK
    }

    // Check if any existing index mentions the filter columns.
    rows.iter().any(|row| {
        let def: &str = row.try_get(0).unwrap_or("");
        filter_columns.iter().any(|col| def.contains(col.as_str()))
    })
}

// ── generate_create_index_sql ─────────────────────────────────────────────────

/// Generate a CREATE INDEX CONCURRENTLY statement for the given table and columns.
fn generate_create_index_sql(table_name: &str, schema: &str, columns: &[String]) -> String {
    let index_name = if columns.is_empty() {
        format!("idx_{table_name}_seq_scan")
    } else {
        format!("idx_{}_{}", table_name, columns.join("_"))
    };

    let col_list = if columns.is_empty() {
        "-- (add relevant columns here)".to_string()
    } else {
        columns.join(", ")
    };

    format!("CREATE INDEX CONCURRENTLY {index_name} ON {schema}.{table_name} ({col_list})")
}

// ── estimate_index_size ───────────────────────────────────────────────────────

/// Estimate btree index size in bytes.
///
/// Formula: `row_count * avg_column_width * 1.3` (btree overhead factor)
/// clamped to a minimum of 8 KB (one index page).
fn estimate_index_size(rows: i64, avg_width: i64) -> i64 {
    let raw = (rows * avg_width.max(8)) as f64 * 1.3;
    let estimate = raw as i64;
    estimate.max(8192) // minimum one 8K page
}

// ── classify_impact ───────────────────────────────────────────────────────────

/// Classify the impact of an index as "high", "medium", or "low".
fn classify_impact(rows: i64, plan_cost: f64) -> &'static str {
    if rows > 100_000 || plan_cost > 50_000.0 {
        "high"
    } else if rows > 10_000 || plan_cost > 5_000.0 {
        "medium"
    } else {
        "low"
    }
}

// ── Public re-export for explain.rs ──────────────────────────────────────────

// The plan JSON extraction helper is shared with explain.rs. We expose it
// from explain.rs under a public name and use it here.

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(json: &str) -> Option<Map<String, Value>> {
        serde_json::from_str::<Value>(json)
            .ok()
            .and_then(|v| v.as_object().cloned())
    }

    // ── Parameter extraction ──────────────────────────────────────────────────

    #[test]
    fn params_sql_required() {
        assert!(SuggestParams::from_args(None).is_err());
    }

    #[test]
    fn params_sql_required_empty() {
        assert!(SuggestParams::from_args(make_args("{}").as_ref()).is_err());
    }

    #[test]
    fn params_sql_empty_string() {
        assert!(SuggestParams::from_args(make_args(r#"{"sql": ""}"#).as_ref()).is_err());
    }

    #[test]
    fn params_schema_defaults_to_public() {
        let p = SuggestParams::from_args(make_args(r#"{"sql": "SELECT 1"}"#).as_ref()).unwrap();
        assert_eq!(p.schema, "public");
    }

    #[test]
    fn params_schema_explicit() {
        let p = SuggestParams::from_args(
            make_args(r#"{"sql": "SELECT 1", "schema": "analytics"}"#).as_ref(),
        )
        .unwrap();
        assert_eq!(p.schema, "analytics");
    }

    #[test]
    fn params_sql_trimmed() {
        let p = SuggestParams::from_args(make_args(r#"{"sql": "  SELECT 1  "}"#).as_ref()).unwrap();
        assert_eq!(p.sql, "SELECT 1");
    }

    // ── Plan tree walking ─────────────────────────────────────────────────────

    fn make_plan(node: Value) -> Value {
        serde_json::json!([{"Plan": node}])
    }

    #[test]
    fn collect_seq_scan_large_table() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "orders",
            "Plan Rows": 50000,
            "Plan Width": 100,
        }));
        let candidates = collect_seq_scan_candidates(&plan);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].table_name, "orders");
    }

    #[test]
    fn collect_seq_scan_small_table_ignored() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "config",
            "Plan Rows": 10,
            "Plan Width": 50,
        }));
        let candidates = collect_seq_scan_candidates(&plan);
        assert!(
            candidates.is_empty(),
            "small table should not be a candidate"
        );
    }

    #[test]
    fn collect_seq_scan_no_relation_name_ignored() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Plan Rows": 50000,
            "Plan Width": 100,
        }));
        let candidates = collect_seq_scan_candidates(&plan);
        assert!(
            candidates.is_empty(),
            "node without relation name should be ignored"
        );
    }

    #[test]
    fn collect_seq_scan_recurses_into_children() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Nested Loop",
            "Plan Rows": 1000,
            "Plan Width": 50,
            "Plans": [
                {
                    "Node Type": "Seq Scan",
                    "Relation Name": "orders",
                    "Plan Rows": 50000,
                    "Plan Width": 100,
                },
                {
                    "Node Type": "Index Scan",
                    "Relation Name": "users",
                    "Plan Rows": 1,
                    "Plan Width": 50,
                }
            ]
        }));
        let candidates = collect_seq_scan_candidates(&plan);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].table_name, "orders");
    }

    #[test]
    fn collect_seq_scan_with_filter() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "orders",
            "Plan Rows": 50000,
            "Plan Width": 100,
            "Filter": "(status = 'open')",
        }));
        let candidates = collect_seq_scan_candidates(&plan);
        assert_eq!(candidates.len(), 1);
        assert!(
            candidates[0].filter_columns.contains(&"status".to_string()),
            "filter columns should include 'status'"
        );
    }

    // ── Filter column extraction ──────────────────────────────────────────────

    #[test]
    fn extract_filter_empty() {
        assert!(extract_filter_columns("").is_empty());
    }

    #[test]
    fn extract_filter_simple_equality() {
        let cols = extract_filter_columns("(status = 'open')");
        assert!(
            cols.contains(&"status".to_string()),
            "should extract 'status': got {cols:?}"
        );
    }

    #[test]
    fn extract_filter_compound_and() {
        let cols = extract_filter_columns("((status = 'open') AND (amount > 100))");
        assert!(
            cols.contains(&"status".to_string()),
            "should have status: {cols:?}"
        );
        assert!(
            cols.contains(&"amount".to_string()),
            "should have amount: {cols:?}"
        );
    }

    #[test]
    fn extract_filter_parameter_placeholder() {
        let cols = extract_filter_columns("(user_id = $1)");
        assert!(
            cols.contains(&"user_id".to_string()),
            "should extract 'user_id': got {cols:?}"
        );
    }

    #[test]
    fn extract_filter_no_duplicates() {
        let cols = extract_filter_columns("(status = 'a' AND status = 'b')");
        let count = cols.iter().filter(|c| *c == "status").count();
        assert_eq!(count, 1, "status should not be duplicated");
    }

    // ── Index SQL generation ──────────────────────────────────────────────────

    #[test]
    fn generate_index_sql_with_columns() {
        let sql = generate_create_index_sql("orders", "public", &["status".to_string()]);
        assert!(sql.contains("CONCURRENTLY"), "must use CONCURRENTLY");
        assert!(sql.contains("public.orders"), "must include schema.table");
        assert!(sql.contains("status"), "must include column");
    }

    #[test]
    fn generate_index_sql_composite() {
        let sql = generate_create_index_sql(
            "orders",
            "public",
            &["status".to_string(), "created_at".to_string()],
        );
        assert!(sql.contains("status, created_at") || sql.contains("status,created_at"));
    }

    #[test]
    fn generate_index_sql_no_columns() {
        let sql = generate_create_index_sql("orders", "public", &[]);
        assert!(sql.contains("CONCURRENTLY"));
        assert!(sql.contains("orders"));
    }

    // ── Size estimation ───────────────────────────────────────────────────────

    #[test]
    fn estimate_size_minimum_8kb() {
        // Even 1 row must produce at least 8192 bytes (one page).
        let size = estimate_index_size(1, 4);
        assert_eq!(size, 8192);
    }

    #[test]
    fn estimate_size_large_table() {
        let size = estimate_index_size(1_000_000, 8);
        assert!(
            size > 1_000_000,
            "1M rows × 8 bytes × 1.3 overhead must exceed 1MB"
        );
    }

    // ── Impact classification ─────────────────────────────────────────────────

    #[test]
    fn impact_high_for_large_table() {
        assert_eq!(classify_impact(200_000, 100_000.0), "high");
    }

    #[test]
    fn impact_medium_for_medium_table() {
        assert_eq!(classify_impact(50_000, 10_000.0), "medium");
    }

    #[test]
    fn impact_low_for_small_table() {
        assert_eq!(classify_impact(500, 100.0), "low");
    }
}
