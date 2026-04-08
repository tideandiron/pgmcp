// src/tools/list_schemas.rs
//
// list_schemas tool — returns visible schemas in the current database.
//
// Returns a JSON object:
//   {
//     "schemas": [
//       {
//         "name":        string,
//         "owner":       string,
//         "description": string | null
//       },
//       ...
//     ]
//   }
//
// Excludes pg_catalog, information_schema, pg_toast*, and pg_temp_* schemas.
// Only schemas the connected role has USAGE privilege on are returned.
// SQL matches src/pg/queries/list_schemas.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `list_schemas` tool call.
///
/// Acquires a connection and queries `pg_namespace` joined with `pg_roles`
/// and `pg_description`. Returns all schemas visible to the connected role,
/// excluding internal Postgres schemas (`pg_catalog`, `information_schema`,
/// `pg_toast*`, `pg_temp_*`).
///
/// Only schemas for which the connected role has `USAGE` privilege are
/// included — this mirrors the visibility rules of `information_schema.schemata`.
///
/// # Errors
///
/// Returns [`McpError::pg_pool_timeout`] if a connection cannot be acquired
/// within the configured timeout, or [`McpError::pg_query_failed`] if the
/// catalog query fails.
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/list_schemas.sql
    let rows = client
        .query(
            "SELECT n.nspname, r.rolname, d.description \
            FROM pg_namespace n \
            JOIN pg_roles r ON r.oid = n.nspowner \
            LEFT JOIN pg_description d \
                ON d.objoid = n.oid AND d.classoid = 'pg_namespace'::regclass \
            WHERE \
                n.nspname NOT IN ('pg_catalog', 'information_schema') \
                AND n.nspname NOT LIKE 'pg_toast%' \
                AND n.nspname NOT LIKE 'pg_temp_%' \
                AND has_schema_privilege(n.nspname, 'USAGE') \
            ORDER BY n.nspname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection before constructing the response.
    drop(client);

    let schemas: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let owner: String = row.get(1);
            let description: Option<String> = row.get(2);
            serde_json::json!({
                "name":        name,
                "owner":       owner,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "schemas": schemas });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
