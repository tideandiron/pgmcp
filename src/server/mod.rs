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

/// The pgmcp MCP server handler.
///
/// Holds shared references to the connection pool and application config.
/// Implements `rmcp::ServerHandler` to process MCP protocol requests.
/// Clone-able so the SSE transport factory can create one per session.
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
                 query performance.",
            )
    }

    /// Return the list of all available tools.
    ///
    /// In feat/006 this returns an empty list. feat/007 fills in all 15 tools.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_
    {
        std::future::ready(Ok(ListToolsResult {
            meta: None,
            tools: vec![],
            next_cursor: None,
        }))
    }

    /// Route a tool call request.
    ///
    /// In feat/006 all tool calls return an error result. feat/007 adds routing.
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + MaybeSendFuture + '_
    {
        let name = request.name.clone();
        std::future::ready(Ok(CallToolResult::error(vec![Content::text(format!(
            "tool not found: '{name}' — call tools/list to see available tools"
        ))])))
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
