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
    tools::{describe_table, list_databases, list_enums, list_schemas, list_tables, server_info},
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

// ── describe_table + list_enums — fixture setup ───────────────────────────────

/// Create the fixture schema, enum type, and tables used by describe_table
/// and list_enums tests.
///
/// Idempotent — uses IF NOT EXISTS / CREATE OR REPLACE wherever possible.
/// Must be called once per container; all tests share the same objects.
async fn create_describe_table_fixtures(url: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct connect for DDL");
    tokio::spawn(conn);

    // Create the enum type used by list_enums tests.
    client
        .execute(
            "DO $$ BEGIN \
               IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'mood') THEN \
                   CREATE TYPE public.mood AS ENUM ('happy', 'neutral', 'sad'); \
               END IF; \
             END $$",
            &[],
        )
        .await
        .expect("create enum mood");

    // Parent table: has PK, a check constraint, a column comment, and a
    // secondary unique index so every branch of describe_table is exercised.
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS public.dt_parent ( \
               id      serial PRIMARY KEY, \
               name    text NOT NULL, \
               score   int  NOT NULL DEFAULT 0, \
               status  public.mood NOT NULL DEFAULT 'neutral', \
               CONSTRAINT dt_parent_score_check CHECK (score >= 0) \
             )",
            &[],
        )
        .await
        .expect("create dt_parent");

    // Add a secondary index on `name`.
    client
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS dt_parent_name_idx \
             ON public.dt_parent (name)",
            &[],
        )
        .await
        .expect("create dt_parent_name_idx");

    // Attach a comment to the `name` column.
    client
        .execute(
            "COMMENT ON COLUMN public.dt_parent.name IS 'The display name'",
            &[],
        )
        .await
        .expect("comment on column");

    // Child table: has a FK back to dt_parent so foreign_key constraint appears.
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS public.dt_child ( \
               id        serial PRIMARY KEY, \
               parent_id int NOT NULL REFERENCES public.dt_parent(id) \
             )",
            &[],
        )
        .await
        .expect("create dt_child");
}

// ── describe_table tests ──────────────────────────────────────────────────────

/// describe_table returns the top-level structure with required keys.
#[tokio::test]
async fn test_describe_table_returns_required_top_level_keys() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed for dt_parent");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();

    for key in &["table", "columns", "constraints", "indexes"] {
        assert!(v.get(*key).is_some(), "missing top-level key: {key}");
    }
}

/// describe_table table sub-object has name, schema, and description fields.
#[tokio::test]
async fn test_describe_table_table_object_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let tbl = &v["table"];

    assert_eq!(tbl["name"], "dt_parent", "table.name must be 'dt_parent'");
    assert_eq!(tbl["schema"], "public", "table.schema must be 'public'");
    // description is null (no COMMENT ON TABLE set on dt_parent).
    assert!(
        tbl["description"].is_null() || tbl["description"].is_string(),
        "table.description must be string or null"
    );
}

/// describe_table returns columns with correct types.
#[tokio::test]
async fn test_describe_table_columns_correct_types() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let columns = v["columns"].as_array().expect("columns must be an array");

    // Must have at least id, name, score, status.
    assert!(columns.len() >= 4, "dt_parent must have at least 4 columns");

    // Every column entry has the required fields.
    for col in columns {
        assert!(col["name"].is_string(), "column.name must be string");
        assert!(col["type"].is_string(), "column.type must be string");
        assert!(col["nullable"].is_boolean(), "column.nullable must be bool");
        assert!(
            col["default"].is_string() || col["default"].is_null(),
            "column.default must be string or null"
        );
        assert!(
            col["description"].is_string() || col["description"].is_null(),
            "column.description must be string or null"
        );
    }

    // `id` column should be integer type.
    let id_col = columns
        .iter()
        .find(|c| c["name"] == "id")
        .expect("id column");
    let id_type = id_col["type"].as_str().unwrap();
    assert!(
        id_type.contains("integer") || id_type.contains("int"),
        "id column type should contain 'integer', got: {id_type}"
    );

    // `name` column should be text and NOT NULL.
    let name_col = columns
        .iter()
        .find(|c| c["name"] == "name")
        .expect("name column");
    assert_eq!(
        name_col["type"], "text",
        "name column must have type 'text'"
    );
    assert_eq!(name_col["nullable"], false, "name column must be NOT NULL");
}

/// describe_table columns include column comments.
#[tokio::test]
async fn test_describe_table_column_description() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let columns = v["columns"].as_array().unwrap();

    let name_col = columns
        .iter()
        .find(|c| c["name"] == "name")
        .expect("name column must exist");
    let desc = name_col["description"]
        .as_str()
        .expect("name column must have a non-null description");
    assert_eq!(
        desc, "The display name",
        "column description must match the COMMENT set on it"
    );
}

