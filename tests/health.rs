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
};

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

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
