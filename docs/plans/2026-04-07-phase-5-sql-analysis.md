# Phase 5: SQL Analysis Layer — Implementation Plan

**Date:** 2026-04-07
**Branches:** feat/014, feat/015, feat/016
**Prerequisite:** Phase 4 (feat/013 schema-cache) merged to main
**Baseline:** 279 passing tests

---

## Overview

Phase 5 implements the SQL Analysis Layer: the pure-Rust transformation and validation
pass that sits between tool handlers and the connection pool. No SQL executes in this
layer — it is strictly parse → classify → validate → transform.

The three branches are ordered by dependency: parser first, guardrails second (depends
on parser types), limit injection third (depends on parser and guardrail types).

```
feat/014  ──►  feat/015  ──►  feat/016
SQL Parser   Guardrails    LIMIT Injection
```

---

## Branch feat/014 — SQL Parser (`src/sql/parser.rs`)

### Purpose

Wrap `sqlparser` 0.61 with the PostgreSQL dialect to produce a `ParsedStatement`
value that downstream components (guardrails, limit injection) consume. This layer
never executes SQL; it only parses and extracts metadata.

### Key types

```rust
/// Classified kind of a parsed SQL statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementKind {
    Select,
    Insert,
    Update,
    Delete,
    CreateTable,
    AlterTable,
    DropTable,
    CreateIndex,
    DropIndex,
    Truncate,
    Copy,
    Set,
    Other,
}

/// Metadata extracted from a single parsed SQL statement.
pub(crate) struct ParsedStatement {
    pub kind: StatementKind,
    pub table_names: Vec<String>,
    pub has_limit: bool,
    pub has_where: bool,
    pub param_count: usize,          // highest $N placeholder found
    pub is_copy_program: bool,       // COPY TO/FROM PROGRAM
    pub raw_stmt: sqlparser::ast::Statement,
}
```

### Public API

```rust
/// Parse a single SQL statement using the PostgreSQL dialect.
///
/// Returns `Err(McpError::sql_parse_error(...))` if:
/// - The input is empty or whitespace only
/// - The input contains more than one semicolon-separated statement
/// - sqlparser fails to parse the SQL
pub(crate) fn parse_statement(sql: &str) -> Result<ParsedStatement, McpError>
```

### Implementation notes

- Use `sqlparser::dialect::PostgreSqlDialect` throughout.
- Parse with `sqlparser::parser::Parser::parse_sql(dialect, sql)`.
- `parse_sql` returns a `Vec<Statement>`. Length != 1 → multi-statement error.
- `StatementKind` is derived from a `match` on the `Statement` variant:
  - `Statement::Query(_)` → `Select`
  - `Statement::Insert(_)` → `Insert`
  - `Statement::Update(_)` → `Update`
  - `Statement::Delete(_)` → `Delete`
  - `Statement::CreateTable(_)` → `CreateTable`
  - `Statement::AlterTable(_)` → `AlterTable`
  - `Statement::Drop { object_type: ObjectType::Table, .. }` → `DropTable`
  - `Statement::CreateIndex(_)` → `CreateIndex`
  - `Statement::Drop { object_type: ObjectType::Index, .. }` → `DropIndex`
  - `Statement::Truncate(_)` → `Truncate`
  - `Statement::Copy { .. }` → `Copy`
  - `Statement::Set(_)` → `Set`
  - Anything else → `Other`
- `has_limit` is true when the statement is a `Query` and
  `query.limit_clause.is_some()`. Use `fetch` as a secondary check
  (FETCH FIRST N ROWS is semantically equivalent to LIMIT).
- `has_where` is true for `Select` (check `selection` field on the first
  `SelectCore`), `Update` (`selection` field), or `Delete` (`selection` field).
- `table_names` is extracted best-effort:
  - For `Select`: walk the `FROM` clause and collect table names.
  - For `Insert`, `Update`, `Delete`: extract the primary table name.
  - For DDL: extract the object name.
- `param_count` counts `$N` placeholders by walking the SQL string after
  successful parse. Use a simple regex or iterator scan: find all `$\d+` tokens
  and take the maximum N.
- `is_copy_program` is `true` when the statement is `Copy { target: CopyTarget::Program { .. }, .. }`.

### Test plan

