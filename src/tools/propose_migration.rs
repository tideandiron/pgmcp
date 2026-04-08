// src/tools/propose_migration.rs
//
// propose_migration tool — analyzes DDL SQL and returns a safety assessment.
//
// Parameters:
//   sql (string, required) — DDL statement to analyze (CREATE/ALTER/DROP/TRUNCATE)
//
// This tool does NOT execute the SQL. It is purely static analysis.
//
// Pipeline:
//   1. Parse SQL with sqlparser
//   2. Classify statement (must be DDL)
//   3. Generate reverse SQL (undo statement)
//   4. Determine lock type and risk levels
//   5. Generate version-aware warnings
//   6. Return structured JSON assessment
//
// Version awareness: reads server_version_num from Postgres once per call to
// emit version-specific warnings (e.g., NOT NULL DEFAULT on PG < 11).
//
// Design invariants:
// - No SQL is executed (only SHOW server_version_num to get PG version)
// - All analysis is pure after the version lookup
// - Requires at least one DDL keyword to accept the statement

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{
    error::McpError,
    server::context::ToolContext,
    sql::parser::{StatementKind, parse_statement},
};

// ── DdlKind ───────────────────────────────────────────────────────────────────

/// Refined DDL classification for migration analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DdlKind {
    CreateTable,
    CreateIndex,
    CreateIndexConcurrently,
    AlterTableAddColumnNullable,
    AlterTableAddColumnNotNull,
    AlterTableAddColumnNotNullDefault,
    AlterTableDropColumn,
    AlterTableAlterColumnType,
    AlterTableOther,
    DropTable,
    DropIndex,
    Truncate,
    Other,
}

// ── Parameters ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MigrationParams {
    sql: String,
}

impl MigrationParams {
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

        Ok(MigrationParams { sql })
    }
}

// ── handle ────────────────────────────────────────────────────────────────────

