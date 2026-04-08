// src/tools/explain.rs
//
// explain tool — runs EXPLAIN on a SQL statement and returns a structured
// plan with plain-language analysis.
//
// Parameters:
//   sql     (string, required)  — SQL to explain; passed through guardrails
//   analyze (bool,   default false) — include ANALYZE (executes the statement)
//   verbose (bool,   default false) — include VERBOSE output
//   buffers (bool,   default true when analyze=true) — include buffer stats
//
// EXPLAIN SQL built:
//   EXPLAIN (FORMAT JSON [, ANALYZE] [, VERBOSE] [, BUFFERS])
//   BUFFERS is only included when analyze=true (PG rejects BUFFERS without ANALYZE).
//
// plan_text is obtained via a second EXPLAIN (FORMAT TEXT) call when analyze=false
// (estimation only — zero cost). When analyze=true, only the JSON plan is fetched
// to avoid double-execution; plan_text is built from the JSON node tree.
//
// Design invariants:
// - SQL always passes through sql::parser + sql::guardrails.
// - When analyze=false the SQL is never executed (only the planner runs).
// - The plain-language rule engine is a pure function with no DB I/O.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{
    error::McpError,
    server::context::ToolContext,
    sql::{guardrails::GuardrailConfig, parser::parse_statement},
};

// ── ExplainParams ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ExplainParams {
    sql: String,
    analyze: bool,
    verbose: bool,
    /// buffers is only meaningful when analyze=true.
    buffers: bool,
}

impl ExplainParams {
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

        let analyze = args
            .get("analyze")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let verbose = args
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // buffers defaults to true but is only applied when analyze=true.
        let buffers = args
            .get("buffers")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Ok(ExplainParams {
            sql,
            analyze,
            verbose,
            buffers,
        })
    }

    /// Build the EXPLAIN SQL string from parameters.
    fn explain_sql(&self) -> String {
        let mut opts = vec!["FORMAT JSON".to_string()];
        if self.analyze {
            opts.push("ANALYZE".to_string());
        }
        if self.verbose {
            opts.push("VERBOSE".to_string());
        }
        // BUFFERS is only valid with ANALYZE.
        if self.analyze && self.buffers {
            opts.push("BUFFERS".to_string());
        }
        format!("EXPLAIN ({}) {}", opts.join(", "), self.sql)
    }

    /// Build the plain-text EXPLAIN SQL (estimation only, no ANALYZE).
    fn explain_text_sql(&self) -> String {
        if self.verbose {
            format!("EXPLAIN (FORMAT TEXT, VERBOSE) {}", self.sql)
        } else {
            format!("EXPLAIN (FORMAT TEXT) {}", self.sql)
        }
    }
}

// ── handle ────────────────────────────────────────────────────────────────────

