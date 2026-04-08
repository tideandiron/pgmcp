// tests/schema_cache.rs
//
// Integration tests for the schema cache (feat/013).
//
// All tests that require Postgres use the testcontainers fixture from
// tests/common/fixtures.rs. Tests are skipped when Docker is unavailable.
//
// Run with: cargo test --test schema_cache -- --nocapture

mod common;

use std::sync::Arc;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::{health, list_enums, list_extensions, list_schemas, list_tables, table_stats},
};
use serde_json::Value;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn test_config(database_url: &str) -> Config {
    Config {
        database_url: database_url.to_string(),
        pool: PoolConfig {
            min_size: 1,
            max_size: 5,
            acquire_timeout_seconds: 10,
            idle_timeout_seconds: 60,
        },
        transport: TransportConfig::default(),
        telemetry: TelemetryConfig::default(),
        cache: CacheConfig {
            invalidation_interval_seconds: 1, // short interval for tests
        },
        guardrails: GuardrailConfig::default(),
    }
}

async fn test_ctx_with_warm_cache(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let warmed: SchemaCache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache warm-up must succeed");
    let cache = Arc::new(warmed);
    ToolContext::new(pool, cache, config)
}

fn test_ctx_with_empty_cache(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache = Arc::new(SchemaCache::empty());
    ToolContext::new(pool, cache, config)
}

// ─── Cache warm-up ────────────────────────────────────────────────────────────

/// Warm cache must contain at least the `public` schema.
#[tokio::test]
async fn test_cache_warmup_populates_schemas() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache: SchemaCache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache warm-up must succeed");

    let schemas = cache.get_schemas().await;
    assert!(
        !schemas.is_empty(),
        "warm cache must contain at least one schema"
    );
    assert!(
        schemas.iter().any(|s| s.name == "public"),
        "public schema must be present"
    );
}

/// Warm cache must have a non-zero `captured_at` timestamp.
#[tokio::test]
async fn test_cache_warmup_sets_captured_at() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache: SchemaCache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache warm-up must succeed");

    let age = cache.age_seconds().await;
    // A freshly warmed cache is recent; age must be much less than 50 years.
    assert!(
        age < 50 * 365 * 24 * 3600,
        "freshly warmed cache should not have a very old timestamp; age={age}"
    );
}

// ─── list_schemas cache paths ─────────────────────────────────────────────────

/// list_schemas serves from the cache when the cache is warm.
#[tokio::test]
async fn test_list_schemas_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = list_schemas::handle(ctx, None)
        .await
        .expect("list_schemas must succeed");

    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    assert!(!schemas.is_empty());
    assert!(schemas.iter().any(|s| s["name"] == "public"));
}

/// list_schemas falls back to Postgres when the cache is empty.
#[tokio::test]
async fn test_list_schemas_fallback_on_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    // Use an empty cache — forces the live-query path.
    let ctx = test_ctx_with_empty_cache(&url);
    let result = list_schemas::handle(ctx, None)
        .await
        .expect("list_schemas fallback must succeed");

    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(
        v["schemas"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "public")
    );
}

// ─── list_tables cache paths ──────────────────────────────────────────────────

