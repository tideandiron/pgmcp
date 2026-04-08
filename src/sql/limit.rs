// src/sql/limit.rs
//
// LIMIT clause injection for SELECT statements.
//
// Injects or caps the LIMIT clause of a SELECT statement. Non-SELECT statements
// are returned unchanged. This module never executes SQL — it is a pure AST
// transformation.
//
// Design invariants:
// - Only the outermost SELECT query is modified. Subquery SELECTs, CTE bodies,
//   and UNION branches are untouched.
// - The injected LIMIT is a simple integer literal.
// - Re-serialisation is via the sqlparser `Display` impl on `Query`, which
//   produces valid PostgreSQL SQL.
// - All errors propagate as `McpError::sql_parse_error`.
// - No SQL is executed here.

// Items in this module are consumed by the query tool handler (feat/018).
// Dead-code lint fires until the query tool integrates this layer.
#![allow(dead_code)]

use sqlparser::ast::{Expr, LimitClause, Query, Statement, Value, ValueWithSpan};

use crate::{error::McpError, sql::parser::parse_statement};

// ── inject_limit ──────────────────────────────────────────────────────────────

/// Inject or cap the `LIMIT` clause of a SELECT statement.
///
/// # Behaviour
///
/// | Input | Output |
/// |---|---|
/// | Non-SELECT | `(sql, false)` — unchanged |
/// | SELECT without LIMIT | `(sql + "LIMIT {default_limit}", true)` |
/// | SELECT with `LIMIT N` where N ≤ max_limit | `(sql, false)` — unchanged |
/// | SELECT with `LIMIT N` where N > max_limit | `(capped_sql, false)` |
/// | SELECT with `LIMIT ALL` | `(sql + "LIMIT {default_limit}", true)` |
///
/// Subquery SELECTs, CTE bodies, and UNION sub-queries are **not** modified;
/// only the outermost `Query` node is touched.
///
/// # Parameters
///
/// - `sql`: raw SQL string to analyse and potentially modify.
/// - `default_limit`: limit to inject when the statement has none.
/// - `max_limit`: maximum allowed limit value; existing limits above this are
///   capped to `max_limit`.
///
/// # Returns
///
/// `Ok((modified_sql, was_injected))` where `was_injected` is `true` if a new
/// LIMIT was added (not merely capped or left unchanged).
///
/// # Errors
///
/// Returns [`McpError::sql_parse_error`] if the SQL cannot be parsed.
///
/// # Examples
///
/// ```rust,ignore
/// let (sql, injected) = inject_limit("SELECT * FROM users", 100, 1000)?;
/// assert!(injected);
/// assert!(sql.contains("LIMIT 100"));
///
/// let (sql, injected) = inject_limit("SELECT * FROM users LIMIT 500", 100, 1000)?;
/// assert!(!injected);
/// assert!(sql.contains("LIMIT 500")); // within max
///
/// let (sql, injected) = inject_limit("SELECT * FROM users LIMIT 5000", 100, 1000)?;
/// assert!(!injected);
/// assert!(sql.contains("LIMIT 1000")); // capped
/// ```
pub(crate) fn inject_limit(
    sql: &str,
    default_limit: u32,
    max_limit: u32,
) -> Result<(String, bool), McpError> {
    use crate::sql::parser::StatementKind;

    let parsed = parse_statement(sql)?;

    // Non-SELECT statements: return unchanged.
    if parsed.kind != StatementKind::Select {
        return Ok((sql.to_string(), false));
    }

    // Extract the Query from the Statement::Query variant.
    let Statement::Query(mut query) = parsed.raw_stmt else {
        // parse_statement returned Select but raw_stmt is not Query — shouldn't happen.
        return Ok((sql.to_string(), false));
    };

    let was_injected = apply_limit(&mut query, default_limit, max_limit);
    Ok((query.to_string(), was_injected))
}

// ── apply_limit ───────────────────────────────────────────────────────────────

