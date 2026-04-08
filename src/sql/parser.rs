// src/sql/parser.rs
//
// SQL statement parser for pgmcp.
//
// Wraps `sqlparser` 0.61 with the PostgreSQL dialect. Parses a single SQL
// statement and returns a `ParsedStatement` containing the classified kind and
// metadata extracted from the AST. This module never executes SQL.
//
// Design invariants:
// - Only one statement is accepted per call. Multi-statement input is rejected.
// - The raw `sqlparser::ast::Statement` is retained so downstream modules
//   (guardrails, limit injection) can work on the AST without re-parsing.
// - All errors are returned as `McpError::sql_parse_error`.
// - No panics on any input, including crafted / malformed SQL.

// Items in this module are consumed by `sql::guardrails` and `sql::limit`.
// Dead-code lint fires until those modules are implemented (feat/015, feat/016).
#![allow(dead_code)]

use sqlparser::{
    ast::{CopyTarget, Delete, FromTable, Query, Statement, TableFactor},
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::error::McpError;

// ── StatementKind ─────────────────────────────────────────────────────────────

/// Classification of a SQL statement by its top-level kind.
///
/// Used by guardrails to decide whether to block a statement and by the limit
/// injector to decide whether to append a LIMIT clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementKind {
    /// `SELECT` (including `SELECT … UNION/INTERSECT/EXCEPT` and CTEs).
    Select,
    /// `INSERT INTO …`
    Insert,
    /// `UPDATE … SET …`
    Update,
    /// `DELETE FROM …`
    Delete,
    /// `CREATE TABLE …`
    CreateTable,
    /// `ALTER TABLE …`
    AlterTable,
    /// `DROP TABLE …`
    DropTable,
    /// `CREATE INDEX …`
    CreateIndex,
    /// `DROP INDEX …`
    DropIndex,
    /// `TRUNCATE …`
    Truncate,
    /// `COPY … TO/FROM …`
    Copy,
    /// `SET <variable> = <value>`
    Set,
    /// Anything else (CREATE VIEW, DROP SCHEMA, CALL, etc.).
    Other,
}

// ── ParsedStatement ───────────────────────────────────────────────────────────

/// Metadata extracted from a single parsed SQL statement.
///
/// Produced by [`parse_statement`]. Consumed by the guardrails module and the
/// limit-injection module. The raw `sqlparser` AST is retained so downstream
/// modules can modify and re-serialize the statement without re-parsing.
#[derive(Debug)]
pub(crate) struct ParsedStatement {
    /// Top-level statement classification.
    pub kind: StatementKind,

    /// Table names referenced by the statement (best-effort; not exhaustive
    /// for complex queries with many joins or subqueries).
    pub table_names: Vec<String>,

    /// `true` if the outermost SELECT has an explicit `LIMIT` (or `FETCH FIRST`).
    pub has_limit: bool,

    /// `true` if the statement has a `WHERE` clause at the top level.
    pub has_where: bool,

    /// Highest `$N` parameter placeholder index found in the SQL text.
    /// Zero means no `$N` placeholders were present.
    pub param_count: usize,

    /// `true` when this is a `COPY … TO/FROM PROGRAM '…'` statement.
    pub is_copy_program: bool,

    /// The raw parsed AST node, retained for limit injection.
    pub raw_stmt: Statement,
}

// ── parse_statement ───────────────────────────────────────────────────────────

