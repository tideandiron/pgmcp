// src/transport/stdio.rs
//
// MCP stdio transport runner.
//
// Reads newline-delimited JSON-RPC from stdin, writes to stdout.
// This is the canonical MCP transport for process-launched servers (e.g.,
// when invoked by Claude Desktop or another MCP client via subprocess spawn).
//
// Tracing must be configured for stderr output before calling run()
// because stdout is claimed exclusively by the MCP protocol wire format.
//
// run() does not return until the transport is closed (client disconnects
// or stdin reaches EOF).

use std::sync::Arc;

use rmcp::ServiceExt as _;

use crate::{
    config::Config,
    error::McpError,
    pg::{cache::SchemaCache, pool::Pool},
    server::PgMcpServer,
};

/// Run the MCP server over stdin/stdout.
///
/// Blocks until the client closes the connection or an unrecoverable error
/// occurs. Returns `Ok(())` on clean shutdown.
///
/// # Errors
///
/// Returns [`McpError`] if rmcp fails to initialize the protocol handshake.
/// Transport-level errors (EOF, broken pipe) are treated as clean shutdown
/// and return `Ok(())`.
pub(crate) async fn run(
    pool: Arc<Pool>,
    cache: Arc<SchemaCache>,
    config: Arc<Config>,
) -> Result<(), McpError> {
    let handler = PgMcpServer::new(pool, cache, config);
    // Returns (Stdin, Stdout); the tuple implements IntoTransport via TransportAdapterAsyncRW.
    let transport: (tokio::io::Stdin, tokio::io::Stdout) = rmcp::transport::io::stdio();

    tracing::info!("starting MCP stdio transport");

    handler
        .serve(transport)
        .await
        .map_err(|e| McpError::internal(format!("MCP stdio handshake failed: {e}")))?
        .waiting()
        .await
        .map_err(|e| McpError::internal(format!("MCP stdio server error: {e}")))?;

    tracing::info!("MCP stdio transport closed");
    Ok(())
}
