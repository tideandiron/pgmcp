// tests/discovery.rs
//
// Integration tests for Phase 3 discovery tools.
//
// Each test acquires a fresh Postgres container, constructs a ToolContext,
// calls the tool handler directly (bypassing the MCP layer), and asserts on
// the JSON response.
//
// Run with: cargo test --test discovery

mod common;

use std::sync::Arc;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::pool::Pool,
    server::context::ToolContext,
    tools::{list_databases, list_schemas, list_tables, server_info},
};
use serde_json::Value;

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
        cache: CacheConfig::default(),
        guardrails: GuardrailConfig::default(),
    }
}

fn test_ctx(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    ToolContext::new(pool, config)
}

// ── server_info ───────────────────────────────────────────────────────────────

/// server_info returns all required top-level fields.
#[tokio::test]
async fn test_server_info_has_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &["version", "version_num", "settings", "role"] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}

/// server_info version_num is a positive integer >= 140000 (PG 14).
#[tokio::test]
async fn test_server_info_version_num_is_valid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let vnum = v["version_num"]
        .as_i64()
        .expect("version_num must be integer");
    assert!(
        vnum >= 140_000,
        "version_num {vnum} should be >= 140000 (PG 14)"
    );
    assert!(
        vnum < 250_000,
        "version_num {vnum} looks unrealistically large"
    );
}

/// server_info settings includes all 4 required keys.
#[tokio::test]
async fn test_server_info_settings_has_required_keys() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let settings = v["settings"].as_object().expect("settings must be object");
    for key in &[
        "statement_timeout",
        "max_connections",
        "work_mem",
        "shared_buffers",
    ] {
        assert!(settings.contains_key(*key), "settings missing key: {key}");
        assert!(
            settings[*key].is_string(),
            "settings.{key} must be a string, got: {:?}",
            settings.get(*key)
        );
    }
}

/// server_info role is a non-empty string.
#[tokio::test]
async fn test_server_info_role_is_non_empty() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let role = v["role"].as_str().expect("role must be string");
    assert!(!role.is_empty(), "role must not be empty");
}

/// server_info version string starts with "PostgreSQL".
#[tokio::test]
async fn test_server_info_version_string_is_postgres() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let version = v["version"].as_str().expect("version must be string");
    assert!(
        version.starts_with("PostgreSQL"),
        "version must start with 'PostgreSQL', got: {version}"
    );
}

/// server_info max_connections setting is present and parseable as a number.
#[tokio::test]
async fn test_server_info_max_connections_is_numeric_string() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = server_info::handle(test_ctx(&url), None)
        .await
        .expect("server_info must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let max_conn = v["settings"]["max_connections"]
        .as_str()
        .expect("max_connections must be a string");
    let parsed: u64 = max_conn
        .parse()
        .expect("max_connections must parse as a positive integer");
    assert!(parsed > 0, "max_connections must be > 0, got {parsed}");
}

// ── list_databases ────────────────────────────────────────────────────────────

/// list_databases returns an array under the "databases" key.
#[tokio::test]
async fn test_list_databases_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(
        v["databases"].is_array(),
        "result must have a 'databases' array"
    );
}

/// list_databases includes the test database.
#[tokio::test]
async fn test_list_databases_includes_pgmcp_test() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    let found = databases.iter().any(|db| db["name"] == "pgmcp_test");
    assert!(found, "pgmcp_test database should be in the list");
}

/// Every database entry has required string fields.
#[tokio::test]
async fn test_list_databases_entries_have_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    assert!(!databases.is_empty(), "should return at least one database");
    for db in databases {
        for field in &["name", "owner", "encoding"] {
            assert!(
                db.get(*field).is_some(),
                "missing field {field} in database entry"
            );
            assert!(db[*field].is_string(), "{field} must be a string");
        }
        // size_bytes is i64 | null
        if !db["size_bytes"].is_null() {
            assert!(
                db["size_bytes"].is_number(),
                "size_bytes must be number or null"
            );
        }
    }
}

/// pgmcp_test database has non-zero size.
#[tokio::test]
async fn test_list_databases_pgmcp_test_size_is_nonzero() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    let test_db = databases
        .iter()
        .find(|db| db["name"] == "pgmcp_test")
        .expect("pgmcp_test must be present");
    let size = test_db["size_bytes"]
        .as_i64()
        .expect("size_bytes must be an integer for pgmcp_test");
    assert!(size > 0, "pgmcp_test size must be > 0, got {size}");
}

/// list_databases handles template0 without erroring (size_bytes is null).
#[tokio::test]
async fn test_list_databases_template0_size_is_null() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed without error on template0");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    // template0 must be present and have null size_bytes (datallowconn = false)
    if let Some(template0) = databases.iter().find(|db| db["name"] == "template0") {
        assert!(
            template0["size_bytes"].is_null(),
            "template0 size_bytes must be null, got: {:?}",
            template0["size_bytes"]
        );
    }
    // Whether template0 appears depends on access — we assert no error occurred
}

// ── list_schemas ──────────────────────────────────────────────────────────────

/// list_schemas returns an array under the "schemas" key.
#[tokio::test]
async fn test_list_schemas_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v["schemas"].is_array(), "result must have 'schemas' array");
}

/// list_schemas includes the public schema.
#[tokio::test]
async fn test_list_schemas_includes_public() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    let found = schemas.iter().any(|s| s["name"] == "public");
    assert!(found, "public schema must be present");
}