/// Parse a single SQL statement using the PostgreSQL dialect.
///
/// # Errors
///
/// Returns [`McpError::sql_parse_error`] if:
/// - The input is empty or contains only whitespace.
/// - The input contains more than one semicolon-separated statement.
/// - `sqlparser` fails to parse the input as valid PostgreSQL.
///
/// # Examples
///
/// ```rust,ignore
/// let parsed = parse_statement("SELECT * FROM users WHERE id = $1")?;
/// assert_eq!(parsed.kind, StatementKind::Select);
/// assert!(parsed.has_where);
/// assert_eq!(parsed.param_count, 1);
/// ```
pub(crate) fn parse_statement(sql: &str) -> Result<ParsedStatement, McpError> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(McpError::sql_parse_error("SQL statement must not be empty"));
    }

    let dialect = PostgreSqlDialect {};
    let mut stmts = Parser::parse_sql(&dialect, trimmed)
        .map_err(|e| McpError::sql_parse_error(format!("could not parse SQL: {e}")))?;

    if stmts.len() > 1 {
        return Err(McpError::sql_parse_error(format!(
            "only a single SQL statement is allowed; got {}",
            stmts.len()
        )));
    }
    if stmts.is_empty() {
        return Err(McpError::sql_parse_error("SQL statement must not be empty"));
    }

    let stmt = stmts.remove(0);
    let kind = classify(&stmt);
    let table_names = extract_table_names(&stmt);
    let has_limit = detect_limit(&stmt);
    let has_where = detect_where(&stmt);
    let param_count = count_params(trimmed);
    let is_copy_program = detect_copy_program(&stmt);

    Ok(ParsedStatement {
        kind,
        table_names,
        has_limit,
        has_where,
        param_count,
        is_copy_program,
        raw_stmt: stmt,
    })
}

// ── classify ──────────────────────────────────────────────────────────────────

fn classify(stmt: &Statement) -> StatementKind {
    use sqlparser::ast::ObjectType;
    match stmt {
        Statement::Query(_) => StatementKind::Select,
        Statement::Insert(_) => StatementKind::Insert,
        Statement::Update(_) => StatementKind::Update,
        Statement::Delete(_) => StatementKind::Delete,
        Statement::CreateTable(_) => StatementKind::CreateTable,
        Statement::AlterTable(_) => StatementKind::AlterTable,
        Statement::Drop {
            object_type: ObjectType::Table,
            ..
        } => StatementKind::DropTable,
        Statement::CreateIndex(_) => StatementKind::CreateIndex,
        Statement::Drop {
            object_type: ObjectType::Index,
            ..
        } => StatementKind::DropIndex,
        Statement::Truncate(_) => StatementKind::Truncate,
        Statement::Copy { .. } => StatementKind::Copy,
        Statement::Set(_) => StatementKind::Set,
        _ => StatementKind::Other,
    }
}

// ── extract_table_names ───────────────────────────────────────────────────────

/// Extract the primary table name(s) referenced by a statement.
///
/// This is best-effort: it covers the common cases (SELECT FROM, INSERT INTO,
/// UPDATE, DELETE FROM, DDL). Complex queries with many joins or subqueries
/// may not enumerate every referenced table.
fn extract_table_names(stmt: &Statement) -> Vec<String> {
    match stmt {
        Statement::Query(q) => extract_query_tables(q),

        Statement::Insert(ins) => {
            vec![ins.table.to_string()]
        }

        Statement::Update(upd) => {
            vec![table_with_joins_name(&upd.table)]
        }

        Statement::Delete(del) => extract_delete_tables(del),

        Statement::CreateTable(ct) => vec![ct.name.to_string()],

        Statement::AlterTable(at) => vec![at.name.to_string()],

        Statement::Drop {
            object_type:
                sqlparser::ast::ObjectType::Table
                | sqlparser::ast::ObjectType::View
                | sqlparser::ast::ObjectType::Index,
            names,
            ..
        } => names.iter().map(|n| n.to_string()).collect(),

        Statement::Truncate(tr) => tr.table_names.iter().map(|t| t.name.to_string()).collect(),

        Statement::Copy { source, .. } => {
            use sqlparser::ast::CopySource;
            match source {
                CopySource::Table { table_name, .. } => vec![table_name.to_string()],
                CopySource::Query(q) => extract_query_tables(q),
            }
        }

        _ => vec![],
    }
}

fn extract_query_tables(q: &Query) -> Vec<String> {
    extract_set_expr_tables(&q.body)
}

fn extract_set_expr_tables(expr: &sqlparser::ast::SetExpr) -> Vec<String> {
    use sqlparser::ast::SetExpr;
    match expr {
        SetExpr::Select(sel) => {
            let mut names = Vec::new();
            for item in &sel.from {
                collect_table_factor_name(&item.relation, &mut names);
            }
            names
        }
        SetExpr::SetOperation { left, right, .. } => {
            let mut names = extract_set_expr_tables(left);
            names.extend(extract_set_expr_tables(right));
            names
        }
        SetExpr::Query(q) => extract_query_tables(q),
        _ => vec![],
    }
}

