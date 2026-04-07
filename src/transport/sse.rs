// src/transport/sse.rs
//
// MCP Streamable HTTP (SSE) transport runner.
//
// Listens on a configurable host:port. Mounts StreamableHttpService at /mcp.
// Supports the full MCP Streamable HTTP spec: POST for client requests,
// GET for server-sent event streams, DELETE for session teardown.
//
// Session management uses rmcp's LocalSessionManager (in-memory, single-process).
// This is appropriate for MVP; a distributed session manager would be needed
// for horizontal scaling.

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use crate::{config::Config, error::McpError, pg::pool::Pool, server::PgMcpServer};

/// Run the MCP server over HTTP with SSE streaming.
///
/// Binds to `config.transport.host:config.transport.port`, mounts the MCP
/// protocol service at `/mcp`, and serves until the process is terminated or
/// `ct` is cancelled.
///
/// # Errors
///
/// Returns [`McpError`] if the TCP listener cannot be bound or axum
/// encounters a fatal error.
pub(crate) async fn run(
    pool: Arc<Pool>,
    config: Arc<Config>,
    ct: CancellationToken,
) -> Result<(), McpError> {
    let bind_addr = format!("{}:{}", config.transport.host, config.transport.port);

    let pool_clone = Arc::clone(&pool);
    let config_clone = Arc::clone(&config);

    let http_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(true)
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(15)))
        .with_cancellation_token(ct.child_token());

    // The factory closure is called once per MCP session (once per initialize
    // request). PgMcpServer::new is cheap — it only clones two Arcs.
    let service: StreamableHttpService<PgMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(PgMcpServer::new(
                    Arc::clone(&pool_clone),
                    Arc::clone(&config_clone),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            http_config,
        );

    let router = axum::Router::new().nest_service("/mcp", service);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| {
            McpError::internal(format!("failed to bind SSE transport to {bind_addr}: {e}"))
        })?;

    let actual_addr = listener
        .local_addr()
        .map_err(|e| McpError::internal(format!("could not get bound address: {e}")))?;

    tracing::info!(
        addr = %actual_addr,
        "MCP SSE transport listening"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move { ct.cancelled_owned().await })
        .await
        .map_err(|e| McpError::internal(format!("SSE transport server error: {e}")))?;

    tracing::info!("MCP SSE transport shut down");
    Ok(())
}
