// src/tools/list_schemas.rs
//
// list_schemas tool — returns visible schemas in the current database.
//
// Cache-first: reads from SchemaCache when populated; falls back to a live
// pg_catalog query when the cache is empty (pre-warm race or empty database).
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
/// Checks the schema cache first. On a cache hit (non-empty result), returns
/// cached data without acquiring a connection. On a cache miss, acquires a
/// connection and queries `pg_namespace` directly.
///
/// # Errors
///
/// Returns [`McpError::pg_pool_timeout`] if a connection cannot be acquired
/// within the configured timeout (fallback path only), or
/// [`McpError::pg_query_failed`] if the catalog query fails.
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    // Cache-first: read schemas from snapshot.
    let cached = ctx.cache.get_schemas().await;
    let schemas: Vec<Value> = if !cached.is_empty() {
        tracing::debug!(count = cached.len(), "list_schemas: cache hit");
        cached
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "name":        s.name,
                    "owner":       s.owner,
                    "description": s.description,
                })
            })
            .collect()
    } else {
        // Cache miss (startup race or empty database): fall through to live query.
        tracing::debug!("list_schemas: cache miss, querying pg_catalog");
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

        drop(client);

        rows.iter()
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
            .collect()
    };

    let body = serde_json::json!({ "schemas": schemas });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
