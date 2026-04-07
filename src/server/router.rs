// src/server/router.rs
//
// Tool call dispatcher for pgmcp.
//
// Routes `tools/call` requests to the appropriate handler function.
// Unknown tool names return a tool_not_found error result.
// Known tools return a stub "not yet implemented" result.
//
// All 15 dispatch arms are present here. Handler implementations are in
// src/tools/<name>.rs and land in Phase 3 (feat/008 through feat/012) and
// Phase 6 (feat/018 through feat/023).
//
// This router has NO business logic. It is a routing table and context factory.

use rmcp::model::{CallToolRequestParams, CallToolResult, Content};

use crate::error::McpError;

use super::context::ToolContext;

/// Dispatch a tool call to the appropriate handler.
///
/// Returns `Ok(CallToolResult)` for known tools (even stub implementations).
/// Returns `Ok(CallToolResult::error(...))` for unknown tool names.
/// Returns `Err(McpError)` only for internal dispatcher failures.
pub(crate) async fn dispatch(
    ctx: ToolContext,
    request: CallToolRequestParams,
) -> Result<CallToolResult, McpError> {
    let name = request.name.as_ref();
    let args = request.arguments.clone();

    tracing::debug!(tool = name, "dispatching tool call");

    match name {
        // Discovery tools
        "list_databases" => crate::tools::list_databases::handle(ctx, args).await,
        "server_info" => crate::tools::server_info::handle(ctx, args).await,
        "list_schemas" => crate::tools::list_schemas::handle(ctx, args).await,
        "list_tables" => crate::tools::list_tables::handle(ctx, args).await,
        "describe_table" => crate::tools::describe_table::handle(ctx, args).await,
        "list_enums" => crate::tools::list_enums::handle(ctx, args).await,
        "list_extensions" => crate::tools::list_extensions::handle(ctx, args).await,
        "table_stats" => crate::tools::table_stats::handle(ctx, args).await,
        // SQL-accepting tools
        "query" => crate::tools::query::handle(ctx, args).await,
        "explain" => crate::tools::explain::handle(ctx, args).await,
        "suggest_index" => crate::tools::suggest_index::handle(ctx, args).await,
        "propose_migration" => crate::tools::propose_migration::handle(ctx, args).await,
        // Introspection tools
        "my_permissions" => crate::tools::my_permissions::handle(ctx, args).await,
        "connection_info" => crate::tools::connection_info::handle(ctx, args).await,
        "health" => crate::tools::health::handle(ctx, args).await,
        // Unknown
        unknown => {
            tracing::warn!(tool = unknown, "tool not found");
            Ok(CallToolResult::error(vec![Content::text(format!(
                "tool_not_found: '{unknown}' is not a known tool. \
                 Call tools/list to see the 15 available tools.",
            ))]))
        }
    }
}
