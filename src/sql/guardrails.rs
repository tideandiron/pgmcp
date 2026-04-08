// src/sql/guardrails.rs
//
// Guardrail evaluation for the SQL analysis layer.
//
// Evaluates a set of policy rules against a `ParsedStatement` produced by
// `sql::parser`. Returns `Ok(())` when the statement is allowed, or
// `Err(McpError::guardrail_violation(...))` with a descriptive reason when
// blocked.
//
// Design invariants:
// - Guardrail checks are pure: they do not acquire pool connections.
// - Every rule has a corresponding test for both the allow and block path.
// - Rules 4 and 5 are always-on (not configurable).
// - Rule error messages are agent-readable — no internal Rust details.
//
// Rules:
//   1. Block DDL (configurable): CREATE, ALTER, DROP, TRUNCATE
//   2. Block COPY PROGRAM (configurable): COPY TO/FROM PROGRAM
//   3. Block session SET (configurable): SET statements
//   4. Unguarded DELETE/UPDATE (always): DELETE or UPDATE without WHERE
//   5. System table DDL (always): DDL targeting pg_* or information_schema tables

// Items in this module are consumed by tool handlers in feat/017+.
// Dead-code lint fires until the query tool integrates the guardrail layer.
#![allow(dead_code)]

use crate::{
    error::McpError,
    sql::parser::{ParsedStatement, StatementKind},
};

// ── GuardrailConfig ───────────────────────────────────────────────────────────

/// Guardrail policy configuration.
///
/// Governs which categories of SQL statement are blocked by the guardrail
/// layer. Rules 4 and 5 are unconditional and are not exposed here.
#[derive(Debug, Clone)]
pub(crate) struct GuardrailConfig {
    /// Block DDL statements: CREATE TABLE, ALTER TABLE, DROP TABLE,
    /// CREATE INDEX, DROP INDEX, TRUNCATE.
    pub block_ddl: bool,

    /// Block `COPY … TO/FROM PROGRAM '…'` statements. These can execute
    /// arbitrary shell commands on the database server.
    pub block_copy_program: bool,

    /// Block `SET <variable>` statements. These modify session state that
    /// affects all subsequent callers sharing the same connection.
    pub block_session_set: bool,
}

impl Default for GuardrailConfig {
    /// Secure defaults: all configurable rules enabled.
    fn default() -> Self {
        Self {
            block_ddl: true,
            block_copy_program: true,
            block_session_set: true,
        }
    }
}

// ── DDL kinds ─────────────────────────────────────────────────────────────────

/// Returns `true` if the statement kind is a DDL operation.
fn is_ddl(kind: StatementKind) -> bool {
    matches!(
        kind,
        StatementKind::CreateTable
            | StatementKind::AlterTable
            | StatementKind::DropTable
            | StatementKind::CreateIndex
            | StatementKind::DropIndex
            | StatementKind::Truncate
    )
}

// ── System table detection ────────────────────────────────────────────────────

/// Returns `true` if any of the table names look like system catalog objects.
///
/// System tables are: any name starting with `pg_` (with any optional
/// schema prefix) or tables in the `information_schema` schema.
fn targets_system_table(table_names: &[String]) -> bool {
    table_names.iter().any(|name| {
        // Strip any schema prefix to get the base name.
        // sqlparser formats qualified names as "schema.table".
        let base = name.rsplit('.').next().unwrap_or(name.as_str());
        let schema = name.split('.').next().unwrap_or("");

        base.starts_with("pg_")
            || schema == "pg_catalog"
            || schema == "information_schema"
            || name.starts_with("pg_")
    })
}

// ── check ─────────────────────────────────────────────────────────────────────

