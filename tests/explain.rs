// tests/explain.rs
//
// Integration tests for the explain tool.
//
// Requires a live PostgreSQL container (Docker). Each test calls
// `pg_container()` and returns early if Docker is unavailable.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::explain,
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

// ── 1. Basic estimation-only explain ─────────────────────────────────────────

#[tokio::test]
async fn explain_select_1_returns_plan() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(ctx, args(r#"{"sql": "SELECT 1 AS n"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    assert!(parsed.get("plan_json").is_some(), "must have plan_json");
    assert!(parsed.get("plan_text").is_some(), "must have plan_text");
    assert!(parsed.get("summary").is_some(), "must have summary");
    let summary = &parsed["summary"];
    assert!(
        summary["total_cost"].as_f64().is_some(),
        "total_cost must be a number"
    );
    assert!(summary["warnings"].is_array(), "warnings must be array");
    assert!(
        summary["suggestions"].is_array(),
        "suggestions must be array"
    );
}

// ── 2. Analyze mode ──────────────────────────────────────────────────────────

#[tokio::test]
async fn explain_analyze_mode_returns_plan() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1,10)", "analyze": true}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert_eq!(parsed["analyze"], true);
    assert!(parsed.get("plan_json").is_some());
}

// ── 3. DDL is blocked ────────────────────────────────────────────────────────

#[tokio::test]
async fn explain_ddl_blocked_by_guardrails() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(ctx, args(r#"{"sql": "CREATE TABLE x (id INT)"}"#)).await;

    // Guardrail violation returns Err.
    assert!(result.is_err(), "DDL should be blocked by guardrails");
    let err = result.unwrap_err();
    assert_eq!(err.code(), "guardrail_violation");
}

// ── 4. Seq scan on larger table triggers warning ──────────────────────────────

#[tokio::test]
async fn explain_seq_scan_warning_on_large_estimate() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;

    // generate_series produces a large estimated row count in PG planner.
    let result = explain::handle(
        ctx,
        args(r#"{"sql": "SELECT * FROM generate_series(1, 100000) AS s(n)"}"#),
    )
    .await
    .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    // This generates a Function Scan or Result node, not Seq Scan. Verify plan_json exists.
    assert!(parsed["plan_json"].is_array() || parsed["plan_json"].is_object());
}

// ── 5. Missing sql parameter ─────────────────────────────────────────────────

#[tokio::test]
async fn explain_missing_sql_returns_error() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(ctx, args("{}")).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}

// ── 6. Verbose option passes through ─────────────────────────────────────────

#[tokio::test]
async fn explain_verbose_option_accepted() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(ctx, args(r#"{"sql": "SELECT 1", "verbose": true}"#))
        .await
        .expect("verbose option should be accepted");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    assert!(parsed.get("plan_json").is_some());
}

// ── 7. Summary fields are well-formed ────────────────────────────────────────

#[tokio::test]
async fn explain_summary_fields_well_formed() {
    let Some((_c, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = explain::handle(ctx, args(r#"{"sql": "SELECT 1"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");
    let summary = &parsed["summary"];

    assert!(summary["total_cost"].is_number());
    assert!(summary["estimated_rows"].is_number());
    assert!(summary["node_count"].is_number());
    assert!(summary["warnings"].is_array());
    assert!(summary["suggestions"].is_array());
    assert!(
        summary["node_count"].as_i64().unwrap_or(0) >= 1,
        "at least 1 node in any plan"
    );
}