Tests live in `src/sql/parser.rs` under `#[cfg(test)]`.

| Test name | Validates |
|---|---|
| `parse_select_simple` | `SELECT 1` → `StatementKind::Select` |
| `parse_select_with_limit` | `SELECT * FROM t LIMIT 10` → `has_limit=true` |
| `parse_select_with_where` | `SELECT * FROM t WHERE id=1` → `has_where=true` |
| `parse_insert` | `INSERT INTO t VALUES (1)` → `Insert` |
| `parse_update_with_where` | `UPDATE t SET x=1 WHERE id=1` → `Update, has_where=true` |
| `parse_update_without_where` | `UPDATE t SET x=1` → `Update, has_where=false` |
| `parse_delete_with_where` | `DELETE FROM t WHERE id=1` → `Delete, has_where=true` |
| `parse_delete_without_where` | `DELETE FROM t` → `Delete, has_where=false` |
| `parse_create_table` | `CREATE TABLE t (id INT)` → `CreateTable` |
| `parse_alter_table` | `ALTER TABLE t ADD COLUMN c TEXT` → `AlterTable` |
| `parse_drop_table` | `DROP TABLE t` → `DropTable` |
| `parse_create_index` | `CREATE INDEX i ON t (c)` → `CreateIndex` |
| `parse_drop_index` | `DROP INDEX i` → `DropIndex` |
| `parse_truncate` | `TRUNCATE TABLE t` → `Truncate` |
| `parse_copy` | `COPY t FROM STDIN` → `Copy` |
| `parse_copy_program` | `COPY t FROM PROGRAM 'cmd'` → `is_copy_program=true` |
| `parse_set` | `SET statement_timeout = '5s'` → `Set` |
| `parse_multi_statement` | `SELECT 1; SELECT 2` → `sql_parse_error` |
| `parse_empty_string` | `""` → `sql_parse_error` |
| `parse_malformed_sql` | `SELECT FROM WHERE` → `sql_parse_error` |
| `param_count_dollar_params` | `SELECT * FROM t WHERE id=$1` → `param_count=1` |
| `param_count_multiple` | `... $1 ... $2 ... $3` → `param_count=3` |
| `parse_case_insensitive` | `select 1` → `Select` |
| `parse_leading_whitespace` | `  SELECT 1  ` → `Select` |
| `parse_with_cte` | `WITH cte AS (SELECT 1) SELECT * FROM cte` → `Select` |

---

## Branch feat/015 — Guardrails (`src/sql/guardrails.rs`)

### Purpose

Evaluate guardrail rules against a `ParsedStatement`. Returns `Ok(())` when
the statement is allowed, or `Err(McpError::guardrail_violation(...))` with a
descriptive reason when it is blocked.

### Key types

```rust
/// Guardrail configuration — mirrors the relevant fields from `Config`.
#[derive(Debug, Clone)]
pub(crate) struct GuardrailConfig {
    pub block_ddl: bool,
    pub block_copy_program: bool,
    pub block_session_set: bool,
}

impl Default for GuardrailConfig { ... }  // all true
```

### Public API

```rust
/// Evaluate all guardrail rules for a parsed statement.
///
/// Rules:
///   1. DDL block (configurable): blocks CREATE, ALTER, DROP, TRUNCATE.
///   2. COPY PROGRAM block (configurable): blocks COPY TO/FROM PROGRAM.
///   3. Session SET block (configurable): blocks SET statements.
///   4. Unguarded DELETE/UPDATE (always): blocks DELETE or UPDATE without WHERE.
///   5. System table DDL (always): blocks DDL targeting pg_* or information_schema.*.
pub(crate) fn check(
    parsed: &ParsedStatement,
    config: &GuardrailConfig,
) -> Result<(), McpError>
```

### Rule descriptions

| # | Rule | Configurable | Triggered by |
|---|---|---|---|
| 1 | Block DDL | `block_ddl` | `CreateTable`, `AlterTable`, `DropTable`, `CreateIndex`, `DropIndex`, `Truncate` |
| 2 | Block COPY PROGRAM | `block_copy_program` | `Copy` + `is_copy_program=true` |
| 3 | Block session SET | `block_session_set` | `Set` |
| 4 | Unguarded DELETE/UPDATE | always | `Delete` or `Update` with `has_where=false` |
| 5 | System table DDL | always | DDL targeting table names starting with `pg_` or in `information_schema` |

