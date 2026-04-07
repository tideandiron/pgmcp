// src/server/tool_defs.rs
//
// Static tool manifest for pgmcp.
//
// Defines all 15 tools as rmcp::model::Tool values. The manifest is built
// once (at startup, on first call to tool_list()) and returned on every
// tools/list request.
//
// Tool names match spec section 4. Parameter schemas are JSON Schema objects.
// Descriptions are written for LLM consumption per design principle 3.

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
            "Returns all databases visible to the connected role on this Postgres instance. \
             Use this to discover available databases before switching context.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "server_info",
            "Returns Postgres server version, key server settings (statement_timeout, \
             max_connections, work_mem, shared_buffers), and the connected role. \
             Use this to understand the capabilities and constraints of the server.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_schemas",
            "Returns all schemas in the current database that are visible to the connected role. \
             Excludes internal schemas (pg_toast, pg_temp_*). \
             Use this to discover the namespace structure before listing tables.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_tables",
            "Returns tables, views, and materialized views in a schema. \
             Filter by kind to narrow results. Includes row estimates from pg_class.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema name to list tables from. Use list_schemas to discover available schemas."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Filter by object kind. One of: 'table', 'view', 'materialized_view', 'all'. Defaults to 'table'.",
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
            "Returns the full definition of a table: columns with types and constraints, \
             primary key, unique constraints, foreign keys, indexes, and check constraints. \
             This is the primary tool for understanding table structure before writing queries.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema containing the table (e.g. 'public')."
                    },
                    "table": {
                        "type": "string",
                        "description": "Table name to describe."
                    }
                },
                "required": ["schema", "table"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_enums",
            "Returns all enum types in a schema with their ordered label values. \
             Use this to understand valid enum values before constructing INSERT or WHERE clauses.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema to list enum types from. Defaults to 'public'.",
                        "default": "public"
                    }
                },
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "list_extensions",
            "Returns all extensions installed in the current database. \
             Use this to discover available capabilities (e.g., pgvector, PostGIS, pg_trgm).",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "table_stats",
            "Returns runtime statistics for a table: row estimate, live/dead tuple counts, \
             sequential and index scan counts, last vacuum/analyze timestamps, \
             and size breakdown (table, toast, indexes). \
             Use this to diagnose performance issues and understand table health.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema containing the table."
                    },
                    "table": {
                        "type": "string",
                        "description": "Table name to get statistics for."
                    }
                },
                "required": ["schema", "table"],
                "additionalProperties": false
            })),
        ),
        // ── SQL-accepting tools ──────────────────────────────────────────────
        Tool::new(
            "query",
            "Executes a SQL query and returns results. The primary tool for data access. \
             Supports SELECT and (with transaction: true) DML statements for dry-run inspection. \
             DDL statements (CREATE, DROP, ALTER, TRUNCATE) are blocked. \
             A LIMIT is automatically injected if not present in the SQL.",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "SQL statement to execute. Must be a single statement."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Optional natural language description of what you are trying to accomplish. Used for logging and observability."
                    },
                    "transaction": {
                        "type": "boolean",
                        "description": "If true, wrap the statement in an explicit transaction that is rolled back after execution. Useful for dry-run DML inspection. Does not affect DDL guardrails.",
                        "default": false
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, parse and analyze the statement but do not execute it. Returns the parsed statement kind and guardrail analysis.",
                        "default": false
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of rows to return. Injected as a LIMIT clause if not already present in the SQL.",
                        "default": 1000,
                        "minimum": 1,
                        "maximum": 50000
                    },
                    "timeout_seconds": {
                        "type": "number",
                        "description": "Statement timeout in seconds. Applied via SET LOCAL statement_timeout. Defaults to the server-configured value."
                    },
                    "format": {
                        "type": "string",
                        "description": "Output format for result rows.",
                        "enum": ["json", "csv"],
                        "default": "json"
                    },
                    "explain": {
                        "type": "boolean",
                        "description": "If true, prepend EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) and return the query plan alongside results.",
                        "default": false
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "explain",
            "Runs EXPLAIN on a SQL statement and returns the query plan with execution statistics. \
             Use analyze: false for plan estimation without execution. \
             Does not return result rows — use query with explain: true for both plan and data.",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "SQL statement to explain. Must be a single statement."
                    },
                    "analyze": {
                        "type": "boolean",
                        "description": "If true (default), run EXPLAIN ANALYZE — executes the statement and collects real runtime statistics. If false, produces estimated plan only without execution.",
                        "default": true
                    },
                    "buffers": {
                        "type": "boolean",
                        "description": "Include buffer usage statistics in the plan. Requires analyze: true.",
                        "default": true
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "suggest_index",
            "Analyzes a SQL statement and the current index state of referenced tables, \
             then proposes indexes that would improve query performance. \
             Uses heuristic rules based on WHERE, JOIN, ORDER BY, and GROUP BY clauses.",
            schema(json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "The SQL statement to analyze for index opportunities."
                    },
                    "schema": {
                        "type": "string",
                        "description": "Default schema for resolving unqualified table references.",
                        "default": "public"
                    }
                },
                "required": ["sql"],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "propose_migration",
            "Given a description of intent and a set of context tables, proposes a database \
             migration as a set of SQL statements with explanations. \
             Uses heuristic patterns. Does NOT execute any SQL — review before applying.",
            schema(json!({
                "type": "object",
                "properties": {
                    "intent": {
                        "type": "string",
                        "description": "Natural language description of what the migration should accomplish. Be specific about the desired schema change."
                    },
                    "context_tables": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Table names (schema-qualified or unqualified) to include as context for the migration."
                    },
                    "schema": {
                        "type": "string",
                        "description": "Default schema for resolving unqualified table names.",
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
            "Reports the privileges of the connected Postgres role: superuser status, \
             schema-level privileges (USAGE, CREATE), and optionally table-level privileges \
             (SELECT, INSERT, UPDATE, DELETE) for a specific table. \
             Use this to understand what operations are safe to attempt.",
            schema(json!({
                "type": "object",
                "properties": {
                    "schema": {
                        "type": "string",
                        "description": "Schema to introspect privileges for.",
                        "default": "public"
                    },
                    "table": {
                        "type": "string",
                        "description": "If specified, include table-level privilege detail for this table."
                    }
                },
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "connection_info",
            "Returns information about the current pgmcp connection to Postgres: \
             host, port, database, connected role, SSL status, server version, \
             and pool statistics (total, idle, and in-use connections). \
             Use this to understand the current connection context.",
            schema(json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            })),
        ),
        Tool::new(
            "health",
            "Liveness and readiness check. Verifies that pgmcp can acquire a pool \
             connection and execute a trivial query (SELECT 1). \
             Returns status 'ok', 'degraded', or 'unhealthy'. \
             Use this to confirm the server is functioning before running queries.",
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
}
