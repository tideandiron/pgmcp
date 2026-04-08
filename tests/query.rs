// tests/query.rs
//
// Integration tests for the query tool.
//
// These tests require a live PostgreSQL container (Docker). Each test calls
// `pg_container()` and returns early (skip) if Docker is unavailable.
//
// Test categories:
//   1. Basic SELECT execution — returns rows in JSON format
//   2. dry_run — returns parse analysis without executing
//   3. LIMIT injection — default limit is injected for SELECT
//   4. Guardrail enforcement — DDL is blocked
//   5. CSV format — rows returned as CSV string
//   6. explain — EXPLAIN plan returned
//   7. Error handling — bad SQL returns structured error
//   8. transaction wrapping
//   9. Type fast paths

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

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/// Extract the text content from a `CallToolResult`.
fn get_text(result: &rmcp::model::CallToolResult) -> &str {
    result.content[0]
        .as_text()
        .expect("content must have text")
        .text
        .as_str()
}

// ── 1. Basic SELECT ───────────────────────────────────────────────────────────

#[tokio::test]
async fn query_select_1_returns_json_result() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1 AS n"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).expect("valid JSON");

    assert!(parsed["rows"].is_array(), "rows must be a JSON array");
    assert_eq!(parsed["row_count"], 1);
    assert_eq!(parsed["format"], "json");
}

#[tokio::test]
async fn query_select_multiple_rows() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT generate_series(1, 5) AS n"}"#))
        .await
        .expect("handle must not error");

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 5);
    let rows = parsed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
}

#[tokio::test]
async fn query_returns_column_metadata() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 42::int4 AS answer, 'hello'::text AS greeting"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let columns = parsed["columns"].as_array().unwrap();
    assert_eq!(columns.len(), 2);

    let col0 = &columns[0];
    assert_eq!(col0["name"], "answer");
    assert_eq!(col0["type"], "int4");

    let col1 = &columns[1];
    assert_eq!(col1["name"], "greeting");
    assert_eq!(col1["type"], "text");
}

// ── 2. dry_run ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn query_dry_run_select_no_execution() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1", "dry_run": true}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();

    assert_eq!(parsed["dry_run"], true);
    assert_eq!(parsed["guardrails_passed"], true);
    assert!(parsed["row_count"].is_null());
}

#[tokio::test]
async fn query_dry_run_ddl_shows_guardrail_failure() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "DROP TABLE nonexistent", "dry_run": true}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();

    assert_eq!(parsed["dry_run"], true);
    assert_eq!(parsed["guardrails_passed"], false);
    assert!(
        !parsed["guardrail_error"].is_null(),
        "guardrail_error should be set for DDL"
    );
}

// ── 3. LIMIT injection ────────────────────────────────────────────────────────

#[tokio::test]
async fn query_select_without_limit_gets_limit_injected() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1, 50) AS n"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();

    assert_eq!(parsed["limit_injected"], true);
    assert_eq!(parsed["row_count"], 50);
}

#[tokio::test]
async fn query_select_with_custom_limit() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1, 100) AS n", "limit": 5}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();

    assert_eq!(parsed["limit_injected"], true);
    assert_eq!(parsed["row_count"], 5);
}

// ── 4. Guardrail enforcement ──────────────────────────────────────────────────

#[tokio::test]
async fn query_ddl_is_blocked() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "CREATE TABLE blocked_table (id INT)"}"#),
    )
    .await;

    let err = result.unwrap_err();
    assert_eq!(err.code(), "guardrail_violation");
}

#[tokio::test]
async fn query_delete_without_where_is_blocked() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "DELETE FROM pg_class"}"#)).await;

    let err = result.unwrap_err();
    assert_eq!(err.code(), "guardrail_violation");
}

// ── 5. CSV format ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn query_csv_format_returns_string_rows() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 1 AS n, 'hello'::text AS msg", "format": "csv"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();

    assert_eq!(parsed["format"], "csv");
    assert!(
        parsed["rows"].is_string(),
        "CSV rows must be a string, got: {}",
        parsed["rows"]
    );

    let csv_content = parsed["rows"].as_str().unwrap();
    assert!(!csv_content.is_empty(), "CSV content must not be empty");
}

// ── 6. explain ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn query_explain_true_returns_valid_response() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1", "explain": true}"#))
        .await
        .unwrap();

    // When explain=true, the response is structurally valid.
    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert!(parsed.is_object(), "response must be a JSON object");
}

// ── 7. Error handling ─────────────────────────────────────────────────────────

#[tokio::test]
async fn query_bad_sql_returns_parse_error() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELEKT BROEKN SQL"}"#)).await;

    let err = result.unwrap_err();
    assert_eq!(err.code(), "sql_parse_error");
}

