// tests/health.rs
//
// Integration tests for src/pg/pool.rs.
//
// These tests require Docker to be running. They spin up a real PostgreSQL
// container via testcontainers and verify pool construction, health check,
// version detection, and concurrent usage.
//
// Run with: cargo test --test health
//
// When Docker is not available the container tests are skipped automatically;
// only the synchronous error-path test runs unconditionally.

mod common;

use std::{sync::Arc, time::Duration};

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::pool::Pool,
    server::context::ToolContext,
    tools::{connection_info, health},
};
use serde_json::Value;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

/// Build a [`ToolContext`] pointing at the given database URL.
fn test_ctx(url: &str) -> ToolContext {
    let config = Arc::new(Config {
        database_url: url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 5,
            acquire_timeout_seconds: 10,
            idle_timeout_seconds: 60,
        },
        transport: TransportConfig::default(),
        telemetry: TelemetryConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailConfig::default(),
    });
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    ToolContext::new(pool, config)
}

/// Build a minimal [`Config`] pointing at the given database URL.
fn test_config(database_url: &str) -> Config {
    Config {
        database_url: database_url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 2,
            acquire_timeout_seconds: 10,
            idle_timeout_seconds: 60,
        },
        transport: TransportConfig::default(),
        telemetry: TelemetryConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailConfig::default(),
    }
}

/// Pool connects and can execute `SELECT 1`.
///
/// Skipped when Docker is not available.
#[tokio::test]
async fn test_pool_connects_to_postgres() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let config = test_config(&url);
    let pool = Pool::build(&config).expect("pool build must succeed");

    pool.health_check(ACQUIRE_TIMEOUT)
        .await
        .expect("health check must pass against a live Postgres instance");
}

/// Pool correctly reports the Postgres major version.
///
/// Skipped when Docker is not available.
#[tokio::test]
async fn test_version_check_passes_for_pg16() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let config = test_config(&url);
    let pool = Pool::build(&config).expect("pool build must succeed");

    let major = pool
        .check_pg_version(ACQUIRE_TIMEOUT)
        .await
        .expect("version check must succeed");

    assert!(major >= 14, "major version {major} should be >= 14");
    assert!(major <= 20, "major version {major} should be realistic");
}

/// `pool.pg_version_string()` returns a non-empty string starting with a digit.
///
/// Skipped when Docker is not available.
#[tokio::test]
async fn test_pg_version_string_is_non_empty() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let config = test_config(&url);
    let pool = Pool::build(&config).expect("pool build must succeed");

    let v = pool
        .pg_version_string(ACQUIRE_TIMEOUT)
        .await
        .expect("version string query must succeed");

    assert!(!v.is_empty(), "version string must not be empty");
    assert!(
        v.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "version string should start with a digit, got: {v}"
    );
}

/// Pool wrapped in `Arc` can be cloned and used concurrently from multiple tasks.
///
/// Skipped when Docker is not available.
#[tokio::test]
async fn test_pool_arc_clone_is_usable() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let config = test_config(&url);
    let pool = Arc::new(Pool::build(&config).expect("pool build must succeed"));

    let pool2 = Arc::clone(&pool);
    let h1 = tokio::spawn(async move {
        pool.health_check(ACQUIRE_TIMEOUT)
            .await
            .expect("task 1 health check");
    });
    let h2 = tokio::spawn(async move {
        pool2
            .health_check(ACQUIRE_TIMEOUT)
            .await
            .expect("task 2 health check");
    });
    h1.await.expect("task 1 join");
    h2.await.expect("task 2 join");
}

/// `Pool::build` with an invalid database URL returns `pg_connect_failed`.
///
/// This test does not require Docker and runs unconditionally.
#[test]
fn test_pool_build_invalid_url_returns_error() {
    let config = test_config("this-is-not-a-valid-url");
    let result = Pool::build(&config);
    assert!(result.is_err(), "expected error for invalid URL");
    let err = result.unwrap_err();
    let json = err.to_json();
    assert_eq!(json["code"], "pg_connect_failed");
}

// ── health tool integration tests ────────────────────────────────────────────

/// health tool returns `status: "ok"` and `pg_reachable: true` against a live
/// Postgres instance.
#[tokio::test]
async fn test_health_returns_ok() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = health::handle(test_ctx(&url), None)
        .await
        .expect("health must not error");
    let text = &result.content[0]
        .as_text()
        .expect("content must be text")
        .text;
    let v: Value = serde_json::from_str(text).expect("must be valid JSON");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["pg_reachable"], true);
    assert_eq!(v["pool_available"], true);
    assert!(v["latency_ms"].is_number());
    assert!(v["latency_ms"].as_f64().unwrap() >= 0.0);
    assert!(v["pool_stats"].is_object());
    assert!(v["pool_stats"]["size"].is_number());
    assert!(v["pool_stats"]["available"].is_number());
}

/// `latency_ms` is measured end-to-end and is reasonably accurate.
/// We accept up to 2000 ms on slow CI machines.
#[tokio::test]
async fn test_health_latency_is_non_negative_and_sane() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = health::handle(test_ctx(&url), None)
        .await
        .expect("health handle must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let latency = v["latency_ms"].as_f64().unwrap();
    assert!(latency >= 0.0, "latency must be >= 0ms, got {latency}");
    assert!(
        latency < 2000.0,
        "latency {latency}ms is unreasonably large"
    );
}

/// health tool returns all required JSON fields.
#[tokio::test]
async fn test_health_response_has_all_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = health::handle(test_ctx(&url), None)
        .await
        .expect("health handle must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &[
        "status",
        "pg_reachable",
        "pool_available",
        "latency_ms",
        "pool_stats",
    ] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}

// ── connection_info tool integration tests ────────────────────────────────────

/// connection_info returns all required fields.
#[tokio::test]
async fn test_connection_info_has_all_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = &result.content[0].as_text().expect("text content").text;
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &[
        "host",
        "port",
        "database",
        "role",
        "ssl",
        "server_version",
        "pool",
    ] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}

/// connection_info returns the correct database name.
#[tokio::test]
async fn test_connection_info_database_is_pgmcp_test() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["database"], "pgmcp_test");
}

/// connection_info returns the correct role name.
#[tokio::test]
async fn test_connection_info_role_is_pgmcp_test() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["role"], "pgmcp_test");
}

/// connection_info pool stats contain non-negative numbers.
#[tokio::test]
async fn test_connection_info_pool_stats_are_numbers() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let pool = &v["pool"];
    assert!(pool["size"].is_number(), "pool.size must be a number");
    assert!(
        pool["available"].is_number(),
        "pool.available must be a number"
    );
    assert!(pool["waiting"].is_number(), "pool.waiting must be a number");
}

/// `server_version` is a non-empty string starting with "PostgreSQL".
#[tokio::test]
async fn test_connection_info_server_version_is_postgres() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let version = v["server_version"].as_str().unwrap();
    assert!(
        version.starts_with("PostgreSQL"),
        "server_version should start with 'PostgreSQL', got: {version}"
    );
}