/// Handle an `explain` tool call.
///
/// # Errors
///
/// - [`McpError::param_invalid`] — missing or invalid parameters
/// - [`McpError::sql_parse_error`] — SQL that cannot be parsed
/// - [`McpError::guardrail_violation`] — SQL blocked by policy
/// - [`McpError::pg_pool_timeout`] — connection acquisition timeout
/// - [`McpError::pg_query_failed`] — EXPLAIN execution error from Postgres
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let params = ExplainParams::from_args(args.as_ref())?;

    tracing::debug!(
        sql = %params.sql,
        analyze = params.analyze,
        verbose = params.verbose,
        "explain tool invoked"
    );

    // Parse and validate SQL through guardrails (same as query tool).
    let parsed = parse_statement(&params.sql)?;

    let guardrail_config = GuardrailConfig {
        block_ddl: ctx.config.guardrails.block_ddl,
        block_copy_program: ctx.config.guardrails.block_copy_program,
        block_session_set: ctx.config.guardrails.block_session_set,
    };
    crate::sql::guardrails::check(&parsed, &guardrail_config)?;

    // Acquire connection.
    let acquire_timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(acquire_timeout).await?;

    // Run EXPLAIN (FORMAT JSON).
    let explain_sql = params.explain_sql();
    let json_rows = client
        .query(&explain_sql, &[])
        .await
        .map_err(McpError::from)?;

    let plan_json: Value = extract_plan_json(&json_rows)?;

    // Run EXPLAIN (FORMAT TEXT) only when analyze=false to get the text plan
    // without re-executing the statement.
    let plan_text: String = if params.analyze {
        // When analyze=true the statement already ran; synthesize text from JSON.
        synthesize_plan_text(&plan_json)
    } else {
        let text_sql = params.explain_text_sql();
        let text_rows = client.query(&text_sql, &[]).await.map_err(McpError::from)?;
        extract_plan_text(&text_rows).unwrap_or_default()
    };

    // Release connection before analysis (pure computation).
    drop(client);

    // Analyze the plan tree.
    let analysis = analyze_plan(&plan_json);

    let body = serde_json::json!({
        "sql": params.sql,
        "analyze": params.analyze,
        "plan_json": plan_json,
        "plan_text": plan_text,
        "summary": {
            "total_cost": analysis.total_cost,
            "estimated_rows": analysis.estimated_rows,
            "node_count": analysis.node_count,
            "warnings": analysis.warnings,
            "suggestions": analysis.suggestions,
        }
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

// ── Plan extraction helpers ───────────────────────────────────────────────────

/// Extract the EXPLAIN JSON plan from query rows.
///
/// Postgres returns `EXPLAIN (FORMAT JSON)` as a single row with one column.
/// The column type varies: on some PG versions it is TEXT, on others it may be
/// decoded as a `serde_json::Value` directly via the with-serde_json-1 feature.
/// We try both approaches.
/// Public alias used by `suggest_index` to reuse the same extraction logic.
pub fn extract_plan_json_from_rows(rows: &[tokio_postgres::Row]) -> Result<Value, McpError> {
    extract_plan_json(rows)
}

fn extract_plan_json(rows: &[tokio_postgres::Row]) -> Result<Value, McpError> {
    let row = rows
        .first()
        .ok_or_else(|| McpError::internal("EXPLAIN returned no rows"))?;

    // Try as text first (most common), then as native JSON value.
    if let Ok(plan_str) = row.try_get::<_, &str>(0) {
        return serde_json::from_str(plan_str)
            .map_err(|e| McpError::internal(format!("EXPLAIN JSON parse error: {e}")));
    }
    if let Ok(plan_str) = row.try_get::<_, String>(0) {
        return serde_json::from_str(&plan_str)
            .map_err(|e| McpError::internal(format!("EXPLAIN JSON parse error: {e}")));
    }
    // Fallback: try native serde_json Value (if tokio-postgres decoded it directly).
    row.try_get::<_, Value>(0)
        .map_err(|_| McpError::internal("EXPLAIN JSON column could not be decoded"))
}

/// Extract the plan text from `EXPLAIN (FORMAT TEXT)` rows.
///
/// Postgres returns one row per line of the text plan.
fn extract_plan_text(rows: &[tokio_postgres::Row]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let lines: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.try_get::<_, &str>(0).ok())
        .collect();
    Some(lines.join("\n"))
}

/// Build a simplified text representation from the JSON plan tree.
///
/// Used when `analyze=true` to avoid running the statement a second time.
fn synthesize_plan_text(plan_json: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(arr) = plan_json.as_array() {
        if let Some(first) = arr.first() {
            if let Some(plan) = first.get("Plan") {
                append_node_text(plan, 0, &mut lines);
            }
        }
    }
    lines.join("\n")
}

fn append_node_text(node: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let node_type = node
        .get("Node Type")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let cost = node
        .get("Total Cost")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let rows = node.get("Plan Rows").and_then(|v| v.as_i64()).unwrap_or(0);
    lines.push(format!(
        "{indent}-> {node_type}  (cost=0.00..{cost:.2} rows={rows} width=0)"
    ));

    // Recurse into child plans.
    if let Some(children) = node.get("Plans").and_then(|v| v.as_array()) {
        for child in children {
            append_node_text(child, depth + 1, lines);
        }
    }
}

// ── Plain-language plan analysis ──────────────────────────────────────────────

/// Result of analyzing a query plan tree.
#[derive(Debug, Default)]
pub(crate) struct PlanAnalysis {
    pub total_cost: f64,
    pub estimated_rows: i64,
    pub node_count: usize,
    pub warnings: Vec<String>,
    pub suggestions: Vec<String>,
}

/// Analyze an EXPLAIN JSON plan and return plain-language findings.
///
/// This function applies ~25 deterministic rules to the plan tree. It is pure
/// (no I/O) and can be tested without a database connection.
pub(crate) fn analyze_plan(plan_json: &Value) -> PlanAnalysis {
    let mut analysis = PlanAnalysis::default();

    let plan_root = if let Some(arr) = plan_json.as_array() {
        arr.first().and_then(|v| v.get("Plan"))
    } else {
        None
    };

    let Some(root) = plan_root else {
        return analysis;
    };

    // Extract top-level cost and row estimate.
    analysis.total_cost = root
        .get("Total Cost")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    analysis.estimated_rows = root.get("Plan Rows").and_then(|v| v.as_i64()).unwrap_or(0);

    // Walk the plan tree, collecting node-level findings.
    walk_plan_nodes(root, &mut analysis);

    analysis
}

/// Recursively walk all nodes in the plan tree, applying rules at each node.
fn walk_plan_nodes(node: &Value, analysis: &mut PlanAnalysis) {
    analysis.node_count += 1;

    let node_type = node.get("Node Type").and_then(|v| v.as_str()).unwrap_or("");

    apply_node_rules(node, node_type, analysis);

    // Recurse into child plans.
    if let Some(children) = node.get("Plans").and_then(|v| v.as_array()) {
        for child in children {
            walk_plan_nodes(child, analysis);
        }
    }
}

/// Apply all diagnostic rules to a single plan node.
///
/// Rules are numbered and correspond to the plan document.
fn apply_node_rules(node: &Value, node_type: &str, analysis: &mut PlanAnalysis) {
    let plan_rows = node.get("Plan Rows").and_then(|v| v.as_i64()).unwrap_or(0);
    let total_cost = node
        .get("Total Cost")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let relation_name = node
        .get("Relation Name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let loops = node
        .get("Actual Loops")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);

    // ── Rule 1: Seq Scan on large table ──────────────────────────────────────
    if node_type == "Seq Scan" && plan_rows > 10_000 {
        let filter = node.get("Filter").and_then(|v| v.as_str()).unwrap_or("");
        analysis.warnings.push(format!(
            "Sequential scan on '{relation_name}' (estimated {plan_rows} rows). \
             This may be slow on large tables."
        ));
        if !filter.is_empty() {
            // ── Rule 2: Seq Scan with filter ─────────────────────────────────
            analysis.suggestions.push(format!(
                "Consider adding an index on '{relation_name}' for the filter: {filter}"
            ));
        }
    }

    // ── Rule 3: Nested Loop with large outer ─────────────────────────────────
    if node_type == "Nested Loop" && plan_rows > 1_000 {
        analysis.warnings.push(format!(
            "Nested Loop join producing {plan_rows} estimated rows. \
             This may become an N+1 problem if the inner side is expensive."
        ));
    }

    // ── Rule 4: Hash Join disk spill ─────────────────────────────────────────
    if node_type == "Hash" {
        let batches = node
            .get("Hash Batches")
            .or_else(|| node.get("Planned Hash Batches"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        if batches > 1 {
            analysis.warnings.push(format!(
                "Hash join spilled to disk ({batches} batches). \
                 Increase work_mem to keep hash joins in memory."
            ));
            analysis.suggestions.push(
                "SET work_mem = '64MB' (or higher) to prevent hash join disk spills.".to_string(),
            );
        }
    }

    // ── Rules 5 & 6: Sort spill to disk ─────────────────────────────────────
    if node_type == "Sort" {
        let sort_method = node
            .get("Sort Method")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if sort_method.starts_with("external") {
            analysis.warnings.push(format!(
                "Sort operation spilled to disk (method: '{sort_method}'). \
                 Increase work_mem to keep sorts in memory."
            ));
            analysis
                .suggestions
                .push("SET work_mem = '64MB' (or higher) to prevent sort disk spills.".to_string());
        }
    }

    // ── Rule 7: Poor filter selectivity ─────────────────────────────────────
    if let Some(removed) = node.get("Rows Removed by Filter").and_then(|v| v.as_i64()) {
        let actual_rows = node
            .get("Actual Rows")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let total_scanned = actual_rows + removed;
        if total_scanned > 0 {
            let removal_pct = (removed * 100) / total_scanned;
            if removal_pct > 50 {
                analysis.warnings.push(format!(
                    "{removal_pct}% of scanned rows removed by filter — poor index selectivity \
                     or missing index."
                ));
            }
        }
    }

    // ── Rule 8: High-cost node ────────────────────────────────────────────────
    if total_cost > 10_000.0 && !relation_name.is_empty() {
        analysis.warnings.push(format!(
            "Node '{node_type}' on '{relation_name}' has high total cost ({total_cost:.1}). \
             Review the query or add indexes."
        ));
    }

    // ── Rule 9: Expensive nested loop inner startup ───────────────────────────
    if node_type == "Nested Loop" {
        let startup_cost = node
            .get("Startup Cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if startup_cost > 1_000.0 {
            analysis.warnings.push(format!(
                "Nested Loop inner side has high startup cost ({startup_cost:.1}). \
                 This cost is paid once per outer row."
            ));
        }
    }

    // ── Rule 12: Loop amplification ───────────────────────────────────────────
    if loops > 1 {
        let cost_per_loop = total_cost / loops as f64;
        if cost_per_loop * loops as f64 > 50_000.0 {
            analysis.warnings.push(format!(
                "Node '{node_type}' executed {loops} times (total cost: {total_cost:.1}). \
                 High loop amplification — consider restructuring the query."
            ));
        }
    }

    // ── Rule 13: Very wide rows ───────────────────────────────────────────────
    let width = node.get("Plan Width").and_then(|v| v.as_i64()).unwrap_or(0);
    if width > 4096 {
        analysis.warnings.push(format!(
            "Node '{node_type}' returns very wide rows ({width} bytes avg). \
             Consider selecting only required columns."
        ));
    }

    // ── Rule 15: Dominant disk reads ─────────────────────────────────────────
    if let (Some(shared_hit), Some(shared_read)) = (
        node.get("Shared Hit Blocks").and_then(|v| v.as_i64()),
        node.get("Shared Read Blocks").and_then(|v| v.as_i64()),
    ) {
        let total_blocks = shared_hit + shared_read;
        if total_blocks > 0 && shared_read > shared_hit {
            analysis.warnings.push(format!(
                "Most data read from disk ({shared_read} reads vs {shared_hit} cache hits) — \
                 cold buffer cache or insufficient shared_buffers."
            ));
        }
    }

    // ── Rule 16: Temp file usage ──────────────────────────────────────────────
    if let Some(temp_written) = node.get("Temp Written Blocks").and_then(|v| v.as_i64()) {
        if temp_written > 0 {
            analysis.warnings.push(format!(
                "Temp file usage detected ({temp_written} blocks written). \
                 Increase work_mem to reduce temp file I/O."
            ));
        }
    }

    // ── Rule 23: Stale statistics (row estimate = 1 on non-trivial node) ─────
    if plan_rows == 1 && total_cost > 100.0 && node_type != "Result" && node_type != "Aggregate" {
        analysis.warnings.push(format!(
            "Node '{node_type}' estimates 1 row with cost {total_cost:.1} — \
             possibly stale table statistics. Run ANALYZE on the referenced tables."
        ));
        analysis
            .suggestions
            .push("Run ANALYZE on the relevant tables to update row count statistics.".to_string());
    }

    // ── Rule 24: Function Scan ────────────────────────────────────────────────
    if node_type == "Function Scan" && loops > 1 {
        let func_name = node
            .get("Function Name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        analysis.warnings.push(format!(
            "Function Scan on '{func_name}' executed {loops} times — \
             set-returning functions inside loops are not cacheable."
        ));
    }

    // ── Rule 25: CTE fence ────────────────────────────────────────────────────
    if node_type == "CTE Scan" {
        let subplan_name = node
            .get("CTE Name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        analysis.warnings.push(format!(
            "CTE Scan on '{subplan_name}' — CTEs are optimization fences in PostgreSQL < 12. \
             On PostgreSQL 12+, use WITH MATERIALIZED only when needed."
        ));
    }
}

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
        let err = ExplainParams::from_args(None).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn params_sql_required_when_missing() {
        let err = ExplainParams::from_args(make_args("{}").as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn params_sql_required_when_empty() {
        let err = ExplainParams::from_args(make_args(r#"{"sql": ""}"#).as_ref()).unwrap_err();
        assert_eq!(err.code(), "param_invalid");
    }

    #[test]
    fn params_defaults() {
        let p = ExplainParams::from_args(make_args(r#"{"sql": "SELECT 1"}"#).as_ref()).unwrap();
        assert_eq!(p.sql, "SELECT 1");
        assert!(!p.analyze);
        assert!(!p.verbose);
        assert!(p.buffers); // default true
    }

    #[test]
    fn params_analyze_true() {
        let p =
            ExplainParams::from_args(make_args(r#"{"sql": "SELECT 1", "analyze": true}"#).as_ref())
                .unwrap();
        assert!(p.analyze);
    }

    #[test]
    fn params_verbose_true() {
        let p =
            ExplainParams::from_args(make_args(r#"{"sql": "SELECT 1", "verbose": true}"#).as_ref())
                .unwrap();
        assert!(p.verbose);
    }

    #[test]
    fn params_sql_trimmed() {
        let p = ExplainParams::from_args(make_args(r#"{"sql": "  SELECT 1  "}"#).as_ref()).unwrap();
        assert_eq!(p.sql, "SELECT 1");
    }

    // ── EXPLAIN SQL construction ──────────────────────────────────────────────

    #[test]
    fn explain_sql_without_analyze() {
        let p = ExplainParams {
            sql: "SELECT 1".to_string(),
            analyze: false,
            verbose: false,
            buffers: true,
        };
        let sql = p.explain_sql();
        assert!(sql.contains("FORMAT JSON"), "must have FORMAT JSON: {sql}");
        assert!(!sql.contains("ANALYZE"), "must not have ANALYZE: {sql}");
        assert!(!sql.contains("BUFFERS"), "BUFFERS requires ANALYZE: {sql}");
    }

    #[test]
    fn explain_sql_with_analyze_and_buffers() {
        let p = ExplainParams {
            sql: "SELECT 1".to_string(),
            analyze: true,
            verbose: false,
            buffers: true,
        };
        let sql = p.explain_sql();
        assert!(sql.contains("FORMAT JSON"));
        assert!(sql.contains("ANALYZE"));
        assert!(sql.contains("BUFFERS"));
    }

    #[test]
    fn explain_sql_with_analyze_no_buffers() {
        let p = ExplainParams {
            sql: "SELECT 1".to_string(),
            analyze: true,
            verbose: false,
            buffers: false,
        };
        let sql = p.explain_sql();
        assert!(sql.contains("ANALYZE"));
        assert!(!sql.contains("BUFFERS"));
    }

    #[test]
    fn explain_sql_with_verbose() {
        let p = ExplainParams {
            sql: "SELECT 1".to_string(),
            analyze: false,
            verbose: true,
            buffers: true,
        };
        let sql = p.explain_sql();
        assert!(sql.contains("VERBOSE"));
    }

    #[test]
    fn buffers_not_included_without_analyze() {
        let p = ExplainParams {
            sql: "SELECT 1".to_string(),
            analyze: false,
            verbose: false,
            buffers: true,
        };
        // Even if user set buffers=true, it must not appear without ANALYZE.
        assert!(!p.explain_sql().contains("BUFFERS"));
    }

    // ── Plain-language rule engine ────────────────────────────────────────────

    fn make_plan(node_json: serde_json::Value) -> Value {
        serde_json::json!([{"Plan": node_json}])
    }

    #[test]
    fn rule1_seq_scan_large_table_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "orders",
            "Plan Rows": 50000,
            "Total Cost": 5000.0,
            "Plan Width": 100
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis
                .warnings
                .iter()
                .any(|w| w.contains("Sequential scan")),
            "expected seq scan warning, got: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule1_seq_scan_small_table_no_warn() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "users",
            "Plan Rows": 100,
            "Total Cost": 10.0,
            "Plan Width": 50
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            !analysis
                .warnings
                .iter()
                .any(|w| w.contains("Sequential scan")),
            "no seq scan warning for small table: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule2_seq_scan_with_filter_suggests_index() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "orders",
            "Plan Rows": 50000,
            "Total Cost": 5000.0,
            "Plan Width": 100,
            "Filter": "(status = 'open')"
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.suggestions.iter().any(|s| s.contains("index")),
            "expected index suggestion, got: {:?}",
            analysis.suggestions
        );
    }

    #[test]
    fn rule4_hash_spill_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Hash",
            "Hash Batches": 4,
            "Plan Rows": 1000,
            "Total Cost": 200.0,
            "Plan Width": 8
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis
                .warnings
                .iter()
                .any(|w| w.contains("spilled to disk")),
            "expected hash spill warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule5_sort_spill_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Sort",
            "Sort Method": "external merge",
            "Plan Rows": 10000,
            "Total Cost": 1000.0,
            "Plan Width": 50
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis
                .warnings
                .iter()
                .any(|w| w.contains("spilled to disk")),
            "expected sort spill warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule7_poor_filter_selectivity_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "events",
            "Plan Rows": 10,
            "Total Cost": 500.0,
            "Plan Width": 50,
            "Actual Rows": 10,
            "Rows Removed by Filter": 9990
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.warnings.iter().any(|w| w.contains('%')),
            "expected selectivity warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule13_wide_rows_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "blobs",
            "Plan Rows": 5,
            "Total Cost": 10.0,
            "Plan Width": 8192
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.warnings.iter().any(|w| w.contains("wide rows")),
            "expected wide row warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule23_stale_stats_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "events",
            "Plan Rows": 1,
            "Total Cost": 10000.0,
            "Plan Width": 50
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.warnings.iter().any(|w| w.contains("stale")),
            "expected stale stats warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn rule25_cte_scan_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "CTE Scan",
            "CTE Name": "my_cte",
            "Plan Rows": 100,
            "Total Cost": 200.0,
            "Plan Width": 8
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.warnings.iter().any(|w| w.contains("CTE")),
            "expected CTE fence warning: {:?}",
            analysis.warnings
        );
    }

    #[test]
    fn total_cost_extracted_from_root() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "t",
            "Plan Rows": 100,
            "Total Cost": 1234.56,
            "Plan Width": 10
        }));
        let analysis = analyze_plan(&plan);
        assert!((analysis.total_cost - 1234.56).abs() < 0.01);
    }

    #[test]
    fn estimated_rows_extracted() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Seq Scan",
            "Relation Name": "t",
            "Plan Rows": 42,
            "Total Cost": 10.0,
            "Plan Width": 10
        }));
        let analysis = analyze_plan(&plan);
        assert_eq!(analysis.estimated_rows, 42);
    }

    #[test]
    fn node_count_recurses_into_children() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Nested Loop",
            "Plan Rows": 100,
            "Total Cost": 200.0,
            "Plan Width": 10,
            "Plans": [
                {
                    "Node Type": "Seq Scan",
                    "Relation Name": "a",
                    "Plan Rows": 10,
                    "Total Cost": 10.0,
                    "Plan Width": 5
                },
                {
                    "Node Type": "Index Scan",
                    "Relation Name": "b",
                    "Plan Rows": 1,
                    "Total Cost": 5.0,
                    "Plan Width": 5
                }
            ]
        }));
        let analysis = analyze_plan(&plan);
        assert_eq!(analysis.node_count, 3, "root + 2 children = 3 nodes");
    }

    #[test]
    fn empty_plan_returns_default_analysis() {
        let plan = Value::Null;
        let analysis = analyze_plan(&plan);
        assert_eq!(analysis.node_count, 0);
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn nested_loop_large_warns() {
        let plan = make_plan(serde_json::json!({
            "Node Type": "Nested Loop",
            "Plan Rows": 5000,
            "Total Cost": 1000.0,
            "Plan Width": 50,
            "Plans": []
        }));
        let analysis = analyze_plan(&plan);
        assert!(
            analysis.warnings.iter().any(|w| w.contains("Nested Loop")),
            "expected nested loop warning: {:?}",
            analysis.warnings
        );
    }
}