/// describe_table returns the primary key constraint.
#[tokio::test]
async fn test_describe_table_primary_key_constraint() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let constraints = v["constraints"]
        .as_array()
        .expect("constraints must be an array");

    let pk = constraints
        .iter()
        .find(|c| c["type"] == "primary_key")
        .expect("dt_parent must have a primary_key constraint");

    assert!(pk["name"].is_string(), "pk.name must be string");
    assert!(pk["definition"].is_string(), "pk.definition must be string");

    // PK columns must include 'id'.
    let pk_cols = pk["columns"]
        .as_array()
        .expect("pk.columns must be an array");
    let has_id = pk_cols.iter().any(|c| c == "id");
    assert!(has_id, "primary key columns must include 'id'");
}

/// describe_table returns the check constraint on score.
#[tokio::test]
async fn test_describe_table_check_constraint() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let constraints = v["constraints"].as_array().unwrap();

    let check = constraints
        .iter()
        .find(|c| c["type"] == "check")
        .expect("dt_parent must have a check constraint");

    let def = check["definition"].as_str().unwrap();
    assert!(
        def.contains("score") || def.to_lowercase().contains("check"),
        "check constraint definition must reference 'score': got '{def}'"
    );
}

/// describe_table returns foreign key constraint on dt_child.
#[tokio::test]
async fn test_describe_table_foreign_key_constraint() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_child","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed for dt_child");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let constraints = v["constraints"].as_array().unwrap();

    let fk = constraints
        .iter()
        .find(|c| c["type"] == "foreign_key")
        .expect("dt_child must have a foreign_key constraint");

    let def = fk["definition"].as_str().unwrap();
    assert!(
        def.to_lowercase().contains("references") || def.to_lowercase().contains("foreign"),
        "FK definition must contain 'REFERENCES': got '{def}'"
    );
    let fk_cols = fk["columns"].as_array().expect("fk.columns must be array");
    let has_parent_id = fk_cols.iter().any(|c| c == "parent_id");
    assert!(has_parent_id, "FK columns must include 'parent_id'");
}

/// describe_table returns indexes including primary and secondary.
#[tokio::test]
async fn test_describe_table_indexes() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let indexes = v["indexes"].as_array().expect("indexes must be an array");

    // Must have at least two indexes: PK index and dt_parent_name_idx.
    assert!(
        indexes.len() >= 2,
        "dt_parent must have at least 2 indexes, got {}",
        indexes.len()
    );

    // Each index entry has required fields.
    for idx in indexes {
        assert!(idx["name"].is_string(), "index.name must be string");
        assert!(idx["type"].is_string(), "index.type must be string");
        assert!(
            idx["is_unique"].is_boolean(),
            "index.is_unique must be bool"
        );
        assert!(
            idx["is_primary"].is_boolean(),
            "index.is_primary must be bool"
        );
        assert!(
            idx["definition"].is_string(),
            "index.definition must be string"
        );
        assert!(
            idx["size_bytes"].is_number(),
            "index.size_bytes must be number"
        );
    }

    // The primary index must be is_primary=true and is_unique=true.
    let pk_idx = indexes
        .iter()
        .find(|i| i["is_primary"] == true)
        .expect("must have a primary index");
    assert_eq!(
        pk_idx["is_unique"], true,
        "primary index must also be unique"
    );

    // The secondary index on name must be present.
    let has_name_idx = indexes.iter().any(|i| {
        i["name"]
            .as_str()
            .map(|n| n.contains("name_idx"))
            .unwrap_or(false)
    });
    assert!(has_name_idx, "dt_parent_name_idx must appear in indexes");
}

/// describe_table with no `table` parameter returns param_invalid.
#[tokio::test]
async fn test_describe_table_missing_table_param() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let result = describe_table::handle(test_ctx(&url), None).await;
    assert!(result.is_err(), "missing table must return an error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        "param_invalid",
        "error code must be param_invalid"
    );
}

/// describe_table for a nonexistent table returns table_not_found.
#[tokio::test]
async fn test_describe_table_nonexistent_table() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let args =
        serde_json::from_str(r#"{"table":"this_table_does_not_exist_xyz","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args).await;
    assert!(result.is_err(), "nonexistent table must return an error");
    let err = result.unwrap_err();
    assert_eq!(
        err.code(),
        "table_not_found",
        "error code must be table_not_found, got: {}",
        err.code()
    );
}

/// describe_table schema defaults to "public" when omitted.
#[tokio::test]
async fn test_describe_table_schema_defaults_to_public() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    // Omit schema — should default to "public" and find dt_parent.
    let args = serde_json::from_str(r#"{"table":"dt_parent"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed when schema is omitted");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["table"]["schema"], "public",
        "schema must default to 'public'"
    );
    assert_eq!(v["table"]["name"], "dt_parent");
}

/// describe_table column default values are returned.
#[tokio::test]
async fn test_describe_table_column_defaults() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(r#"{"table":"dt_parent","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let columns = v["columns"].as_array().unwrap();

    // `score` has DEFAULT 0.
    let score_col = columns
        .iter()
        .find(|c| c["name"] == "score")
        .expect("score column must exist");
    let default_val = score_col["default"]
        .as_str()
        .expect("score must have a default");
    assert!(
        default_val.contains('0'),
        "score default must include '0', got: {default_val}"
    );
}

