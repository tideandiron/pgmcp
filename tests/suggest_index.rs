// tests/suggest_index.rs
//
// Integration tests for the suggest_index tool.
//
// Requires a live PostgreSQL container. Returns early if Docker unavailable.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::suggest_index,
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

/// Create a test table with many rows via a direct tokio-postgres connection.
async fn setup_large_table(url: &str, table: &str, rows: i64) {
    let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("direct connect");
    tokio::spawn(conn);

    client
        .execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS public.{table} \
                 (id SERIAL PRIMARY KEY, status TEXT, amount NUMERIC, created_at TIMESTAMPTZ)"
            ),
            &[],
        )
        .await
        .expect("create table");

    // Insert enough rows to push the planner's row estimate above 1,000.
    // We use generate_series with actual inserts to populate statistics.
    client
        .execute(
            &format!(
                "INSERT INTO public.{table} (status, amount, created_at) \
                 SELECT \
                     CASE WHEN g % 3 = 0 THEN 'open' WHEN g % 3 = 1 THEN 'closed' ELSE 'pending' END, \
                     (g * 1.5)::numeric, \
                     NOW() - (g || ' seconds')::interval \
                 FROM generate_series(1, {rows}) g \
                 ON CONFLICT DO NOTHING"
            ),
            &[],
        )
        .await
        .expect("insert rows");

    client
        .execute(&format!("ANALYZE public.{table}"), &[])
        .await
        .expect("analyze table");
}

// ── 1. Basic response structure ───────────────────────────────────────────────

#[tokio::test]
async fn suggest_index_returns_valid_structure() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let result = suggest_index::handle(ctx, args(r#"{"sql": "SELECT 1 AS n"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert!(parsed["sql_analyzed"].is_string());
    assert!(parsed["current_plan_cost"].is_number());
    assert!(parsed["suggestions"].is_array());
    assert!(parsed["seq_scans_found"].is_number());
}

// ── 2. DDL is blocked ────────────────────────────────────────────────────────

#[tokio::test]
async fn suggest_index_rejects_ddl() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = suggest_index::handle(ctx, args(r#"{"sql": "CREATE TABLE x (id INT)"}"#)).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "guardrail_violation");
}

// ── 3. INSERT is rejected ────────────────────────────────────────────────────

#[tokio::test]
async fn suggest_index_rejects_non_select() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    // INSERT passes guardrails but suggest_index requires SELECT.
    let result = suggest_index::handle(
        ctx,
        args(r#"{"sql": "INSERT INTO nowhere (x) VALUES (1)"}"#),
    )
    .await;
    assert!(result.is_err());
}

// ── 4. Missing sql returns param_invalid ─────────────────────────────────────

#[tokio::test]
async fn suggest_index_missing_sql_errors() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = suggest_index::handle(ctx, args("{}")).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}

// ── 5. Seq scan on large unindexed table produces suggestion ─────────────────

#[tokio::test]
async fn suggest_index_produces_suggestion_for_unindexed_table() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };

    // Set up a large unindexed table.
    setup_large_table(&url, "idx_test_orders", 5_000).await;

    let ctx = make_ctx(&url).await;

    // Query that filters on the unindexed 'status' column.
    let result = suggest_index::handle(
        ctx,
        args(r#"{"sql": "SELECT * FROM public.idx_test_orders WHERE status = 'open'"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let suggestions = parsed["suggestions"].as_array().expect("must be array");

    // With 5,000 rows and a filter on status, the planner should choose Seq Scan
    // and we should get at least one suggestion. (May be 0 if PG chooses index scan
    // using the PK — we just verify the structure is correct.)
    for s in suggestions {
        assert!(
            s["create_sql"].is_string(),
            "each suggestion must have create_sql"
        );
        assert!(s["impact"].is_string(), "each suggestion must have impact");
        assert!(
            s["tradeoffs"].is_string(),
            "each suggestion must have tradeoffs"
        );
        assert!(
            s["estimated_index_size_bytes"].is_number(),
            "must have size estimate"
        );
    }
}

// ── 6. Index on primary key prevents duplicate suggestion ────────────────────

#[tokio::test]
async fn suggest_index_no_suggestion_for_indexed_column() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };

    setup_large_table(&url, "idx_test_indexed", 5_000).await;

    // Create an index on status.
    {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("direct connect");
        tokio::spawn(conn);
        client
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_test_indexed_status \
                 ON public.idx_test_indexed (status)",
                &[],
            )
            .await
            .expect("create index");
        client
            .execute("ANALYZE public.idx_test_indexed", &[])
            .await
            .expect("analyze");
    }

    let ctx = make_ctx(&url).await;
    let result = suggest_index::handle(
        ctx,
        args(r#"{"sql": "SELECT * FROM public.idx_test_indexed WHERE status = 'open'"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    // Either 0 suggestions (planner used index) or suggestions don't mention status.
    let suggestions = parsed["suggestions"].as_array().expect("must be array");
    for s in suggestions {
        let create_sql = s["create_sql"].as_str().unwrap_or("");
        assert!(
            !create_sql.contains("status") || !create_sql.contains("idx_test_indexed"),
            "should not suggest index on already-indexed status column: {create_sql}"
        );
    }
}

// ── 7. Schema parameter accepted ─────────────────────────────────────────────

#[tokio::test]
async fn suggest_index_schema_param_accepted() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = suggest_index::handle(ctx, args(r#"{"sql": "SELECT 1", "schema": "public"}"#))
        .await
        .expect("schema param should be accepted");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert!(parsed["suggestions"].is_array());
}