/// Handle a `propose_migration` tool call.
///
/// Analyzes the given DDL SQL and returns a structured safety assessment.
/// Does not execute the SQL.
///
/// # Errors
///
/// - [`McpError::param_invalid`] — missing or non-DDL SQL
/// - [`McpError::sql_parse_error`] — SQL that cannot be parsed
/// - [`McpError::pg_pool_timeout`] — connection acquisition timeout
/// - [`McpError::pg_query_failed`] — version query failure
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let params = MigrationParams::from_args(args.as_ref())?;

    tracing::debug!(sql = %params.sql, "propose_migration tool invoked");

    // Parse the SQL.
    let parsed = parse_statement(&params.sql)?;

    // Must be DDL.
    let is_ddl = matches!(
        parsed.kind,
        StatementKind::CreateTable
            | StatementKind::CreateIndex
            | StatementKind::AlterTable
            | StatementKind::DropTable
            | StatementKind::DropIndex
            | StatementKind::Truncate
    );

    if !is_ddl {
        return Err(McpError::param_invalid(
            "sql",
            "propose_migration only accepts DDL statements \
             (CREATE TABLE, ALTER TABLE, DROP TABLE, CREATE INDEX, DROP INDEX, TRUNCATE). \
             Use the query tool for DML statements.",
        ));
    }

    // Get PG version for version-aware warnings.
    let pg_version_num = get_pg_version_num(&ctx).await.unwrap_or(0);
    let pg_version_str = format_pg_version(pg_version_num);

    // Classify the DDL more precisely.
    let ddl_kind = classify_ddl_kind(&params.sql, &parsed.kind);

    // Generate analysis.
    let statement_type = statement_type_str(&parsed.kind);
    let is_destructive = is_destructive_operation(ddl_kind);
    let reverse_sql = generate_reverse_sql(ddl_kind, &params.sql, &parsed);
    let lock_type = lock_type_for(ddl_kind);
    let lock_risk = lock_risk_for(ddl_kind);
    let downtime_risk = downtime_risk_for(ddl_kind);
    let data_loss_risk = data_loss_risk_for(ddl_kind);
    let warnings = generate_warnings(ddl_kind, pg_version_num, &params.sql);
    let suggestions = generate_suggestions(ddl_kind, pg_version_num, &params.sql);

    let body = serde_json::json!({
        "sql": params.sql,
        "statement_type": statement_type,
        "is_destructive": is_destructive,
        "reverse_sql": reverse_sql,
        "lock_type": lock_type,
        "lock_risk": lock_risk,
        "downtime_risk": downtime_risk,
        "data_loss_risk": data_loss_risk,
        "warnings": warnings,
        "suggestions": suggestions,
        "pg_version": pg_version_str,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

// ── PG version helpers ────────────────────────────────────────────────────────

/// Query `SHOW server_version_num` and return as an integer.
/// Returns 0 on any error (treated as unknown version).
async fn get_pg_version_num(ctx: &ToolContext) -> Option<u32> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await.ok()?;
    let rows = client.query("SHOW server_version_num", &[]).await.ok()?;
    let row = rows.first()?;
    let ver_str: &str = row.try_get(0).ok()?;
    ver_str.trim().parse::<u32>().ok()
}

/// Format an integer PG version (e.g. 160001) as a human string.
///
/// PG uses server_version_num where major is `num / 10000` and the patch is
/// `num % 100`. For example: 160001 → 16.0.1, 140005 → 14.0.5.
/// We format as "major" when patch=0, "major.patch" otherwise.
pub(crate) fn format_pg_version(num: u32) -> String {
    if num == 0 {
        return "unknown".to_string();
    }
    let major = num / 10_000;
    let patch = num % 100;
    if patch == 0 {
        format!("{major}")
    } else {
        format!("{major}.{patch}")
    }
}

// ── DDL classification ────────────────────────────────────────────────────────

/// Refine the StatementKind into a DdlKind by inspecting the SQL text.
pub(crate) fn classify_ddl_kind(sql: &str, kind: &StatementKind) -> DdlKind {
    let upper = sql.to_uppercase();
    match kind {
        StatementKind::CreateTable => DdlKind::CreateTable,
        StatementKind::DropTable => DdlKind::DropTable,
        StatementKind::DropIndex => DdlKind::DropIndex,
        StatementKind::Truncate => DdlKind::Truncate,
        StatementKind::CreateIndex => {
            if upper.contains("CONCURRENTLY") {
                DdlKind::CreateIndexConcurrently
            } else {
                DdlKind::CreateIndex
            }
        }
        StatementKind::AlterTable => classify_alter_table(&upper),
        _ => DdlKind::Other,
    }
}

fn classify_alter_table(upper: &str) -> DdlKind {
    if upper.contains("DROP COLUMN") {
        DdlKind::AlterTableDropColumn
    } else if upper.contains("ALTER COLUMN") && upper.contains("TYPE") {
        DdlKind::AlterTableAlterColumnType
    } else if upper.contains("ADD COLUMN") {
        // Order matters: check most specific first.
        if upper.contains("NOT NULL") && (upper.contains("DEFAULT") || upper.contains("GENERATED"))
        {
            DdlKind::AlterTableAddColumnNotNullDefault
        } else if upper.contains("NOT NULL") {
            DdlKind::AlterTableAddColumnNotNull
        } else {
            DdlKind::AlterTableAddColumnNullable
        }
    } else {
        DdlKind::AlterTableOther
    }
}

// ── Analysis helpers ──────────────────────────────────────────────────────────

fn statement_type_str(kind: &StatementKind) -> &'static str {
    match kind {
        StatementKind::CreateTable => "create_table",
        StatementKind::AlterTable => "alter_table",
        StatementKind::DropTable => "drop_table",
        StatementKind::CreateIndex => "create_index",
        StatementKind::DropIndex => "drop_index",
        StatementKind::Truncate => "truncate",
        _ => "other",
    }
}

fn is_destructive_operation(kind: DdlKind) -> bool {
    matches!(
        kind,
        DdlKind::DropTable | DdlKind::DropIndex | DdlKind::AlterTableDropColumn | DdlKind::Truncate
    )
}

