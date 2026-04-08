// src/tools/list_enums.rs
//
// list_enums tool — returns all user-defined enum types with their labels.
//
// Parameters: none (lists all enums across all user schemas).
//
// Returns a JSON object:
//   {
//     "enums": [
//       {
//         "name":   string,
//         "schema": string,
//         "values": [string, ...]
//       },
//       ...
//     ]
//   }
//
// System schemas (pg_catalog, information_schema) are excluded.
// Labels are ordered by enumsortorder (float4), which preserves the order
// even when labels are added with ALTER TYPE … ADD VALUE.
// SQL matches src/pg/queries/list_enums.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `list_enums` tool call.
///
/// Acquires a connection and queries `pg_enum`, `pg_type`, and `pg_namespace`
/// to return all user-defined enum types with their ordered label values.
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

    // SQL matches src/pg/queries/list_enums.sql.
    let rows = client
        .query(
            "SELECT \
                t.typname, \
                n.nspname, \
                array_agg(e.enumlabel ORDER BY e.enumsortorder) \
             FROM pg_enum e \
             JOIN pg_type t ON e.enumtypid = t.oid \
             JOIN pg_namespace n ON t.typnamespace = n.oid \
             WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
             GROUP BY t.typname, n.nspname \
             ORDER BY n.nspname, t.typname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection before building the response.
    drop(client);

    let enums: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let schema: String = row.get(1);
            let values: Vec<String> = row.get(2);
            serde_json::json!({
                "name":   name,
                "schema": schema,
                "values": values,
            })
        })
        .collect();

    let body = serde_json::json!({ "enums": enums });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