/// list_schemas excludes internal Postgres schemas.
#[tokio::test]
async fn test_list_schemas_excludes_internal_schemas() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    let internal = ["pg_toast", "pg_catalog", "information_schema"];
    for s in schemas {
        let name = s["name"].as_str().unwrap();
        assert!(
            !internal.contains(&name),
            "internal schema '{name}' must not be in list_schemas output"
        );
        // Also check prefixes for pg_temp_* / pg_toast_temp_*
        assert!(
            !name.starts_with("pg_temp_"),
            "pg_temp_* schema '{name}' must not appear in list_schemas output"
        );
        assert!(
            !name.starts_with("pg_toast"),
            "pg_toast* schema '{name}' must not appear in list_schemas output"
        );
    }
}

/// Each schema entry has name and owner string fields.
#[tokio::test]
async fn test_list_schemas_entries_have_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    assert!(!schemas.is_empty(), "should return at least one schema");
    for s in schemas {
        assert!(s["name"].is_string(), "name must be a string");
        assert!(s["owner"].is_string(), "owner must be a string");
        // description is string | null
        assert!(
            s["description"].is_string() || s["description"].is_null(),
            "description must be string or null"
        );
    }
}

// ── list_tables ───────────────────────────────────────────────────────────────

/// Helper: create a table via a direct tokio-postgres connection.
async fn create_test_table(url: &str, ddl: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct connect for DDL");
    tokio::spawn(conn);
    client.execute(ddl, &[]).await.expect("DDL must succeed");
}

/// list_tables with kind=table returns the test table in public schema.
#[tokio::test]
async fn test_list_tables_returns_tables() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_test \
         (id serial PRIMARY KEY, val text)",
    )
    .await;

    let args = serde_json::from_str(r#"{"schema":"public","kind":"table"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    let found = tables.iter().any(|t| t["name"] == "phase3_lt_test");
    assert!(
        found,
        "phase3_lt_test must appear in list_tables(public, table)"
    );
}

/// list_tables entries have all required fields.
#[tokio::test]
async fn test_list_tables_entry_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_fields \
         (id serial PRIMARY KEY)",
    )
    .await;
    let args = serde_json::from_str(r#"{"schema":"public","kind":"table"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert!(!tables.is_empty(), "should have at least one table");
    let t = tables.first().unwrap();
    for field in &["schema", "name", "kind"] {
        assert!(t[field].is_string(), "field {field} must be a string");
    }
    // row_estimate is i64 or null
    assert!(
        t["row_estimate"].is_number() || t["row_estimate"].is_null(),
        "row_estimate must be number or null"
    );
    // description is string or null
    assert!(
        t["description"].is_string() || t["description"].is_null(),
        "description must be string or null"
    );
}

/// list_tables kind=view filter excludes regular tables.
#[tokio::test]
async fn test_list_tables_view_filter_excludes_tables() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_notview \
         (id serial PRIMARY KEY)",
    )
    .await;
    let args = serde_json::from_str(r#"{"schema":"public","kind":"view"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    let any_table = tables.iter().any(|t| t["kind"] == "table");
    assert!(!any_table, "kind=view filter must not return tables");
}

/// list_tables with no args returns param_invalid.
#[tokio::test]
async fn test_list_tables_missing_schema_is_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_tables::handle(test_ctx(&url), None).await;
    assert!(result.is_err(), "missing schema should return an error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        "param_invalid",
        "error code must be param_invalid"
    );
}

/// list_tables with an unknown schema returns an empty tables array, not an error.
#[tokio::test]
async fn test_list_tables_unknown_schema_returns_empty() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let args = serde_json::from_str(r#"{"schema":"totally_nonexistent_schema_xyz"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("unknown schema should return empty, not error");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert!(
        tables.is_empty(),
        "unknown schema must return empty tables array"
    );
}

/// list_tables kind=all returns both tables and views.
#[tokio::test]
async fn test_list_tables_kind_all_returns_tables_and_views() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_all_base \
         (id serial PRIMARY KEY, val text)",
    )
    .await;
    create_test_table(
        &url,
        "CREATE OR REPLACE VIEW public.phase3_lt_all_view \
         AS SELECT id, val FROM public.phase3_lt_all_base",
    )
    .await;

    let args = serde_json::from_str(r#"{"schema":"public","kind":"all"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables kind=all must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    let has_table = tables.iter().any(|t| t["kind"] == "table");
    let has_view = tables.iter().any(|t| t["kind"] == "view");
    assert!(has_table, "kind=all must include tables");
    assert!(has_view, "kind=all must include views");
}

/// list_tables kind=table returns correct schema and kind fields.
#[tokio::test]
async fn test_list_tables_schema_field_matches_requested() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_schema_check \
         (id serial PRIMARY KEY)",
    )
    .await;
    let args = serde_json::from_str(r#"{"schema":"public","kind":"table"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    for t in tables {
        assert_eq!(
            t["schema"], "public",
            "schema field must match the requested schema"
        );
        assert_eq!(
            t["kind"], "table",
            "kind field must be 'table' when kind=table requested"
        );
    }
}

/// list_tables with invalid kind parameter returns param_invalid.
#[tokio::test]
async fn test_list_tables_invalid_kind_is_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let args = serde_json::from_str(r#"{"schema":"public","kind":"bogus_kind"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args).await;
    assert!(result.is_err(), "invalid kind must return an error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        "param_invalid",
        "error code must be param_invalid for invalid kind"
    );
}
