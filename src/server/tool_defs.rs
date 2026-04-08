// src/server/tool_defs.rs
//
// Static tool manifest for pgmcp.
//
// Defines all 15 tools as rmcp::model::Tool values. The manifest is built
// once (at startup, on first call to tool_list()) and returned on every
// tools/list request.
//
// Tool names match spec section 4. Parameter schemas are JSON Schema objects.
// Descriptions are written for LLM consumption: unambiguous, specify valid
// values explicitly, describe edge cases and return structure.

use std::sync::Arc;

use rmcp::model::Tool;
use serde_json::{Map, Value, json};

/// Build and return the complete list of pgmcp tool definitions.
///
/// Called on every `tools/list` request. The result is returned directly
/// to rmcp for serialization. Tool order is stable (matches spec).
pub(crate) fn tool_list() -> Vec<Tool> {
    vec![
        // ── Discovery tools ──────────────────────────────────────────────────
        Tool::new(
            "list_databases",
            "Lists all databases on the connected PostgreSQL instance that the current role \
             can see. Returns an array of objects each containing: name (string), owner \
             (string), encoding (e.g. UTF8), size_bytes (integer, may be null if no \
             access), and description (string or null). Does not switch the connection to \
             another database — to query a different database, reconnect pgmcp with a new \
             connection string. Use this as your first call to understand what databases \
             are available.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "server_info",
            "Returns metadata about the PostgreSQL server and the current connection. \
             Response includes: version (full version string, e.g. 'PostgreSQL 16.2'), \
             major_version (integer, e.g. 16), current_role (the connected role name), \
             current_database (string), server_settings (object with keys: \
             statement_timeout, max_connections, work_mem, shared_buffers, \
             default_transaction_isolation), and installed_extensions (array of \
             {name, version} objects). Use this to verify the server version before \
             using version-specific SQL features, and to understand connection limits \
             and memory settings before running expensive queries.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_schemas",
            "Lists all schemas in the current database that the connected role has USAGE \
             privilege on. Returns an array of objects with: name (string), owner (string), \
             and description (string or null from pg_description). Always excludes \
             pg_toast, pg_temp_* (temporary schemas), and pg_catalog unless the role has \
             explicit access. The public schema is included if accessible. Call this \
             before list_tables to discover the correct schema name for your target tables.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_tables",
            "Lists tables, views, and materialized views in a specific schema. Returns an \
             array of objects with: name (string), kind (one of: 'table', 'view', \
             'materialized_view'), row_estimate (integer estimated from pg_class.reltuples, \
             may be -1 for views), owner (string), and description (string or null). \
             The `kind` filter defaults to 'table'. Use 'all' to include views and \
             materialized views alongside tables. Row estimates are not exact — use \
             table_stats for current statistics. Use this before describe_table to \
             confirm the table exists in the schema.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Name of the schema to list tables from (e.g. 'public', 'analytics'). \
                                        Must exist and be accessible to the connected role. \
                                        Call list_schemas first if you are unsure of the schema name."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Filter results by object kind. Valid values: \
                                        'table' (default) — regular heap tables only; \
                                        'view' — views only (no data stored); \
                                        'materialized_view' — materialized views (data cached); \
                                        'all' — all three kinds combined. \
                                        If omitted, defaults to 'table'.",
                        "enum": ["table", "view", "materialized_view", "all"],
                        "default": "table"
                    }
                },
                "required": ["schema"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "describe_table",
            "Returns the full definition of a table, view, or materialized view. This is \
             the primary tool for understanding a table's structure before writing queries. \
             Response includes: columns (array of {name, ordinal_position, data_type, \
             pg_type, is_nullable, column_default, description, inferred_description}), \
             primary_key (array of column names), unique_constraints (array of arrays), \
             foreign_keys (array of {columns, references_table, references_columns}), \
             indexes (array of {name, columns, is_unique, index_type, definition}), \
             and check_constraints (array of {name, expression}). \
             The inferred_description field contains a heuristic description of what \
             the column likely stores, derived from its name and type. \
             Returns table_not_found error if the table does not exist.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema containing the table (e.g. 'public'). \
                                        Must match exactly — schema names are case-sensitive."
                    },
                    "table": {
                        "type": "string",
                        "description": "Name of the table, view, or materialized view to describe. \
                                        Must match exactly — table names are case-sensitive. \
                                        Do not include the schema prefix here; use the schema parameter instead."
                    }
                },
                "required": ["schema", "table"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_enums",
            "Lists all user-defined enum types in a schema, with their ordered label values. \
             Returns an array of objects with: schema (string), name (string), and \
             values (ordered array of strings representing enum labels). \
             Enum values are ordered as defined in PostgreSQL (the canonical order used \
             in comparisons). Use this before writing INSERT or WHERE clauses that \
             reference enum columns to ensure you use exact, valid label strings. \
             Built-in enum-like types (e.g. 'char', boolean) are not returned.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema to list enum types from (e.g. 'public'). \
                                        If omitted, defaults to 'public'.",
                        "default": "public"
                    }
                },
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_extensions",
            "Lists all extensions currently installed in the connected database. Returns \
             an array of objects with: name (string), default_version (string), \
             installed_version (string), and schema (string — the schema the extension \
             objects are installed in, typically 'public'). \
             Use this to discover available capabilities before using extension-specific \
             functions or types (e.g., pgvector's <-> operator, PostGIS functions, \
             pg_trgm similarity operators). If an extension is not listed here, its \
             functions and types are not available.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "table_stats",
            "Returns runtime statistics for a table from pg_stat_user_tables and \
             pg_relation_size. Response includes: row_estimate (integer from pg_class), \
             live_rows (integer), dead_rows (integer — unfree space from dead tuples), \
             seq_scans (integer — total sequential scans since last stats reset), \
             idx_scans (integer — total index scans), cache_hit_ratio (float 0.0–1.0, \
             null if no reads yet), table_size_bytes (integer), indexes_size_bytes \
             (integer), total_size_bytes (integer), last_vacuum (ISO 8601 string or null), \
             last_analyze (ISO 8601 string or null). \
             High dead_rows indicates bloat; run VACUUM. \
             High seq_scans with large row_estimate suggests a missing index. \
             Returns table_not_found if the table does not exist in pg_stat_user_tables \
             (views and foreign tables are not tracked).",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema containing the table (e.g. 'public')."
                    },
                    "table": {
                        "type": "string",
                        "description": "Table name to get statistics for. Must be a regular \
                                        table — views and materialized views are not tracked \
                                        in pg_stat_user_tables."
                    }
                },
                "required": ["schema", "table"],
                "additionalProperties": false
            })),
        ),
        // ── SQL-accepting tools ──────────────────────────────────────────────
        Tool::new(
            "query",
            "Executes a single SQL statement and returns results. This is the primary \
             data-access tool. All SQL is parsed, checked against guardrail rules, and \
             (for SELECT) automatically limited before execution. \
             \
             Guardrail rules (always enforced): \
             (1) DDL statements (CREATE, DROP, ALTER, TRUNCATE) are blocked — use \
             propose_migration instead. \
             (2) DELETE and UPDATE without a WHERE clause are blocked to prevent \
             accidental full-table modifications. \
             (3) COPY TO/FROM PROGRAM is blocked. \
             (4) SET statements that modify session state are blocked. \
             \
             For SELECT statements: a LIMIT clause is automatically injected if the SQL \
             does not already contain one. The injected limit defaults to 100 rows and \
             can be overridden with the `limit` parameter (max 10000). \
             \
             Response fields: columns (array of {name, type}), rows (JSON array or CSV \
             string depending on format), row_count (exact integer), truncated (true if \
             row_count == limit and more rows may exist), format (string), sql_executed \
             (the SQL as actually run, after LIMIT injection), limit_injected (boolean), \
             execution_time_ms (float), plan (null unless explain: true). \
             \
             Use dry_run: true to check whether a statement would pass guardrails without \
             executing it. Use transaction: true to wrap DML in a rolled-back transaction \
             for safe inspection.",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "The SQL statement to execute. Must be a single statement \
                                        (no semicolons separating multiple statements). \
                                        SELECT statements have LIMIT injected automatically. \
                                        DDL and unguarded DELETE/UPDATE are blocked."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Optional natural language description of your goal. \
                                        Used for logging and observability only — does not \
                                        affect query execution or results."
                    },
                    "transaction": {
                        "type": "boolean",
                        "description": "If true, wraps the statement in BEGIN/ROLLBACK so no \
                                        data is committed. Use this to safely inspect the effect \
                                        of INSERT/UPDATE/DELETE statements without modifying data. \
                                        Does not override DDL guardrails. Default: false.",
                        "default": false
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, parses and checks the statement against \
                                        guardrail rules but does not execute it. Returns the \
                                        parsed statement kind, whether guardrails passed, and \
                                        whether a LIMIT would be injected. Use this to validate \
                                        SQL before execution. Default: false.",
                        "default": false
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of rows to return. For SELECT statements \
                                        without an existing LIMIT clause, this value is injected \
                                        as LIMIT N. If the SQL already contains LIMIT, the smaller \
                                        of the two limits applies. Valid range: 1–10000. \
                                        Default: 100.",
                        "default": 100,
                        "minimum": 1,
                        "maximum": 10000
                    },
                    "timeout_seconds": {
                        "type": "number",
                        "description": "Statement-level timeout in seconds, applied via \
                                        SET LOCAL statement_timeout. If the statement runs \
                                        longer than this, PostgreSQL cancels it and returns \
                                        a pg_query_failed error. Minimum effective value: 1. \
                                        Default: 30."
                    },
                    "format": {
                        "type": "string",
                        "description": "Output format for result rows. Valid values: \
                                        'json' (default) — array of JSON objects, one per row, \
                                        keys are column names; \
                                        'json_compact' — same structure as json, minimal whitespace; \
                                        'csv' — RFC 4180 CSV string with header row, returned \
                                        as a single string in the rows field. \
                                        Choose 'csv' for large result sets that will be parsed \
                                        by downstream tools. Choose 'json' for direct access \
                                        to typed values.",
                        "enum": ["json", "json_compact", "csv"],
                        "default": "json"
                    },
                    "explain": {
                        "type": "boolean",
                        "description": "If true, prepends EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) \
                                        to the statement and returns the query plan in the `plan` \
                                        field. When explain: true, the `rows` field will be empty \
                                        — the statement is still executed but its rows are not \
                                        returned. Use the standalone `explain` tool if you want \
                                        plan details without also executing the query. Default: false.",
                        "default": false
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "explain",
            "Runs EXPLAIN on a SQL statement and returns the query plan. Unlike the query \
             tool's explain: true option, this tool is dedicated to plan inspection and \
             includes plain-language diagnostic notes. \
             \
             With analyze: true (default), the statement is actually executed to collect \
             real timing and row count statistics; the results are discarded. With \
             analyze: false, returns an estimated plan without executing. \
             \
             Response includes: plan (the raw PostgreSQL EXPLAIN JSON), \
             and diagnostics (array of human-readable strings identifying: sequential \
             scans on large tables, sort spills to disk, nested loop risks, hash batch \
             overflow, missing index opportunities). \
             \
             SELECT, INSERT, UPDATE, DELETE, and WITH (CTE) statements are all supported. \
             DDL statements (CREATE, DROP, etc.) cannot be EXPLAINed and will be blocked \
             by guardrails. Use this before running expensive queries, or to diagnose \
             slow query performance.",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "SQL statement to explain. Must be a single statement. \
                                        DML statements (INSERT/UPDATE/DELETE) with analyze: true \
                                        will actually modify data — use transaction wrapping in \
                                        the query tool instead if you want a dry-run DML plan."
                    },
                    "analyze": {
                        "type": "boolean",
                        "description": "If true (default), runs EXPLAIN ANALYZE — executes the \
                                        statement and returns actual row counts and timing. \
                                        If false, returns estimated plan only without execution \
                                        (faster but less accurate for row count estimates). \
                                        Default: true.",
                        "default": true
                    },
                    "buffers": {
                        "type": "boolean",
                        "description": "If true (default), includes BUFFERS option to show \
                                        shared/local buffer hits and misses. Only meaningful \
                                        when analyze: true. High block reads with few hits \
                                        indicates the working set does not fit in shared_buffers. \
                                        Default: true.",
                        "default": true
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "suggest_index",
            "Analyzes a SQL statement and the existing index coverage on referenced tables, \
             then proposes CREATE INDEX statements that would improve query performance. \
             \
             The analysis uses heuristic rules: columns in WHERE equality predicates become \
             index candidates; JOIN columns and columns in ORDER BY become composite index \
             candidates; existing indexes are checked to avoid redundant suggestions. \
             \
             Response includes: suggestions (array of objects with fields: table (string), \
             columns (array of strings), index_sql (the CREATE INDEX CONCURRENTLY statement \
             to run), impact (one of: 'high', 'medium', 'low' based on estimated table \
             size), and reason (string explaining the suggestion)). An empty suggestions \
             array means the query is already well-indexed or the heuristics found no \
             improvement opportunities. \
             \
             Does not execute any SQL. Review suggestions with explain before applying. \
             Apply with the query tool (requires block_ddl: false in guardrails config).",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "The SQL statement to analyze. SELECT, UPDATE, and \
                                        DELETE statements are supported. The statement is \
                                        parsed but not executed — no data is read or modified."
                    },
                    "schema": {
                        "type": "string",
                        "description": "Default schema for resolving unqualified table names \
                                        in the SQL. For example, if the SQL references 'orders' \
                                        without a schema prefix, this parameter determines which \
                                        schema to look in. Defaults to 'public'.",
                        "default": "public"
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "propose_migration",
            "Given a natural language description of a desired schema change, proposes a \
             database migration as a set of SQL DDL statements with safety analysis. \
             \
             The proposal includes: statements (array of SQL strings to execute in order), \
             reverse_sql (array of SQL strings to undo the migration), warnings (array of \
             strings describing risks: lock acquisition, downtime, data loss), and \
             suggestions (array of strings with safety recommendations). \
             \
             Safety analysis covers: lock types (ACCESS EXCLUSIVE for most DDL blocks all \
             reads and writes), adding NOT NULL constraints on large tables, DROP TABLE \
             (irreversible data loss), and whether CONCURRENTLY variants are available. \
             \
             This tool does NOT execute any SQL. It generates statements for review. \
             Apply migrations with the query tool (requires block_ddl: false in config). \
             \
             Intent examples: 'add a created_at timestamp column to orders, default now()', \
             'add an index on users.email', 'rename column users.name to users.full_name', \
             'add a foreign key from orders.user_id to users.id'.",
            schema(json!({
                "type": "object",
                "properties": {
                    "intent": {
                        "type": "string",
                        "description": "Natural language description of the desired schema change. \
                                        Be specific: name the table, column, type, constraints, \
                                        and any special requirements (e.g., 'nullable', \
                                        'default 0', 'unique'). The more specific the intent, \
                                        the more accurate the generated SQL."
                    },
                    "context_tables": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of table names (schema-qualified or unqualified) \
                                        that the migration touches or should be aware of. \
                                        Include referenced tables for foreign key proposals. \
                                        Example: ['public.orders', 'public.users']. \
                                        If omitted, the migration is generated without schema context."
                    },
                    "schema": {
                        "type": "string",
                        "description": "Default schema for resolving unqualified table names \
                                        in context_tables. Defaults to 'public'.",
                        "default": "public"
                    }
                },
                "required": ["intent"],
                "additionalProperties": false
            })),
        ),
        // ── Introspection tools ──────────────────────────────────────────────
        Tool::new(
            "my_permissions",
            "Reports the privileges of the currently connected PostgreSQL role. This is the \
             authoritative source for understanding what operations are safe to attempt \
             before executing them. \
             \
             Response always includes: role_name (string), is_superuser (boolean), \
             can_create_db (boolean), can_create_role (boolean), \
             schema_privileges (object mapping schema names to {has_usage, has_create}). \
             \
             When `table` is specified, also includes: table_privileges (object with \
             boolean fields: select, insert, update, delete, truncate, references, trigger). \
             \
             Use this before attempting writes to confirm INSERT/UPDATE/DELETE permission. \
             Use this before propose_migration to confirm schema write access. \
             A pg_query_failed result on any tool call may indicate a permission error — \
             check my_permissions to diagnose.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema to check privileges for. The response will \
                                        include schema-level USAGE and CREATE privileges \
                                        for this schema. Defaults to 'public'.",
                        "default": "public"
                    },
                    "table": {
                        "type": "string",
                        "description": "If provided, the response will include table-level \
                                        privileges (SELECT, INSERT, UPDATE, DELETE, TRUNCATE, \
                                        REFERENCES, TRIGGER) for this specific table in the \
                                        given schema. Must be a table name without schema prefix. \
                                        If omitted, table-level privileges are not returned."
                    }
                },
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "connection_info",
            "Returns metadata about the current pgmcp connection to PostgreSQL and the \
             internal connection pool state. Does not execute any SQL beyond what is \
             needed to read server parameters. \
             \
             Response includes: host (string), port (integer), database (string), \
             role (string — the connected role), ssl (boolean), \
             server_version (string — full version string), \
             pool (object with: total_size, idle_count, available_count). \
             \
             Use this to confirm which database and role pgmcp is connected to, and to \
             check pool health (low available_count may indicate pool contention).",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "health",
            "Liveness and readiness check. Acquires a connection from the pool, executes \
             SELECT 1, and measures round-trip latency. \
             \
             Response includes: status (one of: 'ok', 'degraded', 'unhealthy'), \
             latency_ms (float — round-trip time for SELECT 1 in milliseconds), \
             pool (object with total_size, idle_count, available_count), \
             schema_cache_age_seconds (integer — seconds since the schema cache was last \
             refreshed), and error (string or null — set when status is not 'ok'). \
             \
             status 'ok': pool acquired, query succeeded. \
             status 'degraded': pool acquired but latency > 1000ms, or cache is stale. \
             status 'unhealthy': could not acquire pool connection or query failed. \
             \
             Use this as a startup check before issuing queries, or as a heartbeat in \
             orchestration environments.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
    ]
}