### Error messages

Each rule violation must produce a `McpError::guardrail_violation(reason)` where
`reason` is a complete sentence explaining what was blocked and why:

- Rule 1: `"DDL statement (CREATE/ALTER/DROP/TRUNCATE) is blocked by guardrail policy. Use the propose_migration tool to generate schema changes."`
- Rule 2: `"COPY TO/FROM PROGRAM is blocked because it can execute arbitrary shell commands."`
- Rule 3: `"SET statement is blocked because it would modify session state affecting all subsequent callers."`
- Rule 4: `"DELETE/UPDATE without a WHERE clause is blocked to prevent accidental full-table modifications."`
- Rule 5: `"DDL targeting system tables (pg_* or information_schema) is always blocked."`

### Test plan

Tests live in `src/sql/guardrails.rs` under `#[cfg(test)]`. Each test constructs
a `ParsedStatement` by calling `parse_statement()` from feat/014.

| Test | Validates |
|---|---|
| `allow_select` | SELECT passes all rules |
| `allow_insert` | INSERT passes all rules |
| `allow_update_with_where` | UPDATE + WHERE passes |
| `allow_delete_with_where` | DELETE + WHERE passes |
| `block_create_table_ddl` | CREATE TABLE → guardrail_violation |
| `block_alter_table_ddl` | ALTER TABLE → guardrail_violation |
| `block_drop_table_ddl` | DROP TABLE → guardrail_violation |
| `block_truncate_ddl` | TRUNCATE → guardrail_violation |
| `allow_ddl_when_block_ddl_false` | block_ddl=false → CREATE TABLE allowed |
| `block_copy_program` | COPY FROM PROGRAM → guardrail_violation |
| `allow_copy_stdin` | COPY FROM STDIN → allowed when block_copy_program=true |
| `allow_copy_program_when_flag_false` | block_copy_program=false → allowed |
| `block_set_session` | SET statement_timeout → guardrail_violation |
| `allow_set_when_flag_false` | block_session_set=false → allowed |
| `block_delete_without_where` | DELETE without WHERE → guardrail_violation |
| `block_update_without_where` | UPDATE without WHERE → guardrail_violation |
| `block_create_pg_table` | CREATE TABLE pg_foo → system table violation |
| `block_alter_pg_table` | ALTER TABLE pg_catalog.pg_class → system table violation |
| `block_drop_pg_table` | DROP TABLE pg_toast_1234 → system table violation |
| `allow_normal_table_ddl_when_unblocked` | CREATE TABLE public.users (block_ddl=false) → allowed |

---

## Branch feat/016 — LIMIT Injection (`src/sql/limit.rs`)

### Purpose

Inject a `LIMIT` clause into `SELECT` statements that lack one, and enforce a
maximum LIMIT on statements that already have one. Non-SELECT statements are
returned unchanged.

### Public API

```rust
/// Inject or cap the LIMIT clause of a SELECT statement.
///
/// Returns `(modified_sql, was_injected)` where:
/// - `modified_sql` is the SQL to execute (potentially with LIMIT appended/capped).
/// - `was_injected` is true if a new LIMIT was added; false if one already existed
///   (even if it was capped) or if the statement is not a SELECT.
///
/// Behaviour by case:
/// - Non-SELECT → return `(sql.to_string(), false)` unchanged.
/// - SELECT without LIMIT → append `LIMIT {default_limit}`, return `injected=true`.
/// - SELECT with LIMIT N → use `min(N, max_limit)`, return `injected=false`.
/// - SELECT with LIMIT ALL → treat as no limit, inject default.
/// - Subqueries: inject only on outermost query, never on subquery SELECTs.
/// - CTEs: the WITH clause is not modified; only the final SELECT body.
/// - UNION/INTERSECT/EXCEPT: limit applies to the combined result (outermost).
///
/// # Errors
/// Returns `McpError::sql_parse_error` if the SQL cannot be parsed.
pub(crate) fn inject_limit(
    sql: &str,
    default_limit: u32,
    max_limit: u32,
) -> Result<(String, bool), McpError>
```

### Implementation notes