fn lock_type_for(kind: DdlKind) -> &'static str {
    match kind {
        DdlKind::CreateTable => "ShareRowExclusiveLock (on parent table, if any)",
        DdlKind::CreateIndex => "ShareLock (blocks writes during index build)",
        DdlKind::CreateIndexConcurrently => "ShareUpdateExclusiveLock (non-blocking)",
        DdlKind::AlterTableAddColumnNullable => "AccessExclusiveLock",
        DdlKind::AlterTableAddColumnNotNull => "AccessExclusiveLock",
        DdlKind::AlterTableAddColumnNotNullDefault => "AccessExclusiveLock",
        DdlKind::AlterTableDropColumn => "AccessExclusiveLock",
        DdlKind::AlterTableAlterColumnType => "AccessExclusiveLock",
        DdlKind::AlterTableOther => "AccessExclusiveLock",
        DdlKind::DropTable => "AccessExclusiveLock",
        DdlKind::DropIndex => "AccessExclusiveLock",
        DdlKind::Truncate => "AccessExclusiveLock",
        DdlKind::Other => "Unknown",
    }
}

fn lock_risk_for(kind: DdlKind) -> &'static str {
    match kind {
        DdlKind::CreateTable => "low",
        DdlKind::CreateIndex => "high",
        DdlKind::CreateIndexConcurrently => "low",
        DdlKind::AlterTableAddColumnNullable => "medium",
        DdlKind::AlterTableAddColumnNotNull => "high",
        DdlKind::AlterTableAddColumnNotNullDefault => "medium",
        DdlKind::AlterTableDropColumn => "high",
        DdlKind::AlterTableAlterColumnType => "high",
        DdlKind::AlterTableOther => "medium",
        DdlKind::DropTable => "high",
        DdlKind::DropIndex => "high",
        DdlKind::Truncate => "high",
        DdlKind::Other => "unknown",
    }
}

fn downtime_risk_for(kind: DdlKind) -> &'static str {
    match kind {
        DdlKind::CreateTable => "none",
        DdlKind::CreateIndex => "high",
        DdlKind::CreateIndexConcurrently => "none",
        DdlKind::AlterTableAddColumnNullable => "low",
        DdlKind::AlterTableAddColumnNotNull => "high",
        DdlKind::AlterTableAddColumnNotNullDefault => "low",
        DdlKind::AlterTableDropColumn => "medium",
        DdlKind::AlterTableAlterColumnType => "high",
        DdlKind::AlterTableOther => "low",
        DdlKind::DropTable => "none",
        DdlKind::DropIndex => "low",
        DdlKind::Truncate => "none",
        DdlKind::Other => "unknown",
    }
}

fn data_loss_risk_for(kind: DdlKind) -> &'static str {
    match kind {
        DdlKind::DropTable => "high",
        DdlKind::AlterTableDropColumn => "high",
        DdlKind::Truncate => "high",
        DdlKind::AlterTableAlterColumnType => "medium",
        _ => "none",
    }
}

// ── Reverse SQL generation ────────────────────────────────────────────────────

/// Generate the undo (reverse) SQL for a DDL statement.
///
/// Returns `Some(sql)` when a reasonable undo is possible,
/// `None` when the operation cannot be reversed (data is gone).
pub(crate) fn generate_reverse_sql(
    kind: DdlKind,
    original_sql: &str,
    parsed: &crate::sql::parser::ParsedStatement,
) -> Option<String> {
    match kind {
        DdlKind::CreateTable => {
            let table = parsed.table_names.first()?;
            Some(format!("DROP TABLE IF EXISTS {table}"))
        }

        DdlKind::CreateIndex | DdlKind::CreateIndexConcurrently => {
            // Extract index name: "CREATE [UNIQUE] INDEX [CONCURRENTLY] name ON ..."
            let index_name = extract_create_index_name(original_sql)?;
            Some(format!("DROP INDEX CONCURRENTLY IF EXISTS {index_name}"))
        }

        DdlKind::AlterTableAddColumnNullable
        | DdlKind::AlterTableAddColumnNotNull
        | DdlKind::AlterTableAddColumnNotNullDefault => {
            let table = parsed.table_names.first()?;
            let col_name = extract_add_column_name(original_sql)?;
            Some(format!(
                "ALTER TABLE {table} DROP COLUMN IF EXISTS {col_name}"
            ))
        }

        // Cannot reverse — data is gone.
        DdlKind::DropTable | DdlKind::AlterTableDropColumn | DdlKind::Truncate => None,

        // Cannot reverse without original DDL.
        DdlKind::DropIndex => None,

        // Too complex to auto-reverse.
        DdlKind::AlterTableAlterColumnType | DdlKind::AlterTableOther | DdlKind::Other => None,
    }
}