/// Convert a `serde_json::Value` (must be an Object) into `Arc<JsonObject>`.
///
/// # Panics
///
/// Panics at startup if the provided JSON literal is not an object — this
/// indicates a programmer error in this file, caught in tests.
fn schema(value: Value) -> Arc<Map<String, Value>> {
    match value {
        Value::Object(map) => Arc::new(map),
        other => panic!("tool_defs: schema must be a JSON object, got: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_contains_exactly_15_tools() {
        let tools = tool_list();
        assert_eq!(
            tools.len(),
            15,
            "expected exactly 15 tools, got {}: {:?}",
            tools.len(),
            tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn all_tool_names_are_unique() {
        let tools = tool_list();
        let mut names = std::collections::HashSet::new();
        for tool in &tools {
            assert!(
                names.insert(tool.name.as_ref()),
                "duplicate tool name: '{}'",
                tool.name
            );
        }
    }

    #[test]
    fn expected_tool_names_present() {
        let tools = tool_list();
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_ref()).collect();

        let expected = [
            "list_databases",
            "server_info",
            "list_schemas",
            "list_tables",
            "describe_table",
            "list_enums",
            "list_extensions",
            "table_stats",
            "query",
            "explain",
            "suggest_index",
            "propose_migration",
            "my_permissions",
            "connection_info",
            "health",
        ];

        for name in &expected {
            assert!(names.contains(*name), "missing tool: '{name}'");
        }
    }

    #[test]
    fn all_tool_descriptions_are_non_empty() {
        let tools = tool_list();
        for tool in &tools {
            assert!(
                tool.description.as_deref().is_some_and(|d| !d.is_empty()),
                "tool '{}' has empty description",
                tool.name
            );
        }
    }

    /// Every description must be at least 100 characters — short descriptions
    /// are not useful for LLM consumption.
    #[test]
    fn all_tool_descriptions_are_sufficiently_detailed() {
        let tools = tool_list();
        for tool in &tools {
            let desc = tool.description.as_deref().unwrap_or("");
            assert!(
                desc.len() >= 100,
                "tool '{}' description is too short ({} chars, minimum 100): '{}'",
                tool.name,
                desc.len(),
                &desc[..desc.len().min(80)]
            );
        }
    }

    /// Every description must specify what the response contains.
    #[test]
    fn all_tool_descriptions_mention_response_structure() {
        let tools = tool_list();
        // Tools whose descriptions must mention what they return.
        let return_keywords: &[(&str, &[&str])] = &[
            ("list_databases", &["array", "name"]),
            ("server_info", &["version", "role"]),
            ("list_schemas", &["array", "schema"]),
            ("list_tables", &["array", "kind"]),
            ("describe_table", &["column", "constraint"]),
            ("list_enums", &["array", "values"]),
            ("list_extensions", &["array", "name"]),
            ("table_stats", &["bytes", "scan"]),
            ("query", &["row_count", "columns"]),
            ("explain", &["plan", "diagnostics"]),
            ("suggest_index", &["suggestion", "index"]),
            ("propose_migration", &["statement", "warning"]),
            ("my_permissions", &["privilege", "role"]),
            ("connection_info", &["host", "pool"]),
            ("health", &["status", "latency"]),
        ];

        let tool_map: std::collections::HashMap<&str, &str> = tools
            .iter()
            .map(|t| (t.name.as_ref(), t.description.as_deref().unwrap_or("")))
            .collect();

        for (tool_name, keywords) in return_keywords {
            let desc = tool_map
                .get(*tool_name)
                .unwrap_or_else(|| panic!("tool not found: {tool_name}"));
            let desc_lower = desc.to_lowercase();
            for kw in *keywords {
                assert!(
                    desc_lower.contains(kw),
                    "tool '{tool_name}' description should mention '{kw}'"
                );
            }
        }
    }

    #[test]
    fn all_input_schemas_are_valid_objects() {
        let tools = tool_list();
        for tool in &tools {
            // schema() panics if not an object — so if we got this far, schemas are valid
            let schema_value = tool.schema_as_json_value();
            assert!(
                schema_value.is_object(),
                "tool '{}' input_schema is not a JSON object",
                tool.name
            );
            let obj = schema_value.as_object().unwrap();
            assert!(
                obj.contains_key("type"),
                "tool '{}' input_schema missing 'type' field",
                tool.name
            );
        }
    }

    #[test]
    fn tool_schemas_with_required_params_declare_required_array() {
        let tools = tool_list();
        let tools_with_required_params = [
            "list_tables",       // requires schema
            "describe_table",    // requires schema, table
            "table_stats",       // requires schema, table
            "query",             // requires sql
            "explain",           // requires sql
            "suggest_index",     // requires sql
            "propose_migration", // requires intent
        ];

        for tool_name in &tools_with_required_params {
            let tool = tools
                .iter()
                .find(|t| t.name.as_ref() == *tool_name)
                .unwrap_or_else(|| panic!("tool not found: {tool_name}"));
            let schema = tool.schema_as_json_value();
            let obj = schema.as_object().unwrap();
            let required = obj.get("required").expect("must have 'required' field");
            let arr = required.as_array().expect("'required' must be an array");
            assert!(
                !arr.is_empty(),
                "tool '{}' has required parameters but 'required' array is empty",
                tool_name
            );
        }
    }

    /// Parameter descriptions for all tools with parameters must be non-empty.
    #[test]
    fn all_parameter_descriptions_are_non_empty() {
        let tools = tool_list();
        for tool in &tools {
            let schema_value = tool.schema_as_json_value();
            let obj = schema_value.as_object().unwrap();
            if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
                for (param_name, param_schema) in props {
                    if let Some(desc) = param_schema.get("description").and_then(|v| v.as_str()) {
                        assert!(
                            !desc.is_empty(),
                            "tool '{}' parameter '{}' has empty description",
                            tool.name,
                            param_name
                        );
                    }
                }
            }
        }
    }

    /// The query tool schema must list all valid format values explicitly.
    #[test]
    fn query_tool_format_enum_has_all_values() {
        let tools = tool_list();
        let query_tool = tools
            .iter()
            .find(|t| t.name.as_ref() == "query")
            .expect("query tool must exist");
        let schema = query_tool.schema_as_json_value();
        let format_enum = schema["properties"]["format"]["enum"]
            .as_array()
            .expect("format must have enum");
        let values: Vec<&str> = format_enum.iter().filter_map(|v| v.as_str()).collect();
        assert!(values.contains(&"json"), "format enum must include 'json'");
        assert!(
            values.contains(&"json_compact"),
            "format enum must include 'json_compact'"
        );
        assert!(values.contains(&"csv"), "format enum must include 'csv'");
    }

    /// The list_tables tool must have 'all' as a valid kind value.
    #[test]
    fn list_tables_kind_enum_includes_all() {
        let tools = tool_list();
        let tool = tools
            .iter()
            .find(|t| t.name.as_ref() == "list_tables")
            .expect("list_tables must exist");
        let schema = tool.schema_as_json_value();
        let kind_enum = schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind must have enum");
        let values: Vec<&str> = kind_enum.iter().filter_map(|v| v.as_str()).collect();
        assert!(values.contains(&"all"), "kind enum must include 'all'");
        assert!(values.contains(&"table"), "kind enum must include 'table'");
        assert!(values.contains(&"view"), "kind enum must include 'view'");
        assert!(
            values.contains(&"materialized_view"),
            "kind enum must include 'materialized_view'"
        );
    }
}
