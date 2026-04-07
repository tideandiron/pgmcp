// src/tools/list_databases.rs
//
// list_databases tool — returns all databases on this Postgres instance.
//
// Returns a JSON object:
//   {
//     "databases": [
//       {
//         "name":        string,
//         "owner":       string,
//         "encoding":    string,
//         "size_bytes":  number | null,
//         "description": string | null
//       },
//       ...
//     ]
//   }
//
// The query in list_databases.sql returns NULL for size_bytes when
// datallowconn = false (e.g., template0).
// SQL matches src/pg/queries/list_databases.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `list_databases` tool call.
///
/// Acquires a connection and queries `pg_database` joined with `pg_roles`
/// and `pg_shdescription`. Returns all databases visible to the connected
/// role. Databases with `datallowconn = false` (e.g., `template0`) have a
/// `null` `size_bytes` to avoid a permission error from `pg_database_size`.
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

    // SQL matches src/pg/queries/list_databases.sql
    let rows = client
        .query(
            "SELECT \
                d.datname, \
                r.rolname, \
                pg_encoding_to_char(d.encoding), \
                CASE WHEN d.datallowconn THEN pg_database_size(d.oid) ELSE NULL END, \
                sd.description \
            FROM pg_database d \
            JOIN pg_roles r ON r.oid = d.datdba \
            LEFT JOIN pg_shdescription sd \
                ON sd.objoid = d.oid \
                AND sd.classoid = 'pg_database'::regclass \
            ORDER BY d.datname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection before constructing the response.
    drop(client);

    let databases: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let owner: String = row.get(1);
            let encoding: String = row.get(2);
            let size_bytes: Option<i64> = row.get(3);
            let description: Option<String> = row.get(4);
            serde_json::json!({
                "name":        name,
                "owner":       owner,
                "encoding":    encoding,
                "size_bytes":  size_bytes,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "databases": databases });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