/// list_tables cache-first path returns correct kind filtering.
#[tokio::test]
async fn test_list_tables_cache_kind_filter() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Create a test table and a view.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS public.cache_test_tbl (id int); \
                 CREATE OR REPLACE VIEW public.cache_test_view AS SELECT 1 AS x;",
            )
            .await
            .unwrap();
    }

    let warmed: SchemaCache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache warm-up");
    let cache = Arc::new(warmed);
    let ctx = ToolContext::new(Arc::clone(&pool), Arc::clone(&cache), Arc::clone(&config));

    // "table" kind should include cache_test_tbl but not cache_test_view.
    let args =
        serde_json::from_str::<serde_json::Map<_, _>>(r#"{"schema":"public","kind":"table"}"#)
            .unwrap();
    let result = list_tables::handle(ctx.clone(), Some(args))
        .await
        .expect("list_tables must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert!(tables.iter().any(|t| t["name"] == "cache_test_tbl"));
    assert!(!tables.iter().any(|t| t["name"] == "cache_test_view"));

    // "view" kind should include cache_test_view but not cache_test_tbl.
    let args2 =
        serde_json::from_str::<serde_json::Map<_, _>>(r#"{"schema":"public","kind":"view"}"#)
            .unwrap();
    let result2 = list_tables::handle(ctx, Some(args2))
        .await
        .expect("list_tables must succeed");
    let text2 = &result2.content[0].as_text().unwrap().text;
    let v2: Value = serde_json::from_str(text2).unwrap();
    let tables2 = v2["tables"].as_array().unwrap();
    assert!(tables2.iter().any(|t| t["name"] == "cache_test_view"));
    assert!(!tables2.iter().any(|t| t["name"] == "cache_test_tbl"));
}

/// list_tables falls back to Postgres when the cache returns no results.
#[tokio::test]
async fn test_list_tables_fallback_on_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_empty_cache(&url);
    let args = serde_json::from_str::<serde_json::Map<_, _>>(r#"{"schema":"public","kind":"all"}"#)
        .unwrap();
    // With an empty cache, the tool must fall through to PG without panicking.
    let result = list_tables::handle(ctx, Some(args)).await;
    // Even if public has no tables, this must succeed (empty array is fine).
    assert!(
        result.is_ok(),
        "list_tables fallback must succeed: {result:?}"
    );
}

// ─── Cache invalidation ───────────────────────────────────────────────────────

/// Cache refreshes after a new table is created.
#[tokio::test]
async fn test_cache_refreshes_after_new_table() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Populate cache before creating the new table.
    let warmed: SchemaCache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("initial cache warm-up");
    let cache = Arc::new(warmed);

    // Verify the table is not yet in cache.
    let initial_tables: Vec<_> = cache.get_tables("public", &[]).await;
    assert!(!initial_tables.iter().any(|t| t.name == "phase4_new_table"));

    // Create the table.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS public.phase4_new_table (id int PRIMARY KEY)",
                &[],
            )
            .await
            .unwrap();
    }

    // Spawn the invalidation task with a 1-second interval.
    let _handle = pgmcp::pg::invalidation::spawn_invalidation_task(
        Arc::clone(&cache),
        Arc::clone(&pool),
        1, // 1-second interval
    );

    // Wait up to 5 seconds for the cache to pick up the new table.
    let mut found = false;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let tables: Vec<_> = cache.get_tables("public", &[]).await;
        if tables.iter().any(|t| t.name == "phase4_new_table") {
            found = true;
            break;
        }
    }

    assert!(
        found,
        "cache must contain phase4_new_table after invalidation refresh"
    );
}

// ─── list_enums cache path ────────────────────────────────────────────────────

/// list_enums returns results when the cache is warm.
#[tokio::test]
async fn test_list_enums_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Create a test enum if it does not exist.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .execute(
                "DO $$ BEGIN \
                 IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'cache_test_mood') THEN \
                     CREATE TYPE public.cache_test_mood AS ENUM ('happy', 'sad', 'neutral'); \
                 END IF; \
                 END $$",
                &[],
            )
            .await
            .ok(); // ignore error if already exists
    }

    let warmed_enums: SchemaCache = SchemaCache::load_from_pool(&pool).await.unwrap();
    let cache = Arc::new(warmed_enums);
    let ctx = ToolContext::new(pool, Arc::clone(&cache), config);

    let result = list_enums::handle(ctx, None)
        .await
        .expect("list_enums must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(v["enums"].is_array(), "response must have enums array");

    // Verify the enum we created is present.
    let enums = v["enums"].as_array().unwrap();
    assert!(
        enums.iter().any(|e| e["name"] == "cache_test_mood"),
        "cache_test_mood must appear in the enums list"
    );
}

/// list_enums falls back to Postgres when the cache is empty.
#[tokio::test]
async fn test_list_enums_fallback_on_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_empty_cache(&url);
    let result = list_enums::handle(ctx, None)
        .await
        .expect("list_enums fallback must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(v["enums"].is_array());
}

// ─── list_extensions cache path ───────────────────────────────────────────────

