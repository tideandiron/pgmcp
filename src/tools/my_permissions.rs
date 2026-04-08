// src/tools/my_permissions.rs
//
// my_permissions tool — introspects actual PostgreSQL role permissions.
//
// Parameters:
//   schema (string, optional, default "public") — schema to check privileges for
//   table  (string, optional)                   — if set, check table-level privileges
//
// Queries executed:
//   1. pg_roles for role attributes (superuser, createdb, createrole, etc.)
//   2. information_schema.schemata + has_schema_privilege() for schema privileges
//   3. (optional) has_table_privilege() for per-table privileges
//
// The connected role is determined from current_user — agents cannot introspect
// other roles through this tool.
//
// Design invariants:
// - Does not accept a role parameter. Always introspects the session role.
// - All three queries run on a single connection (acquired once, released after).
// - Schema privilege query excludes system schemas automatically.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

// ── Parameters ────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct PermissionsParams {
    schema: String,
    table: Option<String>,
}

impl PermissionsParams {
    fn from_args(args: Option<&Map<String, Value>>) -> Self {
        let schema = args
            .and_then(|m| m.get("schema"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("public")
            .to_string();

        let table = args
            .and_then(|m| m.get("table"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        PermissionsParams { schema, table }
    }
}

// ── handle ────────────────────────────────────────────────────────────────────

/// Handle a `my_permissions` tool call.
///
/// Returns the role attributes, schema-level privileges, and (optionally)
/// table-level privileges for the currently connected Postgres role.
///
/// # Errors
///
/// - [`McpError::pg_pool_timeout`] — connection acquisition timeout
/// - [`McpError::pg_query_failed`] — catalog query error
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let params = PermissionsParams::from_args(args.as_ref());

    tracing::debug!(
        schema = %params.schema,
        table = ?params.table,
        "my_permissions tool invoked"
    );

    let acquire_timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(acquire_timeout).await?;

    // ── Query 1: Role attributes ─────────────────────────────────────────────
    let role_rows = client
        .query(
            "SELECT \
                current_user AS role_name, \
                rolsuper, \
                rolcreatedb, \
                rolcreaterole, \
                rolinherit, \
                rolcanlogin, \
                rolreplication, \
                rolbypassrls, \
                rolconnlimit \
             FROM pg_roles \
             WHERE rolname = current_user",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let role_json = build_role_json(&role_rows)?;

    // ── Query 2: Schema privileges ───────────────────────────────────────────
    // Query all non-system schemas and check USAGE and CREATE privileges.
    let schema_rows = client
        .query(
            "SELECT \
                schema_name, \
                has_schema_privilege(schema_name, 'USAGE') AS can_usage, \
                has_schema_privilege(schema_name, 'CREATE') AS can_create \
             FROM information_schema.schemata \
             WHERE schema_name NOT IN ('pg_toast', 'pg_catalog', 'information_schema') \
               AND schema_name NOT LIKE 'pg_temp_%' \
               AND schema_name NOT LIKE 'pg_toast_temp_%' \
             ORDER BY schema_name",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let schema_privs: Vec<Value> = schema_rows
        .iter()
        .map(|row| {
            let schema_name: String = row.get(0);
            let usage: Option<bool> = row.get(1);
            let create: Option<bool> = row.get(2);
            serde_json::json!({
                "schema": schema_name,
                "usage":  usage.unwrap_or(false),
                "create": create.unwrap_or(false),
            })
        })
        .collect();

    // ── Query 3 (optional): Table privileges ─────────────────────────────────
    let table_privs: Option<Value> = if let Some(ref table) = params.table {
        // Build the fully-qualified table reference for has_table_privilege().
        let qualified = format!("{}.{}", params.schema, table);

        let tbl_rows = client
            .query(
                "SELECT \
                    has_table_privilege($1, 'SELECT')     AS can_select, \
                    has_table_privilege($1, 'INSERT')     AS can_insert, \
                    has_table_privilege($1, 'UPDATE')     AS can_update, \
                    has_table_privilege($1, 'DELETE')     AS can_delete, \
                    has_table_privilege($1, 'TRUNCATE')   AS can_truncate, \
                    has_table_privilege($1, 'REFERENCES') AS can_references",
                &[&qualified],
            )
            .await
            .map_err(McpError::from)?;

        if let Some(row) = tbl_rows.first() {
            let can_select: Option<bool> = row.get(0);
            let can_insert: Option<bool> = row.get(1);
            let can_update: Option<bool> = row.get(2);
            let can_delete: Option<bool> = row.get(3);
            let can_truncate: Option<bool> = row.get(4);
            let can_references: Option<bool> = row.get(5);

            Some(serde_json::json!({
                "table":      qualified,
                "select":     can_select.unwrap_or(false),
                "insert":     can_insert.unwrap_or(false),
                "update":     can_update.unwrap_or(false),
                "delete":     can_delete.unwrap_or(false),
                "truncate":   can_truncate.unwrap_or(false),
                "references": can_references.unwrap_or(false),
            }))
        } else {
            None
        }
    } else {
        None
    };

    drop(client);

    let mut body = serde_json::json!({
        "role": role_json,
        "schema_privileges": schema_privs,
    });

    if let Some(tp) = table_privs {
        body["table_privileges"] = tp;
    }

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

// ── build_role_json ───────────────────────────────────────────────────────────

fn build_role_json(rows: &[tokio_postgres::Row]) -> Result<Value, McpError> {
    let row = rows.first().ok_or_else(|| {
        McpError::internal("pg_roles returned no row for current_user — unexpected")
    })?;

    let role_name: String = row.get(0);
    let is_superuser: bool = row.get(1);
    let can_create_db: bool = row.get(2);
    let can_create_role: bool = row.get(3);
    let inherits: bool = row.get(4);
    let can_login: bool = row.get(5);
    let is_replication: bool = row.get(6);
    let bypass_rls: bool = row.get(7);
    let connection_limit: i32 = row.get(8);

    Ok(serde_json::json!({
        "name":             role_name,
        "is_superuser":     is_superuser,
        "can_create_db":    can_create_db,
        "can_create_role":  can_create_role,
        "inherits":         inherits,
        "can_login":        can_login,
        "is_replication":   is_replication,
        "bypass_rls":       bypass_rls,
        "connection_limit": connection_limit,
    }))
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
    fn params_schema_defaults_to_public() {
        let p = PermissionsParams::from_args(None);
        assert_eq!(p.schema, "public");
        assert!(p.table.is_none());
    }

    #[test]
    fn params_schema_explicit() {
        let p = PermissionsParams::from_args(make_args(r#"{"schema": "analytics"}"#).as_ref());
        assert_eq!(p.schema, "analytics");
    }

    #[test]
    fn params_schema_empty_falls_back_to_public() {
        let p = PermissionsParams::from_args(make_args(r#"{"schema": ""}"#).as_ref());
        assert_eq!(p.schema, "public");
    }

    #[test]
    fn params_table_optional_present() {
        let p = PermissionsParams::from_args(
            make_args(r#"{"schema": "public", "table": "orders"}"#).as_ref(),
        );
        assert_eq!(p.table, Some("orders".to_string()));
    }

    #[test]
    fn params_table_optional_absent() {
        let p = PermissionsParams::from_args(make_args(r#"{"schema": "public"}"#).as_ref());
        assert!(p.table.is_none());
    }

    #[test]
    fn params_table_empty_string_treated_as_none() {
        let p = PermissionsParams::from_args(
            make_args(r#"{"schema": "public", "table": ""}"#).as_ref(),
        );
        assert!(
            p.table.is_none(),
            "empty string table should be treated as None"
        );
    }

    #[test]
    fn params_no_args_gives_defaults() {
        let p = PermissionsParams::from_args(make_args("{}").as_ref());
        assert_eq!(p.schema, "public");
        assert!(p.table.is_none());
    }
}