/// Mutate the top-level `Query` to have an appropriate LIMIT clause.
///
/// Returns `true` if a new limit was injected (not merely capped or unchanged).
fn apply_limit(query: &mut Query, default_limit: u32, max_limit: u32) -> bool {
    match &query.limit_clause {
        None => {
            // No LIMIT at all — inject default.
            query.limit_clause = Some(make_limit_clause(default_limit));
            true
        }

        Some(LimitClause::LimitOffset {
            limit: None, // LIMIT ALL or bare LIMIT without a value
            ..
        }) => {
            // LIMIT ALL semantically means no limit — treat as absent.
            query.limit_clause = Some(make_limit_clause(default_limit));
            true
        }

        Some(LimitClause::LimitOffset {
            limit: Some(Expr::Value(vs)),
            ..
        }) => {
            // Parse the numeric value and cap if necessary.
            if let Some(n) = extract_u64_from_value(&vs.value) {
                let capped = n.min(u64::from(max_limit));
                if capped != n {
                    // Need to replace with capped value.
                    let vs_clone = vs.clone();
                    if let Some(LimitClause::LimitOffset {
                        limit: Some(Expr::Value(ref mut vw)),
                        ..
                    }) = query.limit_clause
                    {
                        *vw = ValueWithSpan {
                            value: Value::Number(capped.to_string(), false),
                            span: vs_clone.span,
                        };
                    }
                }
            }
            // Whether we capped or not, was_injected = false (limit existed).
            false
        }

        Some(LimitClause::OffsetCommaLimit { limit, .. }) => {
            // MySQL-style `LIMIT offset, limit`. Extract and cap.
            if let Expr::Value(vs) = limit {
                if let Some(n) = extract_u64_from_value(&vs.value) {
                    let capped = n.min(u64::from(max_limit));
                    if capped != n {
                        let span = vs.span;
                        if let Some(LimitClause::OffsetCommaLimit {
                            limit: Expr::Value(ref mut vw),
                            ..
                        }) = query.limit_clause
                        {
                            *vw = ValueWithSpan {
                                value: Value::Number(capped.to_string(), false),
                                span,
                            };
                        }
                    }
                }
            }
            false
        }

        // Any other LIMIT expression (computed, subquery, etc.) — leave unchanged.
        Some(_) => false,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Construct a standard `LIMIT {n}` clause.
fn make_limit_clause(n: u32) -> LimitClause {
    LimitClause::LimitOffset {
        limit: Some(Expr::Value(
            Value::Number(n.to_string(), false).with_empty_span(),
        )),
        offset: None,
        limit_by: vec![],
    }
}

/// Extract a `u64` from a `Value::Number` string representation.
/// Returns `None` if the value is not a simple integer.
fn extract_u64_from_value(v: &Value) -> Option<u64> {
    match v {
        Value::Number(s, _) => s.parse::<u64>().ok(),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn inject(sql: &str) -> (String, bool) {
        inject_limit(sql, 100, 1000).expect("inject_limit should not fail")
    }

    fn inject_with(sql: &str, default: u32, max: u32) -> (String, bool) {
        inject_limit(sql, default, max).expect("inject_limit should not fail")
    }

    // ── No LIMIT → inject default ─────────────────────────────────────────

    #[test]
    fn no_limit_select_injects_default() {
        let (sql, injected) = inject("SELECT * FROM users");
        assert!(injected, "should inject LIMIT");
        assert!(
            sql.contains("LIMIT 100"),
            "injected SQL should contain 'LIMIT 100', got: {sql}"
        );
    }

    #[test]
    fn no_limit_complex_select_injects_default() {
        let (sql, injected) = inject("SELECT id, name FROM users WHERE active = true");
        assert!(injected);
        assert!(sql.contains("LIMIT 100"), "got: {sql}");
    }

    #[test]
    fn no_limit_cte_select_injects_at_outer() {
        let (sql, injected) = inject("WITH cte AS (SELECT 1 AS n) SELECT n FROM cte");
        assert!(injected, "CTE outer SELECT should get LIMIT injected");
        assert!(
            sql.contains("LIMIT 100"),
            "LIMIT should appear in output, got: {sql}"
        );
    }

    #[test]
    fn no_limit_union_select_injects_once() {
        let (sql, injected) = inject("SELECT 1 UNION ALL SELECT 2");
        assert!(injected, "UNION should get LIMIT at outermost level");
        assert!(sql.contains("LIMIT 100"), "got: {sql}");
    }

    // ── LIMIT below max → unchanged ───────────────────────────────────────

    #[test]
    fn existing_limit_below_max_unchanged() {
        let (sql, injected) = inject("SELECT * FROM users LIMIT 5");
        assert!(!injected, "should not inject if LIMIT already present");
        assert!(
            sql.contains("LIMIT 5"),
            "original LIMIT should be preserved, got: {sql}"
        );
        assert!(!sql.contains("LIMIT 100"), "default should not appear");
    }

    #[test]
    fn existing_limit_equals_max_unchanged() {
        let (sql, injected) = inject_with("SELECT * FROM users LIMIT 1000", 100, 1000);
        assert!(!injected);
        assert!(sql.contains("LIMIT 1000"), "got: {sql}");
    }

    // ── LIMIT above max → capped ──────────────────────────────────────────

    #[test]
    fn existing_limit_above_max_is_capped() {
        let (sql, injected) = inject_with("SELECT * FROM users LIMIT 5000", 100, 1000);
        assert!(!injected, "capping is not injection");
        assert!(
            sql.contains("LIMIT 1000"),
            "limit should be capped to max, got: {sql}"
        );
        assert!(
            !sql.contains("LIMIT 5000"),
            "original over-limit should not appear"
        );
    }

    #[test]
    fn limit_slightly_above_max_is_capped() {
        let (sql, _) = inject_with("SELECT * FROM users LIMIT 1001", 100, 1000);
        assert!(sql.contains("LIMIT 1000"), "got: {sql}");
    }

    // ── Non-SELECT → unchanged ────────────────────────────────────────────

    #[test]
    fn insert_returned_unchanged() {
        let original = "INSERT INTO users (name) VALUES ('alice')";
        let (sql, injected) = inject(original);
        assert!(!injected);
        // The returned SQL should represent the same statement (formatting may differ).
        assert!(!sql.contains("LIMIT"), "INSERT must not get LIMIT");
    }

    #[test]
    fn delete_with_where_returned_unchanged() {
        let (sql, injected) = inject("DELETE FROM users WHERE id = 1");
        assert!(!injected);
        assert!(!sql.contains("LIMIT"), "DELETE must not get LIMIT");
    }

    #[test]
    fn update_with_where_returned_unchanged() {
        let (sql, injected) = inject("UPDATE users SET name = 'bob' WHERE id = 1");
        assert!(!injected);
        assert!(!sql.contains("LIMIT"), "UPDATE must not get LIMIT");
    }

    // ── LIMIT ALL → treated as no limit ───────────────────────────────────

    #[test]
    fn limit_all_treated_as_no_limit() {
        let (sql, injected) = inject("SELECT * FROM users LIMIT ALL");
        assert!(injected, "LIMIT ALL should be replaced with default limit");
        assert!(
            sql.contains("LIMIT 100"),
            "LIMIT ALL should become LIMIT 100, got: {sql}"
        );
        assert!(
            !sql.to_uppercase().contains("LIMIT ALL"),
            "LIMIT ALL should be gone"
        );
    }

    // ── ORDER BY is preserved ─────────────────────────────────────────────

    #[test]
    fn order_by_preserved_when_limit_injected() {
        let (sql, injected) = inject("SELECT * FROM users ORDER BY id DESC");
        assert!(injected);
        // ORDER BY must appear before LIMIT.
        let order_pos = sql
            .to_uppercase()
            .find("ORDER BY")
            .expect("ORDER BY missing");
        let limit_pos = sql.to_uppercase().find("LIMIT").expect("LIMIT missing");
        assert!(
            order_pos < limit_pos,
            "ORDER BY must come before LIMIT, got: {sql}"
        );
    }

    #[test]
    fn order_by_limit_select_unchanged() {
        let (sql, injected) = inject("SELECT * FROM users ORDER BY id ASC LIMIT 20");
        assert!(!injected);
        assert!(sql.contains("LIMIT 20"), "got: {sql}");
    }

    // ── Subquery handling ─────────────────────────────────────────────────

    #[test]
    fn subquery_in_from_does_not_get_inner_limit() {
        // Only the outer SELECT should get LIMIT; the subquery SELECT should not.
        let (sql, injected) = inject("SELECT * FROM (SELECT id FROM users) sub");
        assert!(injected, "outer SELECT should get LIMIT");
        assert!(
            sql.contains("LIMIT 100"),
            "outer LIMIT should be present, got: {sql}"
        );
        // The inner `(SELECT id FROM users)` subquery should NOT have a LIMIT.
        // We verify by checking the SQL before the closing paren.
        let paren_pos = sql.find(')').expect("subquery paren must be present");
        let inner = &sql[..paren_pos];
        assert!(
            !inner.to_uppercase().contains("LIMIT"),
            "inner subquery must not get LIMIT, got inner: {inner}"
        );
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[test]
    fn default_limit_zero_is_injected() {
        let (sql, injected) = inject_with("SELECT 1", 0, 1000);
        assert!(injected);
        assert!(sql.contains("LIMIT 0"), "got: {sql}");
    }

    #[test]
    fn parse_error_propagated() {
        let result = inject_limit("SELEKT BROKEN SQL", 100, 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "sql_parse_error");
    }

    #[test]
    fn empty_string_returns_parse_error() {
        let result = inject_limit("", 100, 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "sql_parse_error");
    }

    #[test]
    fn select_with_where_gets_limit() {
        let (sql, injected) = inject("SELECT * FROM orders WHERE status = 'open'");
        assert!(injected);
        assert!(sql.contains("LIMIT 100"), "got: {sql}");
    }

    #[test]
    fn custom_default_and_max() {
        let (sql, injected) = inject_with("SELECT * FROM t", 50, 200);
        assert!(injected);
        assert!(sql.contains("LIMIT 50"), "got: {sql}");
    }

    #[test]
    fn custom_max_caps_existing_large_limit() {
        let (sql, injected) = inject_with("SELECT * FROM t LIMIT 300", 50, 200);
        assert!(!injected);
        assert!(sql.contains("LIMIT 200"), "got: {sql}");
    }
}