/// list_extensions returns results when the cache is warm.
#[tokio::test]
async fn test_list_extensions_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = list_extensions::handle(ctx, None)
        .await
        .expect("list_extensions must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let extensions = v["extensions"].as_array().unwrap();
    // plpgsql is always installed.
    assert!(
        extensions.iter().any(|e| e["name"] == "plpgsql"),
        "plpgsql extension must be present"
    );
}

/// list_extensions falls back to Postgres when the cache is empty.
#[tokio::test]
async fn test_list_extensions_fallback_on_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_empty_cache(&url);
    let result = list_extensions::handle(ctx, None)
        .await
        .expect("list_extensions fallback must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(
        v["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "plpgsql")
    );
}

// ─── table_stats cache path ───────────────────────────────────────────────────

/// table_stats serves from cache when the table is in the snapshot.
#[tokio::test]
async fn test_table_stats_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Create a table and insert some rows to ensure stats are non-trivial.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS public.cache_stats_tbl (id serial PRIMARY KEY, val text); \
                 INSERT INTO public.cache_stats_tbl (val) VALUES ('a'), ('b'), ('c') \
                 ON CONFLICT DO NOTHING;",
            )
            .await
            .unwrap();
        // Analyze so pg_stat_user_tables has data.
        client
            .execute("ANALYZE public.cache_stats_tbl", &[])
            .await
            .unwrap();
    }

    let warmed_stats: SchemaCache = SchemaCache::load_from_pool(&pool).await.unwrap();
    let cache = Arc::new(warmed_stats);
    let ctx = ToolContext::new(pool, cache, config);

    let args = serde_json::from_str::<serde_json::Map<_, _>>(
        r#"{"table":"cache_stats_tbl","schema":"public"}"#,
    )
    .unwrap();
    let result = table_stats::handle(ctx, Some(args))
        .await
        .expect("table_stats must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();

    assert_eq!(v["table"], "cache_stats_tbl");
    assert_eq!(v["schema"], "public");
    assert!(v["row_estimate"].is_number());
    assert!(v["sizes"].is_object());
    assert!(v["cache_hit_ratio"].is_number());
}

/// table_stats falls back to Postgres on cache miss.
#[tokio::test]
async fn test_table_stats_fallback_on_cache_miss() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Create a table to query.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS public.cache_fallback_tbl (id int PRIMARY KEY); \
                 ANALYZE public.cache_fallback_tbl;",
            )
            .await
            .unwrap();
    }

    // Use an empty cache — forces the live-query fallback.
    let ctx = test_ctx_with_empty_cache(&url);
    let args = serde_json::from_str::<serde_json::Map<_, _>>(
        r#"{"table":"cache_fallback_tbl","schema":"public"}"#,
    )
    .unwrap();
    let result = table_stats::handle(ctx, Some(args))
        .await
        .expect("table_stats fallback must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["table"], "cache_fallback_tbl");
}

// ─── health tool cache age ────────────────────────────────────────────────────

/// health response includes schema_cache_age_seconds as a number.
#[tokio::test]
async fn test_health_reports_cache_age() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = health::handle(ctx, None)
        .await
        .expect("health must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();

    assert!(
        v.get("schema_cache_age_seconds").is_some(),
        "health response must include schema_cache_age_seconds"
    );
    assert!(
        v["schema_cache_age_seconds"].is_number(),
        "schema_cache_age_seconds must be a number"
    );

    // A freshly warmed cache is recent: age should be < 60 seconds.
    let age = v["schema_cache_age_seconds"].as_f64().unwrap();
    assert!(
        age < 60.0,
        "freshly warmed cache age should be < 60s, got {age}"
    );
}

/// health response with an empty cache reports a very large cache age.
#[tokio::test]
async fn test_health_reports_large_age_for_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_empty_cache(&url);
    let result = health::handle(ctx, None)
        .await
        .expect("health must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();

    let age = v["schema_cache_age_seconds"].as_f64().unwrap();
    // Empty cache has captured_at = 0 → age = current epoch ≈ 1.7 billion.
    assert!(
        age > 1_000_000_000.0,
        "empty cache age must reflect epoch distance, got {age}"
    );
}