- Call `parse_statement(sql)` to get the AST.
- If the statement is not `StatementKind::Select`, return `(sql.to_string(), false)`.
- For a SELECT, extract the `Box<Query>` from `Statement::Query`.
- Check `query.limit_clause`:
  - `None`: inject. Set `query.limit_clause = Some(LimitClause::LimitOffset { limit: Some(Expr::Value(Value::Number(default_limit.to_string(), false))), offset: None, limit_by: vec![] })`. Return `injected=true`.
  - `Some(LimitClause::LimitOffset { limit: Some(Expr::Value(Value::Number(n, _))), .. })`:
    parse `n` as `u64`. If `n > max_limit`, replace with `max_limit`. Return `injected=false`.
  - `Some(LimitClause::LimitOffset { limit: None, .. })` (LIMIT ALL or bare): treat as no limit, inject `default_limit`, return `injected=true`.
  - `Some(LimitClause::OffsetCommaLimit { limit, .. })`: extract and cap, return `injected=false`.
  - Any other limit expression (computed): leave unchanged, return `injected=false`.
- Reconstruct the SQL via `query.to_string()`. The `Display` impl on `sqlparser`
  AST nodes produces valid PostgreSQL SQL.
- The inner subqueries in `FROM` clauses, `WHERE EXISTS`, CTEs, etc., are not
  touched because we only mutate the top-level `Query` struct.

### Test plan

Tests live in `src/sql/limit.rs` under `#[cfg(test)]`.

| Test | Validates |
|---|---|
| `no_limit_select_injects_default` | `SELECT * FROM t` → injects default, injected=true |
| `existing_limit_below_max_unchanged` | `SELECT * FROM t LIMIT 5` (max=100) → limit=5, injected=false |
| `existing_limit_above_max_is_capped` | `SELECT * FROM t LIMIT 500` (max=100) → limit=100, injected=false |
| `existing_limit_equals_max_unchanged` | `SELECT * FROM t LIMIT 100` (max=100) → limit=100, injected=false |
| `non_select_returned_unchanged` | `INSERT INTO t VALUES (1)` → unchanged, injected=false |
| `delete_returned_unchanged` | `DELETE FROM t WHERE id=1` → unchanged |
| `subquery_limit_not_injected` | `SELECT * FROM (SELECT * FROM t) sub` → only outer gets LIMIT |
| `cte_outer_select_gets_limit` | `WITH cte AS (SELECT 1) SELECT * FROM cte` → outer gets LIMIT |
| `union_gets_limit_at_outer` | `SELECT 1 UNION SELECT 2` → gets LIMIT on combined result |
| `limit_all_treated_as_no_limit` | `SELECT * FROM t LIMIT ALL` → replaced with default |
| `order_by_preserved` | `SELECT * FROM t ORDER BY id` → LIMIT appended after ORDER BY |
| `default_zero_injects_zero` | edge: default_limit=0 still injects |
| `parse_error_propagated` | malformed SQL → sql_parse_error |

---

## Acceptance Checklist (applies to all three branches)

- [ ] `cargo test` passes (all existing 279 tests + new tests)
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] No `unsafe` code outside of zero-cost abstraction boundaries
- [ ] All public-facing items have doc comments
- [ ] No `.unwrap()` or `.expect()` in production paths
- [ ] Error messages are agent-readable (no Rust internal details)

---

## Deviations from Master Plan

1. `ParsedStatement` returns `raw_stmt: sqlparser::ast::Statement` so that both
   guardrails and limit injection can work directly on the AST without re-parsing.
   The master plan spec describes these as independent modules but sharing the AST
   object avoids a second parse.

2. The master plan's feat/016 API signature `inject_limit(sql, limit_value)` is
   extended to `inject_limit(sql, default_limit, max_limit)` to support LIMIT
   capping (max_limit). The spec section 3.2 describes capping semantics, so both
   parameters are required.

3. System-table DDL guardrail (rule 5) is added as an "always on" rule even
   though the master plan table lists only three configurable rules. The design
   spec section 3.2 lists it as an unconditional rule.

4. `GuardrailConfig` is a new type not mentioned by name in the master plan; the
   master plan refers to `Config.guardrail_policy`. We define `GuardrailConfig`
   as a sub-struct that is constructed from `Config` at the call site.
