// tests/mcp_protocol.rs
//
// Integration tests for the MCP protocol layer.
//
// These tests start a PgMcpServer and connect to it via an in-process
// duplex transport (tokio::io::duplex). No network is required.
// A real Postgres container is needed because PgMcpServer holds a Pool.

mod common;

use std::sync::Arc;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::PgMcpServer,
};
use rmcp::{
    RoleClient, ServiceExt as _, model::CallToolRequestParams, serve_client,
    service::RunningService,
};

fn test_config(database_url: &str) -> Config {
    Config {
        database_url: database_url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 2,
            acquire_timeout_seconds: 5,
            idle_timeout_seconds: 60,
        },
        transport: TransportConfig::default(),
        telemetry: TelemetryConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailConfig::default(),
    }
}

/// Helper: create a connected server and client over an in-process channel.
///
/// Returns the running client service. The server runs in a background task.
async fn connect(pool: Arc<Pool>, config: Arc<Config>) -> RunningService<RoleClient, ()> {
    // duplex() gives us a bidirectional in-memory pipe. Each half implements
    // AsyncRead + AsyncWrite which maps to TransportAdapterAsyncCombinedRW.
    let (server_io, client_io) = tokio::io::duplex(65_536);

    // Start the server in a background task. The server owns one half of the
    // duplex pipe. It terminates when the client side is dropped.
    let cache = Arc::new(SchemaCache::empty());
    let handler = PgMcpServer::new(pool, cache, config);
    tokio::spawn(async move {
        if let Ok(running) = handler.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });

    // Start the client using the `()` (no-op) ClientHandler.
    // serve_client performs the MCP initialize handshake automatically.
    serve_client((), client_io).await.expect("client connect")
}

/// MCP handshake succeeds and the server reports the tools capability.
#[tokio::test]
async fn test_handshake_declares_tools_capability() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    let client: RunningService<RoleClient, ()> = connect(pool, config).await;

    // peer_info() is set by the MCP initialize response.
    let server_info = client.peer_info().expect("server info after handshake");

    assert!(
        server_info.capabilities.tools.is_some(),
        "server must declare tools capability"
    );
    assert_eq!(
        server_info.server_info.name, "pgmcp",
        "server name must be 'pgmcp'"
    );

    let _ = client.cancel().await;
}

/// tools/list returns a valid result (empty array in feat/006, 15 tools in feat/007).
#[tokio::test]
async fn test_tools_list_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    let client: RunningService<RoleClient, ()> = connect(pool, config).await;

    let result = client.list_tools(None).await.expect("list_tools");

    // In feat/006 the list is empty; feat/007 fills it with 15 tools.
    // This assertion is intentionally broad: we just ensure the Vec is present.
    let _ = result.tools.len(); // always >= 0; ensures the Vec is accessible

    let _ = client.cancel().await;
}

/// tools/call for an unknown tool returns an error result (not a protocol error).
#[tokio::test]
async fn test_unknown_tool_returns_error_result() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    let client: RunningService<RoleClient, ()> = connect(pool, config).await;

    let result = client
        .call_tool(CallToolRequestParams::new("totally_unknown_tool"))
        .await
        .expect("call_tool should not return a protocol error");

    // The server must set is_error: Some(true) for an unknown tool.
    assert_eq!(
        result.is_error,
        Some(true),
        "unknown tool must return an error result"
    );

    let _ = client.cancel().await;
}
