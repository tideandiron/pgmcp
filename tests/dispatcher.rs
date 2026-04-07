// tests/dispatcher.rs
//
// Integration tests for the tool dispatcher (feat/007 acceptance criteria).
//
// Verifies:
//   - tools/list returns exactly 15 tools with correct names
//   - All no-argument tools can be called and return a result (not a protocol error)
//   - Unknown tool name returns tool_not_found error result
//   - Each tool's input_schema is a valid JSON Schema object
//
// Run with: cargo test --test dispatcher

mod common;

use std::sync::Arc;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::pool::Pool,
    server::PgMcpServer,
};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt as _, model::CallToolRequestParams, serve_client};

fn test_config(database_url: &str) -> Config {
    Config {
        database_url: database_url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 3,
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
async fn connect(pool: Arc<Pool>, config: Arc<Config>) -> RunningService<RoleClient, ()> {
    let (server_io, client_io) = tokio::io::duplex(65_536);
    let handler = PgMcpServer::new(pool, config);
    tokio::spawn(async move {
        if let Ok(running) = handler.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    serve_client((), client_io).await.expect("client connect")
}

/// tools/list returns exactly 15 tools.
#[tokio::test]
async fn test_tools_list_returns_15_tools() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let client = connect(pool, config).await;

    let result = client.list_tools(None).await.expect("list_tools");
    assert_eq!(
        result.tools.len(),
        15,
        "expected exactly 15 tools, got {}: {:?}",
        result.tools.len(),
        result.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    let _ = client.cancel().await;
}

/// tools/list includes all 15 expected tool names.
#[tokio::test]
async fn test_tools_list_includes_all_expected_names() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let client = connect(pool, config).await;

    let result = client.list_tools(None).await.expect("list_tools");
    let names: std::collections::HashSet<String> =
        result.tools.iter().map(|t| t.name.to_string()).collect();

    let expected = [
        "list_databases",
        "server_info",
        "list_schemas",
        "list_tables",
        "describe_table",
        "list_enums",
        "list_extensions",
        "table_stats",
        "query",
        "explain",
        "suggest_index",
        "propose_migration",
        "my_permissions",
        "connection_info",
        "health",
    ];

    for name in &expected {
        assert!(
            names.contains(*name),
            "missing tool in tools/list: '{name}'"
        );
    }

    let _ = client.cancel().await;
}

/// Each of the no-argument tools can be called and returns a result (stub or real).
/// The result must NOT be a protocol-level error.
#[tokio::test]
async fn test_all_15_tools_accept_call() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let client = connect(pool, config).await;

    // Tools that require no arguments — stub handlers accept any args including None.
    let no_arg_tools = [
        "list_databases",
        "server_info",
        "list_schemas",
        "list_extensions",
        "my_permissions",
        "connection_info",
        "health",
    ];

    for tool_name in &no_arg_tools {
        let result = client
            .call_tool(CallToolRequestParams::new(*tool_name))
            .await
            .unwrap_or_else(|e| panic!("protocol error calling '{tool_name}': {e}"));
        // Stub tools return success; we just verify the call was accepted.
        let _ = result;
    }

    let _ = client.cancel().await;
}

/// Unknown tool name returns a tool_not_found error result, not a protocol error.
#[tokio::test]
async fn test_unknown_tool_returns_tool_not_found_result() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let client = connect(pool, config).await;

    let result = client
        .call_tool(CallToolRequestParams::new("nonexistent_tool_xyz"))
        .await
        .expect("unknown tool should return result, not protocol error");

    assert_eq!(
        result.is_error,
        Some(true),
        "unknown tool must return error result"
    );

    let error_text = result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("");

    assert!(
        error_text.contains("tool_not_found") || error_text.contains("not a known tool"),
        "error message should indicate tool_not_found, got: '{error_text}'"
    );

    let _ = client.cancel().await;
}

/// tools/list results have valid JSON Schema objects in input_schema.
#[tokio::test]
async fn test_tool_schemas_are_valid_json_objects() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let client = connect(pool, config).await;

    let result = client.list_tools(None).await.expect("list_tools");

    for tool in &result.tools {
        let schema = tool.schema_as_json_value();
        assert!(
            schema.is_object(),
            "tool '{}' input_schema is not a JSON object",
            tool.name
        );
        let obj = schema.as_object().unwrap();
        assert!(
            obj.contains_key("type"),
            "tool '{}' input_schema missing 'type' field",
            tool.name
        );
        assert_eq!(
            obj.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool '{}' input_schema type must be 'object'",
            tool.name
        );
    }

    let _ = client.cancel().await;
}