/// Extract the index name from a CREATE INDEX statement.
///
/// Handles: `CREATE [UNIQUE] INDEX [CONCURRENTLY] name ON ...`
fn extract_create_index_name(sql: &str) -> Option<String> {
    // Normalize whitespace.
    let words: Vec<&str> = sql.split_whitespace().collect();
    // Find "INDEX" keyword position.
    let idx_pos = words.iter().position(|w| w.to_uppercase() == "INDEX")?;
    // The token after INDEX (skipping CONCURRENTLY if present) is the name.
    let mut candidate_pos = idx_pos + 1;
    if candidate_pos < words.len() && words[candidate_pos].to_uppercase() == "CONCURRENTLY" {
        candidate_pos += 1;
    }
    // The next token — but if it's "ON", the index is unnamed (not standard but handle gracefully).
    if candidate_pos < words.len() {
        let name = words[candidate_pos];
        if name.to_uppercase() != "ON" {
            return Some(name.trim_matches(';').to_string());
        }
    }
    None
}

/// Extract the column name from an ADD COLUMN clause.
///
/// Handles: `ALTER TABLE t ADD COLUMN col_name type ...`
fn extract_add_column_name(sql: &str) -> Option<String> {
    let words: Vec<&str> = sql.split_whitespace().collect();
    // Find "COLUMN" keyword.
    let col_pos = words.iter().position(|w| w.to_uppercase() == "COLUMN")?;
    // The next token is the column name.
    words
        .get(col_pos + 1)
        .map(|w| w.trim_matches('"').to_string())
}

// ── Warning generation ────────────────────────────────────────────────────────

/// Generate a list of plain-language warnings for the DDL operation.
pub(crate) fn generate_warnings(kind: DdlKind, pg_version_num: u32, sql: &str) -> Vec<String> {
    let mut warnings = Vec::new();

    // Lock-based warnings.
    match kind {
        DdlKind::CreateIndex => {
            warnings.push(
                "CREATE INDEX (without CONCURRENTLY) acquires ShareLock, which blocks \
                 writes to the table for the entire duration of the index build."
                    .to_string(),
            );
        }
        DdlKind::AlterTableAddColumnNullable
        | DdlKind::AlterTableAddColumnNotNull
        | DdlKind::AlterTableAddColumnNotNullDefault
        | DdlKind::AlterTableDropColumn
        | DdlKind::AlterTableAlterColumnType
        | DdlKind::AlterTableOther
        | DdlKind::DropTable
        | DdlKind::DropIndex
        | DdlKind::Truncate => {
            warnings.push(
                "This operation acquires AccessExclusiveLock, blocking all concurrent \
                 reads and writes to the table until the operation completes."
                    .to_string(),
            );
        }
        _ => {}
    }

    // Data loss warnings.
    match kind {
        DdlKind::DropTable => {
            warnings.push(
                "DROP TABLE permanently deletes the table and all its data. \
                 This cannot be undone without a backup."
                    .to_string(),
            );
        }
        DdlKind::AlterTableDropColumn => {
            warnings.push(
                "DROP COLUMN permanently removes the column and all its data. \
                 This cannot be undone without a backup."
                    .to_string(),
            );
        }
        DdlKind::Truncate => {
            warnings.push(
                "TRUNCATE removes all rows from the table. \
                 This cannot be undone without a backup."
                    .to_string(),
            );
        }
        _ => {}
    }

    // PG version-aware warnings.
    let pg_major = if pg_version_num > 0 {
        pg_version_num / 10_000
    } else {
        99 // unknown version → skip version-specific warnings
    };

    if kind == DdlKind::AlterTableAddColumnNotNullDefault && pg_major < 11 {
        warnings.push(format!(
            "On PostgreSQL {pg_major} (< 11): ADD COLUMN with NOT NULL DEFAULT \
             triggers a full table rewrite. This can take a long time on large tables \
             and blocks all access. PostgreSQL 11+ avoids the rewrite."
        ));
    }

    if kind == DdlKind::AlterTableAddColumnNotNull {
        warnings.push(
            "Adding a NOT NULL column without a default value requires all existing rows \
             to be updated. On non-empty tables this will fail unless a default is provided."
                .to_string(),
        );
    }

    // CHECK IF EXISTS recommendation.
    let upper = sql.to_uppercase();
    if (kind == DdlKind::DropTable || kind == DdlKind::DropIndex) && !upper.contains("IF EXISTS") {
        warnings.push(
            "The statement does not use IF EXISTS. If the table/index does not exist, \
             the statement will fail with an error."
                .to_string(),
        );
    }

    // Type change warning.
    if kind == DdlKind::AlterTableAlterColumnType {
        warnings.push(
            "ALTER COLUMN TYPE may require a full table rewrite depending on the type \
             conversion. If the cast is not implicit, a USING clause is required."
                .to_string(),
        );
    }

    warnings
}

