// src/server/mod.rs
//
// PgMcpServer — the core MCP server handler.
//
// Implements rmcp::ServerHandler. The MCP protocol layer (rmcp) dispatches
// requests to these methods. PgMcpServer is responsible for:
//   - Reporting server capabilities and identity to clients (get_info)
//   - Listing available tools (list_tools) — delegates to tool_defs
//   - Routing tool calls (call_tool) — delegates to router
//   - Holding shared state (pool, config) for injection into ToolContext
//
// PgMcpServer is Clone because the SSE transport creates one instance per
// client session via the StreamableHttpService factory closure.

#![allow(dead_code)]

pub(crate) mod context;
pub(crate) mod router;
pub(crate) mod tool_defs;

use std::sync::Arc;

use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    service::{MaybeSendFuture, RequestContext},
};

use crate::{config::Config, pg::pool::Pool};

use self::context::ToolContext;

/// The pgmcp MCP server handler.
///
/// Holds shared references to the connection pool and application config.
/// Implements `rmcp::ServerHandler` to process MCP protocol requests.
/// Clone is cheap — both fields are Arc-wrapped.
#[derive(Clone)]
pub struct PgMcpServer {
    pool: Arc<Pool>,
    config: Arc<Config>,
}

impl PgMcpServer {
    /// Create a new server handler.
    pub fn new(pool: Arc<Pool>, config: Arc<Config>) -> Self {
        Self { pool, config }
    }
}

impl ServerHandler for PgMcpServer {
    /// Return server identity and capabilities for the MCP initialize handshake.
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("pgmcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "pgmcp is a PostgreSQL MCP server. Use the available tools to \
                 inspect the database schema, execute SQL queries, and analyze \
                 query performance. Start with server_info or health to verify \
                 connectivity, then use list_schemas and list_tables to explore \
                 the schema, and query to run SQL.",
            )
    }

    /// Return the complete list of all 15 pgmcp tools.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_
    {
        std::future::ready(Ok(ListToolsResult {
            meta: None,
            tools: tool_defs::tool_list(),
            next_cursor: None,
        }))
    }

    /// Route a tool call to the appropriate handler via the dispatcher.
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + MaybeSendFuture + '_
    {
        let ctx = ToolContext::new(Arc::clone(&self.pool), Arc::clone(&self.config));
        async move {
            router::dispatch(ctx, request).await.or_else(|mcp_err| {
                // Convert McpError to CallToolResult::error so the protocol
                // always returns a well-formed response. Only truly unrecoverable
                // internal errors that cannot be expressed as a tool result should
                // propagate as ErrorData — those should be extremely rare.
                tracing::error!(error = %mcp_err, "tool handler returned McpError");
                Ok(CallToolResult::error(vec![Content::text(
                    mcp_err.to_json().to_string(),
                )]))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_declares_tools_capability() {
        // Verify that the ServerCapabilities builder correctly sets the tools
        // capability flag. This is a compile + runtime correctness check that
        // does not require a live pool.
        let caps = ServerCapabilities::builder().enable_tools().build();
        assert!(caps.tools.is_some(), "tools capability must be declared");
    }
}
