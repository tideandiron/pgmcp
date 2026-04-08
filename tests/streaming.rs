// tests/streaming.rs
//
// Integration tests for streaming serialization with live data.
//
// Tests here verify the JSON/CSV encoders against real PostgreSQL rows.
// Each test requires a Docker PostgreSQL container.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::query,
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
        .expect("pool must be healthy");

    let cache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache must load");

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

// ── JSON serialization ────────────────────────────────────────────────────────

#[tokio::test]
async fn streaming_json_encodes_all_basic_types() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let sql = "SELECT 1::int2 AS i2, 42::int4 AS i4, 9999999::int8 AS i8, 1.5::float4 AS f4, 2.718::float8 AS f8, true AS b, 'hello'::text AS t";

    let result = query::handle(ctx, args(&format!(r#"{{"sql": "{sql}"}}"#)))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);

    assert_eq!(rows[0]["i2"], 1);
    assert_eq!(rows[0]["i4"], 42);
    assert_eq!(rows[0]["i8"], 9999999);
    assert_eq!(rows[0]["b"], true);
    assert_eq!(rows[0]["t"], "hello");

    let f8 = rows[0]["f8"].as_f64().unwrap();
    assert!((f8 - 2.718).abs() < 0.001);
}

#[tokio::test]
async fn streaming_json_null_values_serialize_correctly() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT NULL::int4 AS ni, NULL::text AS nt"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();

    assert!(rows[0]["ni"].is_null(), "NULL int4 must be JSON null");
    assert!(rows[0]["nt"].is_null(), "NULL text must be JSON null");
}

// ── CSV serialization ─────────────────────────────────────────────────────────

#[tokio::test]
async fn streaming_csv_header_row_present() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 1::int4 AS n, 'hello'::text AS msg", "format": "csv"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let csv = parsed["rows"].as_str().unwrap();

    let first_line = csv.split('\n').next().unwrap_or("").trim_end_matches('\r');
    assert!(
        first_line.contains('n') || first_line.contains("msg"),
        "CSV header missing column names: got '{first_line}'"
    );
}

#[tokio::test]
async fn streaming_csv_empty_result_when_no_rows() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 1 WHERE false", "format": "csv"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 0);
    let csv = parsed["rows"].as_str().unwrap_or("");
    assert!(
        csv.is_empty(),
        "CSV with 0 rows should be empty string: got '{csv}'"
    );
}

// ── BatchSizer integration ────────────────────────────────────────────────────

#[tokio::test]
async fn streaming_batch_sizer_reports_row_count() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1, 200) AS n", "limit": 300}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 200);
}