/// describe_table returns a table-level check constraint with a non-null
/// columns array rather than silently dropping the constraint.
///
/// A table-level CHECK like `CHECK (start_date < end_date)` is syntactically
/// defined at the table level (not on a specific column). In Postgres, when
/// such a constraint references columns, `conkey` is populated with those
/// column numbers; when it references no columns at all, `conkey` is NULL.
///
/// The original INNER JOIN on `pg_attribute` used `attnum = ANY(conkey)` which
/// evaluates to NULL when `conkey` IS NULL, causing the constraint row to be
/// silently dropped. After switching to LEFT JOIN + FILTER the constraint must
/// appear in the output regardless of whether `conkey` is NULL.
///
/// This test also exercises the non-NULL-conkey path (Postgres does populate
/// `conkey` for checks that reference specific columns), verifying both that
/// the constraint appears and that the `columns` field is a JSON array.
#[tokio::test]
async fn test_describe_table_table_level_check_constraint() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    // Create the fixture table in this container (idempotent).
    {
        use tokio_postgres::NoTls;
        let (client, conn) = tokio_postgres::connect(&url, NoTls)
            .await
            .expect("direct connect for DDL");
        tokio::spawn(conn);
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS public.dt_check_test ( \
                   start_date date, \
                   end_date   date, \
                   CONSTRAINT valid_range CHECK (start_date < end_date) \
                 )",
                &[],
            )
            .await
            .expect("create dt_check_test");
    }

    let args =
        serde_json::from_str(r#"{"table":"dt_check_test","schema":"public"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed for dt_check_test");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();

    let constraints = v["constraints"]
        .as_array()
        .expect("constraints must be an array");

    // The constraint must appear — it was silently dropped before the bug fix.
    let valid_range = constraints
        .iter()
        .find(|c| c["name"] == "valid_range")
        .expect("valid_range constraint must appear in describe_table output");

    assert_eq!(
        valid_range["type"], "check",
        "valid_range must be type 'check'"
    );

    // The columns field must be a JSON array (never null), even for constraints
    // whose conkey is NULL. build_constraint normalises Option<Vec<_>> to Vec<_>.
    assert!(
        valid_range["columns"].is_array(),
        "valid_range.columns must be a JSON array, got: {:?}",
        valid_range["columns"]
    );

    // Definition must reference the condition expression.
    let def = valid_range["definition"]
        .as_str()
        .expect("valid_range.definition must be a string");
    assert!(
        def.contains("start_date") && def.contains("end_date"),
        "definition must reference both columns: got '{def}'"
    );
}

// ── list_enums tests ──────────────────────────────────────────────────────────

/// list_enums returns an "enums" array.
#[tokio::test]
async fn test_list_enums_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let result = list_enums::handle(test_ctx(&url), None)
        .await
        .expect("list_enums must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v["enums"].is_array(), "result must have an 'enums' array");
}

/// list_enums returns the mood enum with correct labels in order.
#[tokio::test]
async fn test_list_enums_mood_enum_values() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let result = list_enums::handle(test_ctx(&url), None)
        .await
        .expect("list_enums must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let enums = v["enums"].as_array().unwrap();

    let mood = enums
        .iter()
        .find(|e| e["name"] == "mood")
        .expect("mood enum must be present");

    assert_eq!(mood["schema"], "public", "mood must be in public schema");

    let values = mood["values"]
        .as_array()
        .expect("mood.values must be array");
    assert_eq!(values.len(), 3, "mood must have exactly 3 labels");
    assert_eq!(values[0], "happy", "first label must be 'happy'");
    assert_eq!(values[1], "neutral", "second label must be 'neutral'");
    assert_eq!(values[2], "sad", "third label must be 'sad'");
}

/// Every enum entry has the required fields.
#[tokio::test]
async fn test_list_enums_entries_have_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let result = list_enums::handle(test_ctx(&url), None)
        .await
        .expect("list_enums must succeed");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let enums = v["enums"].as_array().unwrap();
    assert!(!enums.is_empty(), "must have at least one enum");

    for e in enums {
        assert!(e["name"].is_string(), "enum.name must be string");
        assert!(e["schema"].is_string(), "enum.schema must be string");
        assert!(e["values"].is_array(), "enum.values must be array");
        let vals = e["values"].as_array().unwrap();
        assert!(!vals.is_empty(), "enum.values must not be empty");
        for val in vals {
            assert!(val.is_string(), "each enum value must be a string");
        }
    }
}

/// list_enums on a fresh DB without any user enums returns an empty array.
#[tokio::test]
async fn test_list_enums_empty_when_no_enums() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    // Do NOT call create_describe_table_fixtures — fresh container, no enums.
    let result = list_enums::handle(test_ctx(&url), None)
        .await
        .expect("list_enums must succeed on a fresh database");
    let text = result.content[0].as_text().unwrap().text.clone();
    let v: Value = serde_json::from_str(&text).unwrap();
    let enums = v["enums"].as_array().unwrap();
    // A fresh postgres:16-alpine container has no user enums.
    assert!(
        enums.is_empty(),
        "fresh database must have no user enums, got: {enums:?}"
    );
}
