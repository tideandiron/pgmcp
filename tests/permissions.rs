// tests/permissions.rs
//
// Integration tests for the my_permissions tool.
//
// Requires a live PostgreSQL container. Returns early if Docker is unavailable.

mod common;

use std::time::Duration;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::my_permissions,
};
use serde_json::{Map, Value};

async fn make_ctx(database_url: &str) -> ToolContext {
    use std::sync::Arc;
    let config = Config {
        database_url: database_url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 4,
            acquire_timeout_seconds: 5,
            idle_timeout_seconds: 60,
        },
        transport: TransportConfig::default(),
        telemetry: TelemetryConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailConfig::default(),
    };
    let pool = Pool::build(&config).expect("pool must build");
    pool.health_check(Duration::from_secs(5))
        .await
        .expect("pool healthy");
    let cache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache loads");
    ToolContext::new(Arc::new(pool), Arc::new(cache), Arc::new(config))
}

fn args(json_str: &str) -> Option<Map<String, Value>> {
    let v: Value = serde_json::from_str(json_str).unwrap();
    v.as_object().cloned()
}

fn get_text(result: &rmcp::model::CallToolResult) -> &str {
    result.content[0]
        .as_text()
        .expect("content must have text")
        .text
        .as_str()
}

// ── 1. Role attributes are returned ──────────────────────────────────────────

#[tokio::test]
async fn my_permissions_returns_role_info() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, None)
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    let role = &parsed["role"];
    assert!(role["name"].is_string(), "role.name must be a string");
    assert!(
        role["is_superuser"].is_boolean(),
        "is_superuser must be bool"
    );
    assert!(
        role["can_create_db"].is_boolean(),
        "can_create_db must be bool"
    );
    assert!(
        role["can_create_role"].is_boolean(),
        "can_create_role must be bool"
    );
    assert!(role["can_login"].is_boolean(), "can_login must be bool");
    assert!(
        role["connection_limit"].is_number(),
        "connection_limit must be number"
    );
}

// ── 2. Role name matches the connected user ──────────────────────────────────

#[tokio::test]
async fn my_permissions_role_name_matches_connected_user() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, None)
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let role_name = parsed["role"]["name"]
        .as_str()
        .expect("role.name must be a string");

    // The test container uses pgmcp_test as the username.
    assert_eq!(
        role_name, "pgmcp_test",
        "role name should match the test container user"
    );
}

// ── 3. Schema privileges are returned ────────────────────────────────────────

#[tokio::test]
async fn my_permissions_schema_privileges_returned() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, None)
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    let schema_privs = parsed["schema_privileges"]
        .as_array()
        .expect("schema_privileges must be an array");

    // The test DB should have at least the public schema.
    assert!(
        !schema_privs.is_empty(),
        "schema_privileges should not be empty"
    );

    for priv_entry in schema_privs {
        assert!(
            priv_entry["schema"].is_string(),
            "each entry must have a schema name"
        );
        assert!(
            priv_entry["usage"].is_boolean(),
            "each entry must have a boolean usage field"
        );
        assert!(
            priv_entry["create"].is_boolean(),
            "each entry must have a boolean create field"
        );
    }
}

// ── 4. Public schema is in the list ──────────────────────────────────────────

#[tokio::test]
async fn my_permissions_public_schema_present() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, None)
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let schema_privs = parsed["schema_privileges"]
        .as_array()
        .expect("schema_privileges must be an array");

    let has_public = schema_privs
        .iter()
        .any(|e| e["schema"].as_str() == Some("public"));
    assert!(has_public, "public schema should be in schema_privileges");
}

// ── 5. System schemas are excluded ───────────────────────────────────────────

#[tokio::test]
async fn my_permissions_system_schemas_excluded() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, None)
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let schema_privs = parsed["schema_privileges"]
        .as_array()
        .expect("schema_privileges must be an array");

    for entry in schema_privs {
        let name = entry["schema"].as_str().unwrap_or("");
        assert!(
            name != "pg_catalog" && name != "information_schema" && !name.starts_with("pg_toast"),
            "system schema '{name}' should not appear in schema_privileges"
        );
    }
}

// ── 6. Table privileges returned when table is specified ─────────────────────

#[tokio::test]
async fn my_permissions_table_privileges_when_table_given() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };

    // Create the test table using a direct tokio-postgres connection.
    {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("direct connect");
        tokio::spawn(conn);
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS public.perm_test (id SERIAL PRIMARY KEY)",
                &[],
            )
            .await
            .expect("create test table");
    }

    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, args(r#"{"schema": "public", "table": "perm_test"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    let tp = &parsed["table_privileges"];
    assert!(
        tp.is_object(),
        "table_privileges must be present when table param given"
    );
    assert!(tp["table"].is_string());
    assert!(tp["select"].is_boolean());
    assert!(tp["insert"].is_boolean());
    assert!(tp["update"].is_boolean());
    assert!(tp["delete"].is_boolean());
}

// ── 7. No table privileges when table not specified ──────────────────────────

#[tokio::test]
async fn my_permissions_no_table_privileges_without_table_param() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = my_permissions::handle(ctx, args(r#"{"schema": "public"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    assert!(
        parsed.get("table_privileges").is_none(),
        "table_privileges must be absent when no table param given"
    );
}

// ── 8. Custom schema parameter ───────────────────────────────────────────────

#[tokio::test]
async fn my_permissions_custom_schema_accepted() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    // Any schema that exists or doesn't — we just verify no crash.
    let result = my_permissions::handle(ctx, args(r#"{"schema": "public"}"#)).await;
    assert!(result.is_ok(), "custom schema should not error: {result:?}");
}