/// Evaluate all guardrail rules for a parsed statement.
///
/// # Rules
///
/// 1. **Block DDL** (`config.block_ddl`): blocks CREATE TABLE, ALTER TABLE,
///    DROP TABLE, CREATE INDEX, DROP INDEX, TRUNCATE.
/// 2. **Block COPY PROGRAM** (`config.block_copy_program`): blocks
///    `COPY … TO/FROM PROGRAM '…'`.
/// 3. **Block session SET** (`config.block_session_set`): blocks any `SET`
///    statement that would modify session state.
/// 4. **Unguarded DELETE/UPDATE** (always): blocks DELETE or UPDATE statements
///    that lack a WHERE clause. This prevents accidental full-table modifications.
/// 5. **System table DDL** (always): blocks DDL targeting `pg_*` or
///    `information_schema` tables regardless of the DDL rule setting.
///
/// # Errors
///
/// Returns [`McpError::guardrail_violation`] with a descriptive reason for
/// every rule violation.
///
/// # Examples
///
/// ```rust,ignore
/// let config = GuardrailConfig::default();
/// let parsed = parse_statement("DROP TABLE users")?;
/// let result = check(&parsed, &config);
/// assert!(result.is_err()); // blocked by rule 1
/// ```
pub(crate) fn check(parsed: &ParsedStatement, config: &GuardrailConfig) -> Result<(), McpError> {
    // ── Rule 5 (always): system table DDL ────────────────────────────────
    // Check before rule 1 so the error message is more specific.
    if is_ddl(parsed.kind) && targets_system_table(&parsed.table_names) {
        return Err(McpError::guardrail_violation(
            "DDL targeting system catalog tables (pg_* or information_schema) is always \
             blocked. System tables are managed by PostgreSQL and must not be modified \
             directly.",
        ));
    }

    // ── Rule 1 (configurable): block DDL ─────────────────────────────────
    if config.block_ddl && is_ddl(parsed.kind) {
        return Err(McpError::guardrail_violation(
            "DDL statement (CREATE/ALTER/DROP/TRUNCATE) is blocked by the guardrail policy. \
             Use the propose_migration tool to generate and review schema changes safely.",
        ));
    }

    // ── Rule 2 (configurable): block COPY PROGRAM ────────────────────────
    if config.block_copy_program && parsed.is_copy_program {
        return Err(McpError::guardrail_violation(
            "COPY TO/FROM PROGRAM is blocked because it can execute arbitrary shell commands \
             on the database server. Use COPY with a file path or STDIN/STDOUT instead.",
        ));
    }

    // ── Rule 3 (configurable): block session SET ──────────────────────────
    if config.block_session_set && parsed.kind == StatementKind::Set {
        return Err(McpError::guardrail_violation(
            "SET statement is blocked because it modifies session-level parameters that \
             affect all subsequent callers sharing the same connection pool slot.",
        ));
    }

    // ── Rule 4 (always): unguarded DELETE/UPDATE ──────────────────────────
    let is_unguarded_mutation =
        matches!(parsed.kind, StatementKind::Delete | StatementKind::Update) && !parsed.has_where;
    if is_unguarded_mutation {
        return Err(McpError::guardrail_violation(
            "DELETE or UPDATE without a WHERE clause is blocked to prevent accidental \
             full-table modifications. Add a WHERE clause to target specific rows.",
        ));
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::parser::parse_statement;

    fn default_config() -> GuardrailConfig {
        GuardrailConfig::default()
    }

    fn permissive_config() -> GuardrailConfig {
        GuardrailConfig {
            block_ddl: false,
            block_copy_program: false,
            block_session_set: false,
        }
    }

    // Helper: parse and check in one call.
    fn evaluate(sql: &str, config: &GuardrailConfig) -> Result<(), McpError> {
        let parsed = parse_statement(sql).expect("test SQL must parse cleanly");
        check(&parsed, config)
    }

    // ── Rule 1: Block DDL ─────────────────────────────────────────────────

    #[test]
    fn block_create_table_ddl() {
        let result = evaluate("CREATE TABLE orders (id INT)", &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "guardrail_violation");
        assert!(err.message().to_lowercase().contains("ddl"));
    }

    #[test]
    fn block_alter_table_ddl() {
        let result = evaluate("ALTER TABLE orders ADD COLUMN note TEXT", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn block_drop_table_ddl() {
        let result = evaluate("DROP TABLE orders", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn block_truncate_ddl() {
        let result = evaluate("TRUNCATE TABLE orders", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn block_create_index_ddl() {
        let result = evaluate("CREATE INDEX idx ON orders (amount)", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn block_drop_index_ddl() {
        let result = evaluate("DROP INDEX idx_orders_amount", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn allow_ddl_when_block_ddl_false() {
        // block_ddl=false → DDL on a normal table is allowed.
        let config = GuardrailConfig {
            block_ddl: false,
            ..default_config()
        };
        let result = evaluate("CREATE TABLE new_table (id INT)", &config);
        assert!(result.is_ok(), "DDL should be allowed when block_ddl=false");
    }

    // ── Rule 2: Block COPY PROGRAM ────────────────────────────────────────

    #[test]
    fn block_copy_from_program() {
        let result = evaluate(
            "COPY orders FROM PROGRAM 'cat /etc/passwd'",
            &default_config(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "guardrail_violation");
        assert!(err.message().to_lowercase().contains("program"));
    }

    #[test]
    fn block_copy_to_program() {
        let result = evaluate(
            "COPY orders TO PROGRAM 'cat > /tmp/dump.csv'",
            &default_config(),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn allow_copy_stdin_when_block_copy_program_enabled() {
        // COPY FROM STDIN is not a PROGRAM invocation — must be allowed.
        let result = evaluate("COPY orders FROM STDIN;", &default_config());
        assert!(
            result.is_ok(),
            "COPY FROM STDIN should be allowed when block_copy_program=true"
        );
    }

    #[test]
    fn allow_copy_stdout_when_block_copy_program_enabled() {
        let result = evaluate("COPY orders TO STDOUT", &default_config());
        assert!(result.is_ok(), "COPY TO STDOUT should be allowed");
    }

    #[test]
    fn allow_copy_program_when_flag_false() {
        let config = GuardrailConfig {
            block_copy_program: false,
            ..default_config()
        };
        let result = evaluate("COPY orders FROM PROGRAM 'echo hi'", &config);
        assert!(
            result.is_ok(),
            "COPY PROGRAM should be allowed when block_copy_program=false"
        );
    }

    // ── Rule 3: Block session SET ─────────────────────────────────────────

    #[test]
    fn block_set_statement_timeout() {
        let result = evaluate("SET statement_timeout = '5s'", &default_config());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "guardrail_violation");
        assert!(err.message().to_lowercase().contains("set"));
    }

    #[test]
    fn block_set_role() {
        let result = evaluate("SET ROLE analyst", &default_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn allow_set_when_flag_false() {
        let config = GuardrailConfig {
            block_session_set: false,
            ..default_config()
        };
        let result = evaluate("SET statement_timeout = '30s'", &config);
        assert!(
            result.is_ok(),
            "SET should be allowed when block_session_set=false"
        );
    }

    // ── Rule 4: Unguarded DELETE/UPDATE (always) ──────────────────────────

    #[test]
    fn block_delete_without_where() {
        // Even with a fully permissive config, unguarded DELETE is blocked.
        let result = evaluate("DELETE FROM orders", &permissive_config());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "guardrail_violation");
        assert!(
            err.message().to_lowercase().contains("where"),
            "error should mention WHERE clause"
        );
    }

    #[test]
    fn block_update_without_where() {
        let result = evaluate("UPDATE orders SET amount = 0", &permissive_config());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn allow_delete_with_where() {
        let result = evaluate("DELETE FROM orders WHERE id = 1", &default_config());
        assert!(result.is_ok(), "DELETE with WHERE should pass guardrails");
    }

    #[test]
    fn allow_update_with_where() {
        let result = evaluate(
            "UPDATE orders SET amount = 100 WHERE id = 1",
            &default_config(),
        );
        assert!(result.is_ok(), "UPDATE with WHERE should pass guardrails");
    }

    // ── Rule 5: System table DDL (always) ────────────────────────────────

    #[test]
    fn block_create_pg_table() {
        // Even with block_ddl=false, DDL on pg_* tables is blocked.
        let config = GuardrailConfig {
            block_ddl: false,
            ..default_config()
        };
        let result = evaluate("CREATE TABLE pg_custom (id INT)", &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "guardrail_violation");
        assert!(
            err.message().to_lowercase().contains("system"),
            "error should mention system tables"
        );
    }

    #[test]
    fn block_alter_pg_table() {
        let config = GuardrailConfig {
            block_ddl: false,
            ..default_config()
        };
        let result = evaluate("ALTER TABLE pg_class ADD COLUMN x INT", &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn block_drop_pg_table() {
        let config = GuardrailConfig {
            block_ddl: false,
            ..default_config()
        };
        let result = evaluate("DROP TABLE pg_toast_1234", &config);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "guardrail_violation");
    }

    #[test]
    fn allow_normal_table_ddl_when_unblocked() {
        // Normal table + block_ddl=false → allowed.
        let config = GuardrailConfig {
            block_ddl: false,
            ..default_config()
        };
        let result = evaluate("CREATE TABLE public_users (id INT)", &config);
        assert!(
            result.is_ok(),
            "Normal table DDL with block_ddl=false should pass"
        );
    }

    // ── Allow paths for SELECT and INSERT ─────────────────────────────────

    #[test]
    fn allow_select() {
        let result = evaluate(
            "SELECT * FROM orders WHERE status = 'open'",
            &default_config(),
        );
        assert!(result.is_ok(), "SELECT should always pass guardrails");
    }

    #[test]
    fn allow_select_no_where() {
        // SELECT without WHERE is perfectly fine (unguarded-mutation rule
        // only applies to DELETE/UPDATE).
        let result = evaluate("SELECT * FROM orders", &default_config());
        assert!(result.is_ok());
    }

    #[test]
    fn allow_insert() {
        let result = evaluate(
            "INSERT INTO orders (amount) VALUES (100)",
            &default_config(),
        );
        assert!(result.is_ok(), "INSERT should pass guardrails");
    }

    // ── Guardrail violation message quality ───────────────────────────────

    #[test]
    fn ddl_violation_message_mentions_propose_migration() {
        let result = evaluate("DROP TABLE orders", &default_config());
        let err = result.unwrap_err();
        assert!(
            err.message().contains("propose_migration") || err.hint().contains("propose_migration"),
            "DDL violation should reference propose_migration tool"
        );
    }

    #[test]
    fn copy_program_violation_message_mentions_shell_commands() {
        let result = evaluate(
            "COPY orders FROM PROGRAM 'cat /etc/hosts'",
            &default_config(),
        );
        let err = result.unwrap_err();
        assert!(
            err.message().to_lowercase().contains("shell")
                || err.message().to_lowercase().contains("command"),
            "COPY PROGRAM violation should mention shell commands"
        );
    }

    #[test]
    fn set_violation_message_mentions_session() {
        let result = evaluate("SET work_mem = '64MB'", &default_config());
        let err = result.unwrap_err();
        assert!(
            err.message().to_lowercase().contains("session"),
            "SET violation should mention session state"
        );
    }

    #[test]
    fn unguarded_delete_violation_message_mentions_where() {
        let result = evaluate("DELETE FROM orders", &permissive_config());
        let err = result.unwrap_err();
        assert!(
            err.message().to_lowercase().contains("where"),
            "Unguarded DELETE violation should mention WHERE clause"
        );
    }
}