fn collect_table_factor_name(factor: &TableFactor, out: &mut Vec<String>) {
    if let TableFactor::Table { name, .. } = factor {
        out.push(name.to_string());
    }
}

fn table_with_joins_name(t: &sqlparser::ast::TableWithJoins) -> String {
    match &t.relation {
        TableFactor::Table { name, .. } => name.to_string(),
        other => other.to_string(),
    }
}

fn extract_delete_tables(del: &Delete) -> Vec<String> {
    match &del.from {
        FromTable::WithFromKeyword(items) | FromTable::WithoutKeyword(items) => items
            .iter()
            .filter_map(|t| {
                if let TableFactor::Table { name, .. } = &t.relation {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect(),
    }
}

// ── detect_limit ──────────────────────────────────────────────────────────────

/// Returns `true` if the outermost SELECT has an explicit `LIMIT` or `FETCH FIRST`.
fn detect_limit(stmt: &Statement) -> bool {
    let Statement::Query(q) = stmt else {
        return false;
    };
    if q.fetch.is_some() {
        return true;
    }
    match &q.limit_clause {
        None => false,
        Some(sqlparser::ast::LimitClause::LimitOffset { limit, .. }) => {
            // `LIMIT ALL` produces limit: None — treat as "no real limit".
            limit.is_some()
        }
        Some(sqlparser::ast::LimitClause::OffsetCommaLimit { .. }) => true,
    }
}

// ── detect_where ─────────────────────────────────────────────────────────────

/// Returns `true` if the statement has a WHERE clause at the outermost level.
fn detect_where(stmt: &Statement) -> bool {
    match stmt {
        Statement::Query(q) => query_has_where(q),
        Statement::Update(upd) => upd.selection.is_some(),
        Statement::Delete(del) => del.selection.is_some(),
        _ => false,
    }
}

fn query_has_where(q: &Query) -> bool {
    use sqlparser::ast::SetExpr;
    match q.body.as_ref() {
        SetExpr::Select(sel) => sel.selection.is_some(),
        SetExpr::Query(inner) => query_has_where(inner),
        _ => false,
    }
}

// ── count_params ──────────────────────────────────────────────────────────────

/// Count `$N` positional parameter placeholders in the raw SQL text.
///
/// Returns the highest N found. `$1`, `$2`, `$3` → 3.
/// Returns 0 if no `$N` placeholders are present.
fn count_params(sql: &str) -> usize {
    let mut max_n: usize = 0;
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                if let Ok(n) = sql[start..end].parse::<usize>() {
                    if n > max_n {
                        max_n = n;
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    max_n
}

// ── detect_copy_program ───────────────────────────────────────────────────────

/// Returns `true` if this is a `COPY … TO/FROM PROGRAM '…'` statement.
fn detect_copy_program(stmt: &Statement) -> bool {
    match stmt {
        Statement::Copy { target, .. } => matches!(target, CopyTarget::Program { .. }),
        _ => false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(sql: &str) -> ParsedStatement {
        parse_statement(sql).expect("expected parse to succeed")
    }

    fn parse_err(sql: &str) -> McpError {
        parse_statement(sql).expect_err("expected parse to fail")
    }

    // ── StatementKind classification ─────────────────────────────────────

    #[test]
    fn parse_select_simple() {
        let p = parse("SELECT 1");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_select_case_insensitive() {
        let p = parse("select * from users");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_select_leading_trailing_whitespace() {
        let p = parse("  SELECT 1  ");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_insert() {
        let p = parse("INSERT INTO users (name) VALUES ('alice')");
        assert_eq!(p.kind, StatementKind::Insert);
    }

    #[test]
    fn parse_update_with_where() {
        let p = parse("UPDATE users SET name='bob' WHERE id=1");
        assert_eq!(p.kind, StatementKind::Update);
        assert!(p.has_where, "UPDATE with WHERE should have has_where=true");
    }

    #[test]
    fn parse_update_without_where() {
        let p = parse("UPDATE users SET active=false");
        assert_eq!(p.kind, StatementKind::Update);
        assert!(
            !p.has_where,
            "UPDATE without WHERE should have has_where=false"
        );
    }

    #[test]
    fn parse_delete_with_where() {
        let p = parse("DELETE FROM users WHERE id=1");
        assert_eq!(p.kind, StatementKind::Delete);
        assert!(p.has_where, "DELETE with WHERE should have has_where=true");
    }

    #[test]
    fn parse_delete_without_where() {
        let p = parse("DELETE FROM users");
        assert_eq!(p.kind, StatementKind::Delete);
        assert!(
            !p.has_where,
            "DELETE without WHERE should have has_where=false"
        );
    }

    #[test]
    fn parse_create_table() {
        let p = parse("CREATE TABLE orders (id SERIAL PRIMARY KEY, amount NUMERIC)");
        assert_eq!(p.kind, StatementKind::CreateTable);
    }

    #[test]
    fn parse_alter_table() {
        let p = parse("ALTER TABLE orders ADD COLUMN note TEXT");
        assert_eq!(p.kind, StatementKind::AlterTable);
    }

    #[test]
    fn parse_drop_table() {
        let p = parse("DROP TABLE IF EXISTS orders");
        assert_eq!(p.kind, StatementKind::DropTable);
    }

    #[test]
    fn parse_create_index() {
        let p = parse("CREATE INDEX idx_orders_amount ON orders (amount)");
        assert_eq!(p.kind, StatementKind::CreateIndex);
    }

    #[test]
    fn parse_drop_index() {
        let p = parse("DROP INDEX idx_orders_amount");
        assert_eq!(p.kind, StatementKind::DropIndex);
    }

    #[test]
    fn parse_truncate() {
        let p = parse("TRUNCATE TABLE orders");
        assert_eq!(p.kind, StatementKind::Truncate);
    }

    #[test]
    fn parse_copy_stdin() {
        // sqlparser requires a semicolon after STDIN to mark the end of the
        // COPY statement (it then reads TSV data until `\.`).
        // We supply an empty payload (just the semicolon terminator).
        let p = parse("COPY orders FROM STDIN;");
        assert_eq!(p.kind, StatementKind::Copy);
        assert!(!p.is_copy_program, "COPY FROM STDIN is not a program");
    }

    #[test]
    fn parse_copy_program() {
        let p = parse("COPY orders FROM PROGRAM 'cat /tmp/data.csv'");
        assert_eq!(p.kind, StatementKind::Copy);
        assert!(
            p.is_copy_program,
            "COPY FROM PROGRAM should set is_copy_program"
        );
    }

    #[test]
    fn parse_copy_to_stdout() {
        let p = parse("COPY orders TO STDOUT");
        assert_eq!(p.kind, StatementKind::Copy);
        assert!(!p.is_copy_program);
    }

    #[test]
    fn parse_set_statement() {
        let p = parse("SET statement_timeout = '5s'");
        assert_eq!(p.kind, StatementKind::Set);
    }

    #[test]
    fn parse_set_role() {
        let p = parse("SET ROLE analyst");
        assert_eq!(p.kind, StatementKind::Set);
    }

    // ── has_limit ─────────────────────────────────────────────────────────

    #[test]
    fn parse_select_with_limit() {
        let p = parse("SELECT * FROM users LIMIT 10");
        assert!(p.has_limit, "SELECT with LIMIT should have has_limit=true");
    }

    #[test]
    fn parse_select_without_limit() {
        let p = parse("SELECT * FROM users");
        assert!(
            !p.has_limit,
            "SELECT without LIMIT should have has_limit=false"
        );
    }

    #[test]
    fn parse_select_with_fetch_first() {
        let p = parse("SELECT * FROM users FETCH FIRST 5 ROWS ONLY");
        assert!(p.has_limit, "FETCH FIRST counts as a limit");
    }

    #[test]
    fn parse_select_limit_all_treated_as_no_limit() {
        let p = parse("SELECT * FROM users LIMIT ALL");
        // LIMIT ALL means no effective limit — has_limit should be false.
        assert!(
            !p.has_limit,
            "LIMIT ALL should be treated as no effective limit"
        );
    }

    // ── has_where ─────────────────────────────────────────────────────────

    #[test]
    fn parse_select_with_where() {
        let p = parse("SELECT * FROM users WHERE id = 1");
        assert!(p.has_where, "SELECT with WHERE should have has_where=true");
    }

    #[test]
    fn parse_select_without_where() {
        let p = parse("SELECT * FROM users");
        assert!(
            !p.has_where,
            "SELECT without WHERE should have has_where=false"
        );
    }

    // ── param_count ───────────────────────────────────────────────────────

    #[test]
    fn param_count_single() {
        let p = parse("SELECT * FROM users WHERE id = $1");
        assert_eq!(p.param_count, 1);
    }

    #[test]
    fn param_count_multiple() {
        let p = parse("SELECT * FROM users WHERE id = $1 AND name = $2 AND age = $3");
        assert_eq!(p.param_count, 3);
    }

    #[test]
    fn param_count_none() {
        let p = parse("SELECT 1");
        assert_eq!(p.param_count, 0);
    }

    #[test]
    fn param_count_noncontiguous() {
        // $1 and $5 — max should be 5.
        let p = parse("SELECT $1, $5");
        assert_eq!(p.param_count, 5);
    }

    // ── table_names ───────────────────────────────────────────────────────

    #[test]
    fn table_names_select_simple() {
        let p = parse("SELECT * FROM orders");
        assert!(
            p.table_names.iter().any(|n| n.contains("orders")),
            "table_names should contain 'orders', got {:?}",
            p.table_names
        );
    }

    #[test]
    fn table_names_insert() {
        let p = parse("INSERT INTO orders (amount) VALUES (100)");
        assert!(p.table_names.iter().any(|n| n.contains("orders")));
    }

    #[test]
    fn table_names_update() {
        let p = parse("UPDATE orders SET amount = 200 WHERE id = 1");
        assert!(p.table_names.iter().any(|n| n.contains("orders")));
    }

    #[test]
    fn table_names_delete() {
        let p = parse("DELETE FROM orders WHERE id = 1");
        assert!(p.table_names.iter().any(|n| n.contains("orders")));
    }

    #[test]
    fn table_names_create_table() {
        let p = parse("CREATE TABLE items (id INT)");
        assert!(p.table_names.iter().any(|n| n.contains("items")));
    }

    #[test]
    fn table_names_drop_table() {
        let p = parse("DROP TABLE items");
        assert!(p.table_names.iter().any(|n| n.contains("items")));
    }

    // ── error cases ───────────────────────────────────────────────────────

    #[test]
    fn parse_empty_string() {
        let err = parse_err("");
        assert_eq!(err.code(), "sql_parse_error");
    }

    #[test]
    fn parse_whitespace_only() {
        let err = parse_err("   \t\n  ");
        assert_eq!(err.code(), "sql_parse_error");
    }

    #[test]
    fn parse_malformed_sql() {
        // Genuinely unparseable SQL — keyword mismatch at start.
        let err = parse_err("SELEKT * FORM users");
        assert_eq!(err.code(), "sql_parse_error");
    }

    #[test]
    fn parse_multi_statement_rejected() {
        let err = parse_err("SELECT 1; SELECT 2");
        assert_eq!(err.code(), "sql_parse_error");
        assert!(
            err.message().contains("single"),
            "error should mention single statement requirement"
        );
    }

    #[test]
    fn parse_multi_statement_three_rejected() {
        let err = parse_err("SELECT 1; INSERT INTO t VALUES (1); SELECT 2");
        assert_eq!(err.code(), "sql_parse_error");
    }

    // ── complex queries ───────────────────────────────────────────────────

    #[test]
    fn parse_with_cte() {
        let p = parse("WITH cte AS (SELECT 1 AS n) SELECT n FROM cte");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_union() {
        let p = parse("SELECT 1 UNION ALL SELECT 2");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_subquery_in_from() {
        let p = parse("SELECT * FROM (SELECT id FROM users) sub");
        assert_eq!(p.kind, StatementKind::Select);
    }

    #[test]
    fn parse_select_order_by_limit() {
        let p = parse("SELECT * FROM users ORDER BY id DESC LIMIT 20");
        assert_eq!(p.kind, StatementKind::Select);
        assert!(p.has_limit);
    }
}
