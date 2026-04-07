// src/tools/connection_info.rs
//
// connection_info tool — reports metadata about the current Postgres connection.
//
// Returns a JSON object:
//   {
//     "host":           string,
//     "port":           number,
//     "database":       string,
//     "role":           string,
//     "ssl":            bool,
//     "server_version": string,
//     "pool": {
//       "size":      usize,
//       "available": usize,
//       "waiting":   usize,
//     }
//   }
//
// SQL mirrors src/pg/queries/server_settings.sql — that file is the canonical
// reference for the query logic used here.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `connection_info` tool call.
///
/// Acquires a connection from the pool, executes the session-info query, and
/// returns a JSON snapshot of the connection metadata. The connection is
/// released before pool statistics are sampled so that the `available` count
/// reflects the post-query state.
///
/// # Errors
///
/// Returns [`McpError::pg_pool_timeout`] if a connection cannot be acquired
/// within the configured timeout, or [`McpError::pg_query_failed`] if the
/// session-info query fails.
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Query matches src/pg/queries/server_settings.sql
    let row = client
        .query_one(
            "SELECT \
                current_user, \
                current_database(), \
                COALESCE(current_setting('listen_addresses', true), 'localhost'), \
                COALESCE(current_setting('port', true)::int4, 5432), \
                version(), \
                COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false)",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let role: String = row.get(0);
    let database: String = row.get(1);
    let host: String = row.get(2);
    let port: i32 = row.get(3);
    let server_version: String = row.get(4);
    let ssl: bool = row.get(5);

    // Release the connection before sampling pool stats so that the
    // `available` count reflects the state after the query completes.
    drop(client);

    let pool_status = ctx.pool.inner().status();

    let body = serde_json::json!({
        "host":           host,
        "port":           port,
        "database":       database,
        "role":           role,
        "ssl":            ssl,
        "server_version": server_version,
        "pool": {
            "size":      pool_status.size,
            "available": pool_status.available,
            "waiting":   pool_status.waiting,
        }
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
