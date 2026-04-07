// src/tools/server_info.rs
//
// server_info tool — returns Postgres server version, key settings, connected role.
//
// Returns a JSON object:
//   {
//     "version":     string,   -- version() output
//     "version_num": number,   -- server_version_num integer
//     "settings": {
//       "statement_timeout": string,
//       "max_connections":   string,
//       "work_mem":          string,
//       "shared_buffers":    string
//     },
//     "role": string           -- current_user
//   }
//
// All setting values are returned as-is from current_setting() — Postgres
// stores them as strings (e.g., "5000" for 5000ms, "128MB" for 128MB).
// SQL matches the server_info block in src/pg/queries/server_settings.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `server_info` tool call.
///
/// Acquires a connection from the pool and executes a single query that
/// returns the Postgres version, key server settings, and the connected
/// role. The connection is released before the response is constructed.
///
/// # Errors
///
/// Returns [`McpError::pg_pool_timeout`] if a connection cannot be acquired
/// within the configured timeout, or [`McpError::pg_query_failed`] if the
/// query fails.
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Single-row query returning version and 4 key settings.
    // Matches the server_info block in src/pg/queries/server_settings.sql.
    let row = client
        .query_one(
            "SELECT \
                version(), \
                current_setting('server_version_num')::int4, \
                current_user, \
                current_setting('statement_timeout'), \
                current_setting('max_connections'), \
                current_setting('work_mem'), \
                current_setting('shared_buffers')",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let version_string: String = row.get(0);
    let version_num: i32 = row.get(1);
    let role: String = row.get(2);
    let statement_timeout: String = row.get(3);
    let max_connections: String = row.get(4);
    let work_mem: String = row.get(5);
    let shared_buffers: String = row.get(6);

    // Release the connection before constructing the response.
    drop(client);

    let body = serde_json::json!({
        "version":     version_string,
        "version_num": version_num,
        "settings": {
            "statement_timeout": statement_timeout,
            "max_connections":   max_connections,
            "work_mem":          work_mem,
            "shared_buffers":    shared_buffers,
        },
        "role": role,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