/// Generate actionable suggestions for the DDL operation.
pub(crate) fn generate_suggestions(kind: DdlKind, pg_version_num: u32, _sql: &str) -> Vec<String> {
    let mut suggestions = Vec::new();
    let pg_major = if pg_version_num > 0 {
        pg_version_num / 10_000
    } else {
        99
    };

    match kind {
        DdlKind::CreateIndex => {
            suggestions.push(
                "Use CREATE INDEX CONCURRENTLY to build the index without blocking writes. \
                 Note: CONCURRENTLY cannot be used inside a transaction block."
                    .to_string(),
            );
        }

        DdlKind::AlterTableAddColumnNotNull => {
            if pg_major >= 11 {
                suggestions.push(
                    "Add a DEFAULT value to the column to avoid table rewrites (PG 11+). \
                     Example: ALTER TABLE t ADD COLUMN col TYPE NOT NULL DEFAULT value"
                        .to_string(),
                );
            } else {
                suggestions.push(
                    "Consider adding the column as nullable first, backfilling data, \
                     then adding the NOT NULL constraint."
                        .to_string(),
                );
            }
        }

        DdlKind::AlterTableAlterColumnType => {
            suggestions.push(
                "Test the migration on a copy of the data first. \
                 If the cast is not trivial, add a USING clause."
                    .to_string(),
            );
            suggestions.push(
                "For zero-downtime: add a new column, backfill, then rename columns \
                 in a single transaction."
                    .to_string(),
            );
        }

        DdlKind::DropTable | DdlKind::AlterTableDropColumn => {
            suggestions.push(
                "Take a database backup before running this statement. \
                 Consider renaming the table/column first and dropping after a \
                 validation period."
                    .to_string(),
            );
        }

        DdlKind::AlterTableAddColumnNullable | DdlKind::AlterTableAddColumnNotNullDefault => {
            suggestions.push(
                "If the table is large, consider running this during a low-traffic window \
                 as it still acquires AccessExclusiveLock."
                    .to_string(),
            );
        }

        _ => {}
    }

    suggestions
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::parser::parse_statement;

    fn make_args(json: &str) -> Option<Map<String, Value>> {
        serde_json::from_str::<Value>(json)
            .ok()
            .and_then(|v| v.as_object().cloned())
    }

    fn parse(sql: &str) -> crate::sql::parser::ParsedStatement {
        parse_statement(sql).expect("test SQL must parse")
    }

    // ── Parameter extraction ──────────────────────────────────────────────────

    #[test]
    fn params_sql_required() {
        assert!(MigrationParams::from_args(None).is_err());
    }

    #[test]
    fn params_sql_required_empty_args() {
        assert!(MigrationParams::from_args(make_args("{}").as_ref()).is_err());
    }

    #[test]
    fn params_sql_empty_string() {
        assert!(MigrationParams::from_args(make_args(r#"{"sql": ""}"#).as_ref()).is_err());
    }

    #[test]
    fn params_sql_valid() {
        let p =
            MigrationParams::from_args(make_args(r#"{"sql": "CREATE TABLE t (id INT)"}"#).as_ref())
                .unwrap();
        assert_eq!(p.sql, "CREATE TABLE t (id INT)");
    }

    // ── DDL classification ────────────────────────────────────────────────────

    #[test]
    fn classify_create_table() {
        let p = parse("CREATE TABLE orders (id INT)");
        let kind = classify_ddl_kind("CREATE TABLE orders (id INT)", &p.kind);
        assert_eq!(kind, DdlKind::CreateTable);
    }

    #[test]
    fn classify_create_index() {
        let p = parse("CREATE INDEX idx ON orders (status)");
        let kind = classify_ddl_kind("CREATE INDEX idx ON orders (status)", &p.kind);
        assert_eq!(kind, DdlKind::CreateIndex);
    }

    #[test]
    fn classify_create_index_concurrently() {
        let p = parse("CREATE INDEX CONCURRENTLY idx ON orders (status)");
        let kind = classify_ddl_kind("CREATE INDEX CONCURRENTLY idx ON orders (status)", &p.kind);
        assert_eq!(kind, DdlKind::CreateIndexConcurrently);
    }

    #[test]
    fn classify_drop_table() {
        let p = parse("DROP TABLE orders");
        let kind = classify_ddl_kind("DROP TABLE orders", &p.kind);
        assert_eq!(kind, DdlKind::DropTable);
    }

    #[test]
    fn classify_truncate() {
        let p = parse("TRUNCATE TABLE orders");
        let kind = classify_ddl_kind("TRUNCATE TABLE orders", &p.kind);
        assert_eq!(kind, DdlKind::Truncate);
    }

    #[test]
    fn classify_alter_add_column_nullable() {
        let p = parse("ALTER TABLE orders ADD COLUMN note TEXT");
        let kind = classify_ddl_kind("ALTER TABLE orders ADD COLUMN note TEXT", &p.kind);
        assert_eq!(kind, DdlKind::AlterTableAddColumnNullable);
    }

    #[test]
    fn classify_alter_add_column_not_null() {
        let p = parse("ALTER TABLE orders ADD COLUMN code TEXT NOT NULL");
        let kind = classify_ddl_kind("ALTER TABLE orders ADD COLUMN code TEXT NOT NULL", &p.kind);
        assert_eq!(kind, DdlKind::AlterTableAddColumnNotNull);
    }

    #[test]
    fn classify_alter_add_column_not_null_default() {
        let p = parse("ALTER TABLE orders ADD COLUMN code TEXT NOT NULL DEFAULT 'x'");
        let kind = classify_ddl_kind(
            "ALTER TABLE orders ADD COLUMN code TEXT NOT NULL DEFAULT 'x'",
            &p.kind,
        );
        assert_eq!(kind, DdlKind::AlterTableAddColumnNotNullDefault);
    }

    #[test]
    fn classify_alter_drop_column() {
        let p = parse("ALTER TABLE orders DROP COLUMN note");
        let kind = classify_ddl_kind("ALTER TABLE orders DROP COLUMN note", &p.kind);
        assert_eq!(kind, DdlKind::AlterTableDropColumn);
    }

    #[test]
    fn classify_alter_column_type() {
        let p = parse("ALTER TABLE orders ALTER COLUMN amount TYPE BIGINT");
        let kind = classify_ddl_kind(
            "ALTER TABLE orders ALTER COLUMN amount TYPE BIGINT",
            &p.kind,
        );
        assert_eq!(kind, DdlKind::AlterTableAlterColumnType);
    }

    // ── Reverse SQL ───────────────────────────────────────────────────────────

    #[test]
    fn reverse_create_table() {
        let p = parse("CREATE TABLE orders (id INT)");
        let kind = classify_ddl_kind("CREATE TABLE orders (id INT)", &p.kind);
        let rev = generate_reverse_sql(kind, "CREATE TABLE orders (id INT)", &p);
        assert!(rev.is_some());
        let rev = rev.unwrap();
        assert!(rev.contains("DROP TABLE"), "got: {rev}");
        assert!(rev.contains("orders"), "got: {rev}");
    }

    #[test]
    fn reverse_create_index() {
        let sql = "CREATE INDEX idx_orders_status ON orders (status)";
        let p = parse(sql);
        let kind = classify_ddl_kind(sql, &p.kind);
        let rev = generate_reverse_sql(kind, sql, &p);
        assert!(rev.is_some());
        let rev = rev.unwrap();
        assert!(rev.contains("DROP INDEX"), "got: {rev}");
        assert!(rev.contains("idx_orders_status"), "got: {rev}");
    }

    #[test]
    fn reverse_create_index_concurrently() {
        let sql = "CREATE INDEX CONCURRENTLY idx_orders_status ON orders (status)";
        let p = parse(sql);
        let kind = classify_ddl_kind(sql, &p.kind);
        let rev = generate_reverse_sql(kind, sql, &p);
        assert!(rev.is_some());
        assert!(rev.unwrap().contains("idx_orders_status"));
    }

    #[test]
    fn reverse_alter_add_column() {
        let sql = "ALTER TABLE orders ADD COLUMN note TEXT";
        let p = parse(sql);
        let kind = classify_ddl_kind(sql, &p.kind);
        let rev = generate_reverse_sql(kind, sql, &p);
        assert!(rev.is_some());
        let rev = rev.unwrap();
        assert!(rev.contains("DROP COLUMN"), "got: {rev}");
        assert!(rev.contains("note"), "got: {rev}");
    }

    #[test]
    fn reverse_drop_table_is_none() {
        let sql = "DROP TABLE orders";
        let p = parse(sql);
        let kind = classify_ddl_kind(sql, &p.kind);
        let rev = generate_reverse_sql(kind, sql, &p);
        assert!(rev.is_none(), "DROP TABLE cannot be reversed");
    }

    #[test]
    fn reverse_truncate_is_none() {
        let sql = "TRUNCATE TABLE orders";
        let p = parse(sql);
        let kind = classify_ddl_kind(sql, &p.kind);
        let rev = generate_reverse_sql(kind, sql, &p);
        assert!(rev.is_none(), "TRUNCATE cannot be reversed");
    }

    // ── Destructive classification ─────────────────────────────────────────────

    #[test]
    fn drop_table_is_destructive() {
        assert!(is_destructive_operation(DdlKind::DropTable));
    }

    #[test]
    fn truncate_is_destructive() {
        assert!(is_destructive_operation(DdlKind::Truncate));
    }

    #[test]
    fn drop_column_is_destructive() {
        assert!(is_destructive_operation(DdlKind::AlterTableDropColumn));
    }

    #[test]
    fn create_table_is_not_destructive() {
        assert!(!is_destructive_operation(DdlKind::CreateTable));
    }

    #[test]
    fn add_column_is_not_destructive() {
        assert!(!is_destructive_operation(
            DdlKind::AlterTableAddColumnNullable
        ));
    }

    // ── Lock type ─────────────────────────────────────────────────────────────

    #[test]
    fn create_index_concurrently_has_lower_lock() {
        let lock = lock_type_for(DdlKind::CreateIndexConcurrently);
        assert!(lock.contains("ShareUpdateExclusiveLock"), "got: {lock}");
    }

    #[test]
    fn create_index_has_share_lock() {
        let lock = lock_type_for(DdlKind::CreateIndex);
        assert!(lock.contains("ShareLock"), "got: {lock}");
    }

    #[test]
    fn drop_table_has_access_exclusive() {
        let lock = lock_type_for(DdlKind::DropTable);
        assert!(lock.contains("AccessExclusiveLock"), "got: {lock}");
    }

    // ── Warning generation ────────────────────────────────────────────────────

    #[test]
    fn create_index_warns_about_writes_blocked() {
        let warnings = generate_warnings(DdlKind::CreateIndex, 160001, "CREATE INDEX i ON t (c)");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("ShareLock") || w.contains("writes")),
            "must warn about write blocking: {warnings:?}"
        );
    }

    #[test]
    fn drop_table_warns_data_loss() {
        let warnings = generate_warnings(DdlKind::DropTable, 160001, "DROP TABLE orders");
        assert!(
            warnings
                .iter()
                .any(|w| w.to_lowercase().contains("data") || w.contains("backup")),
            "must warn about data loss: {warnings:?}"
        );
    }

    #[test]
    fn drop_table_without_if_exists_warns() {
        let warnings = generate_warnings(DdlKind::DropTable, 160001, "DROP TABLE orders");
        assert!(
            warnings.iter().any(|w| w.contains("IF EXISTS")),
            "must warn about missing IF EXISTS: {warnings:?}"
        );
    }

    #[test]
    fn add_not_null_default_on_pg10_warns_about_rewrite() {
        // PG 10 (100013 in version_num format)
        let warnings = generate_warnings(
            DdlKind::AlterTableAddColumnNotNullDefault,
            100013,
            "ALTER TABLE t ADD COLUMN x TEXT NOT NULL DEFAULT 'a'",
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("rewrite") || w.contains("< 11")),
            "must warn about table rewrite on PG < 11: {warnings:?}"
        );
    }

    #[test]
    fn add_not_null_default_on_pg11_no_rewrite_warning() {
        let warnings = generate_warnings(
            DdlKind::AlterTableAddColumnNotNullDefault,
            110000,
            "ALTER TABLE t ADD COLUMN x TEXT NOT NULL DEFAULT 'a'",
        );
        let has_rewrite = warnings.iter().any(|w| w.contains("rewrite"));
        assert!(
            !has_rewrite,
            "should NOT warn about table rewrite on PG 11+: {warnings:?}"
        );
    }

    // ── Suggestion generation ─────────────────────────────────────────────────

    #[test]
    fn create_index_suggests_concurrently() {
        let suggestions =
            generate_suggestions(DdlKind::CreateIndex, 160001, "CREATE INDEX i ON t (c)");
        assert!(
            suggestions.iter().any(|s| s.contains("CONCURRENTLY")),
            "must suggest CONCURRENTLY: {suggestions:?}"
        );
    }

    #[test]
    fn drop_table_suggests_backup() {
        let suggestions = generate_suggestions(DdlKind::DropTable, 160001, "DROP TABLE t");
        assert!(
            suggestions
                .iter()
                .any(|s| s.to_lowercase().contains("backup")),
            "must suggest backup: {suggestions:?}"
        );
    }

    // ── PG version formatting ─────────────────────────────────────────────────

    #[test]
    fn format_pg_version_16_1() {
        // 160001 = major 16, patch 1
        assert_eq!(format_pg_version(160001), "16.1");
    }

    #[test]
    fn format_pg_version_16_0() {
        assert_eq!(format_pg_version(160000), "16");
    }

    #[test]
    fn format_pg_version_14_5() {
        // 140005 = major 14, patch 5
        assert_eq!(format_pg_version(140005), "14.5");
    }

    #[test]
    fn format_pg_version_zero_is_unknown() {
        assert_eq!(format_pg_version(0), "unknown");
    }

    // ── Index name extraction ─────────────────────────────────────────────────

    #[test]
    fn extract_index_name_simple() {
        let name = extract_create_index_name("CREATE INDEX idx_status ON t (status)");
        assert_eq!(name, Some("idx_status".to_string()));
    }

    #[test]
    fn extract_index_name_concurrently() {
        let name = extract_create_index_name("CREATE INDEX CONCURRENTLY idx_status ON t (status)");
        assert_eq!(name, Some("idx_status".to_string()));
    }

    #[test]
    fn extract_index_name_unique() {
        let name = extract_create_index_name("CREATE UNIQUE INDEX idx_email ON users (email)");
        assert_eq!(name, Some("idx_email".to_string()));
    }

    // ── Column name extraction ────────────────────────────────────────────────

    #[test]
    fn extract_add_column_name_simple() {
        let col = extract_add_column_name("ALTER TABLE t ADD COLUMN note TEXT");
        assert_eq!(col, Some("note".to_string()));
    }

    #[test]
    fn extract_add_column_name_with_not_null() {
        let col =
            extract_add_column_name("ALTER TABLE t ADD COLUMN code TEXT NOT NULL DEFAULT 'x'");
        assert_eq!(col, Some("code".to_string()));
    }
}