#[tokio::test]
async fn query_missing_sql_returns_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{}"#)).await;

    let err = result.unwrap_err();
    assert_eq!(err.code(), "param_invalid");
}

#[tokio::test]
async fn query_nonexistent_table_returns_pg_query_failed() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT * FROM definitely_does_not_exist_table_xyz"}"#),
    )
    .await;

    let err = result.unwrap_err();
    assert_eq!(err.code(), "pg_query_failed");
}

// ── 8. transaction wrapping ───────────────────────────────────────────────────

#[tokio::test]
async fn query_transaction_true_selects_work() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 1 AS n", "transaction": true}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 1);
}

// ── 9. Type fast paths ────────────────────────────────────────────────────────

#[tokio::test]
async fn query_int4_and_text_types() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 42::int4 AS n, 'world'::text AS s"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["n"], 42);
    assert_eq!(rows[0]["s"], "world");
}

#[tokio::test]
async fn query_bool_type() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT true::bool AS t, false::bool AS f"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    assert_eq!(rows[0]["t"], true);
    assert_eq!(rows[0]["f"], false);
}

#[tokio::test]
async fn query_null_value_serializes_as_null() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT NULL::text AS n"}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    assert!(rows[0]["n"].is_null(), "NULL should serialize as JSON null");
}

#[tokio::test]
async fn query_float8_type() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    // Use a float that doesn't approximate a named constant to avoid clippy::approx_constant.
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1.5::float8 AS val"}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    let val = rows[0]["val"].as_f64().unwrap();
    assert!((val - 1.5_f64).abs() < f64::EPSILON * 10.0);
}

#[tokio::test]
async fn query_uuid_type() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT '550e8400-e29b-41d4-a716-446655440000'::uuid AS id"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let rows = parsed["rows"].as_array().unwrap();
    let id = rows[0]["id"].as_str().unwrap();
    assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
}

#[tokio::test]
async fn query_execution_time_is_reported() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1"}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let ms = parsed["execution_time_ms"].as_f64().unwrap();
    assert!(ms >= 0.0, "execution_time_ms must be non-negative");
}

#[tokio::test]
async fn query_sql_executed_field_present() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1 AS x"}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert!(
        parsed["sql_executed"].is_string(),
        "sql_executed must be present and a string"
    );
}

// ── 10. truncated indicator ───────────────────────────────────────────────────

#[tokio::test]
async fn query_truncated_false_when_rows_below_limit() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    // 3 rows with limit 10 — not truncated
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1, 3) AS n", "limit": 10}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 3, "row_count must be accurate");
    assert_eq!(
        parsed["truncated"], false,
        "truncated must be false when rows < limit"
    );
}

#[tokio::test]
async fn query_truncated_true_when_rows_equal_limit() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    // Generate 5 rows with limit 5 — result hits limit exactly → truncated
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT generate_series(1, 10) AS n", "limit": 5}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["row_count"], 5, "row_count must equal the limit");
    assert_eq!(
        parsed["truncated"], true,
        "truncated must be true when row_count == limit"
    );
}

#[tokio::test]
async fn query_response_has_all_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(ctx, args(r#"{"sql": "SELECT 1 AS x"}"#))
        .await
        .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let required = [
        "columns",
        "rows",
        "row_count",
        "truncated",
        "format",
        "sql_executed",
        "limit_injected",
        "execution_time_ms",
        "plan",
    ];
    for field in &required {
        assert!(
            parsed.get(*field).is_some(),
            "response must contain field '{field}'"
        );
    }
}

#[tokio::test]
async fn query_csv_format_has_header_row() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 42::int4 AS answer, 'hi'::text AS greeting", "format": "csv"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    let csv = parsed["rows"].as_str().expect("CSV must be a string");
    // First line is the header
    let first_line = csv.lines().next().expect("CSV must have at least one line");
    assert!(
        first_line.contains("answer") && first_line.contains("greeting"),
        "CSV header must contain column names, got: '{first_line}'"
    );
}

#[tokio::test]
async fn query_json_compact_format_is_valid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        return;
    };
    let ctx = make_ctx(&url).await;
    let result = query::handle(
        ctx,
        args(r#"{"sql": "SELECT 1 AS x", "format": "json_compact"}"#),
    )
    .await
    .unwrap();

    let parsed: Value = serde_json::from_str(get_text(&result)).unwrap();
    assert_eq!(parsed["format"], "json_compact");
    assert!(parsed["rows"].is_array(), "json_compact rows must be array");
    assert_eq!(parsed["row_count"], 1);
}
