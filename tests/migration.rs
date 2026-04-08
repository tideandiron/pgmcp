// tests/migration.rs
//
// Integration tests for the propose_migration tool.
//
// Most analysis is pure (no DB needed), but we verify the version string
// comes from a real PG connection.
//
// Requires a live PostgreSQL container. Returns early if Docker unavailable.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::propose_migration,
};
use serde_json::{Map, Value};

async fn make_ctx(database_url: &str) -> ToolContext {
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

// ── 1. CREATE TABLE assessment ────────────────────────────────────────────────

#[tokio::test]
async fn migration_create_table_returns_assessment() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(
        ctx,
        args(r#"{"sql": "CREATE TABLE mig_test (id SERIAL PRIMARY KEY, name TEXT)"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    assert_eq!(parsed["statement_type"], "create_table");
    assert_eq!(parsed["is_destructive"], false);
    assert!(parsed["reverse_sql"].is_string(), "reverse_sql must be set");
    assert!(parsed["lock_type"].is_string());
    assert!(parsed["warnings"].is_array());
    assert!(parsed["suggestions"].is_array());
    assert!(parsed["pg_version"].is_string());
}

// ── 2. PG version is from real DB ────────────────────────────────────────────

#[tokio::test]
async fn migration_pg_version_from_real_db() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(ctx, args(r#"{"sql": "CREATE TABLE vtest (id INT)"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let ver = parsed["pg_version"].as_str().unwrap_or("unknown");
    assert_ne!(ver, "unknown", "pg_version should be detected from real DB");
    // CI matrix tests PG 14-17; default local image is postgres:16-alpine
    assert!(
        ver.starts_with("17")
            || ver.starts_with("16")
            || ver.starts_with("15")
            || ver.starts_with("14"),
        "expected modern PG version, got: {ver}"
    );
}

// ── 3. DROP TABLE is marked destructive ──────────────────────────────────────

#[tokio::test]
async fn migration_drop_table_is_destructive() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result =
        propose_migration::handle(ctx, args(r#"{"sql": "DROP TABLE IF EXISTS mig_test"}"#))
            .await
            .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert_eq!(parsed["statement_type"], "drop_table");
    assert_eq!(parsed["is_destructive"], true);
    assert_eq!(parsed["data_loss_risk"], "high");
    assert!(
        parsed["reverse_sql"].is_null(),
        "DROP TABLE should have null reverse_sql"
    );
}

// ── 4. CREATE INDEX suggests CONCURRENTLY ────────────────────────────────────

#[tokio::test]
async fn migration_create_index_warns_and_suggests_concurrent() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(
        ctx,
        args(r#"{"sql": "CREATE INDEX idx_test ON mig_test (name)"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let warnings = parsed["warnings"].as_array().unwrap();
    let suggestions = parsed["suggestions"].as_array().unwrap();

    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .is_some_and(|s| s.contains("ShareLock") || s.contains("writes"))),
        "should warn about write blocking: {warnings:?}"
    );
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str().is_some_and(|s| s.contains("CONCURRENTLY"))),
        "should suggest CONCURRENTLY: {suggestions:?}"
    );
}

// ── 5. SELECT is rejected ─────────────────────────────────────────────────────

#[tokio::test]
async fn migration_rejects_select() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(ctx, args(r#"{"sql": "SELECT * FROM users"}"#)).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}

// ── 6. INSERT is rejected ─────────────────────────────────────────────────────

#[tokio::test]
async fn migration_rejects_insert() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result =
        propose_migration::handle(ctx, args(r#"{"sql": "INSERT INTO t (x) VALUES (1)"}"#)).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}

// ── 7. ADD COLUMN has reverse SQL ────────────────────────────────────────────

#[tokio::test]
async fn migration_add_column_has_reverse() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(
        ctx,
        args(r#"{"sql": "ALTER TABLE orders ADD COLUMN note TEXT"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let rev = &parsed["reverse_sql"];
    assert!(rev.is_string(), "ADD COLUMN should have a reverse SQL");
    let rev_str = rev.as_str().unwrap();
    assert!(
        rev_str.contains("DROP COLUMN"),
        "reverse should DROP COLUMN: {rev_str}"
    );
    assert!(
        rev_str.contains("note"),
        "reverse should mention column name: {rev_str}"
    );
}

// ── 8. TRUNCATE is high data loss risk ───────────────────────────────────────

#[tokio::test]
async fn migration_truncate_data_loss_high() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(ctx, args(r#"{"sql": "TRUNCATE TABLE orders"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert_eq!(parsed["data_loss_risk"], "high");
    assert_eq!(parsed["is_destructive"], true);
}

// ── 9. Response fields are always present ────────────────────────────────────

#[tokio::test]
async fn migration_response_always_has_all_fields() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(
        ctx,
        args(r#"{"sql": "CREATE TABLE audit_log (id SERIAL, event TEXT)"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    for field in &[
        "sql",
        "statement_type",
        "is_destructive",
        "lock_type",
        "lock_risk",
        "downtime_risk",
        "data_loss_risk",
        "warnings",
        "suggestions",
        "pg_version",
    ] {
        assert!(
            parsed.get(*field).is_some(),
            "response must have '{field}' field"
        );
    }
}

// ── 10. CREATE INDEX CONCURRENTLY has low lock risk ───────────────────────────

#[tokio::test]
async fn migration_create_index_concurrently_low_risk() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = propose_migration::handle(
        ctx,
        args(r#"{"sql": "CREATE INDEX CONCURRENTLY idx_test_conc ON mig_test (name)"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert_eq!(parsed["lock_risk"], "low");
    assert_eq!(parsed["downtime_risk"], "none");
}
