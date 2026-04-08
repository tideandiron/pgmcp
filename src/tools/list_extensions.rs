// src/tools/list_extensions.rs
//
// list_extensions tool — returns all extensions installed in the current database.
//
// Parameters: none.
//
// Queries pg_extension joined with pg_namespace for the schema, and LEFT JOINs
// pg_available_extensions for the human-readable description. The LEFT JOIN is
// intentional: locally-built extensions may not appear in pg_available_extensions.
//
// Returns a JSON object:
//   {
//     "extensions": [
//       {
//         "name":        string,   -- e.g. "plpgsql"
//         "version":     string,   -- e.g. "1.0"
//         "schema":      string,   -- e.g. "pg_catalog"
//         "description": string    -- from pg_available_extensions, or ""
//       },
//       ...
//     ]
//   }
//
// Results are ordered alphabetically by name.
// SQL matches src/pg/queries/list_extensions.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `list_extensions` tool call.
///
/// Acquires a connection, queries `pg_extension` joined with `pg_namespace` and
/// `pg_available_extensions`, and returns all installed extensions with their
/// version, schema, and description.
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

    // SQL matches src/pg/queries/list_extensions.sql.
    let rows = client
        .query(
            "SELECT e.extname, e.extversion, n.nspname, \
                    COALESCE(a.comment, '') AS description \
             FROM pg_extension e \
             JOIN pg_namespace n ON e.extnamespace = n.oid \
             LEFT JOIN pg_available_extensions a ON a.name = e.extname \
             ORDER BY e.extname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection — query is done.
    drop(client);

    let extensions: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let version: String = row.get(1);
            let schema: String = row.get(2);
            let description: String = row.get(3);
            serde_json::json!({
                "name":        name,
                "version":     version,
                "schema":      schema,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "extensions": extensions });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
