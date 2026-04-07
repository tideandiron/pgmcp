# pgmcp Phase 3: Discovery Tools — Implementation Plan

**Branches:** feat/008 through feat/012  
**Phase:** 3 of 9  
**Date:** 2026-04-07  
**Depends on:** Phase 2 complete (feat/001 – feat/007 merged)

---

## Phase 3 Overview

Phase 3 turns the 8 discovery tool stubs into real implementations backed by `pg_catalog` queries. Every tool in this phase executes actual SQL — no mocking, no hardcoded responses.

By the end of this phase:

- All 8 discovery tools return real data from a live Postgres 16 instance.
- `tests/discovery.rs` contains at least 30 integration tests covering happy paths, edge cases, and error paths.
- `tests/health.rs` is extended with tool-level integration tests for `health` and `connection_info`.
- 5 SQL files (`server_settings.sql`, `list_databases.sql`, `list_schemas.sql`, `list_tables.sql`, `describe_table.sql`, `list_enums.sql`, `list_extensions.sql`, `table_stats.sql`) contain the production queries.

### Key constraints for all Phase 3 handlers

1. Handler signature is `pub(crate) async fn handle(ctx: ToolContext, args: Option<Map<String, Value>>) -> Result<CallToolResult, McpError>`.
2. Acquire a connection via `ctx.pool.get(Duration::from_secs(ctx.config.pool.acquire_timeout_seconds))`.
3. Release the connection immediately when the query is done — do not hold it across any await point you don't own.
4. Map `tokio_postgres::Error` to `McpError` via `McpError::from(e)` or `.map_err(McpError::from)`.
5. Return `CallToolResult::success(vec![Content::text(json_string)])` on success.
6. Return `Err(McpError::param_invalid(...))` for missing/invalid parameters.
7. Return `Err(McpError::schema_not_found(...))` or `Err(McpError::table_not_found(...))` when the named object does not exist.
8. Do not call `unwrap()` or `expect()` outside of `#[cfg(test)]` blocks.

### Test infrastructure reuse

All integration tests use the fixture from `tests/common/fixtures.rs`:

```rust
let Some((_container, url)) = common::fixtures::pg_container().await else {
    eprintln!("SKIP: Docker not available");
    return;
};
```

The `_container` binding must stay alive for the full test to keep the Docker container running. The helper creates a `postgres:16-alpine` container with:
- User: `pgmcp_test`
- Password: `pgmcp_test`
- Database: `pgmcp_test`

### Config builder used in tests

```rust
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
```

### ToolContext builder for tests

```rust
fn test_ctx(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool"));
    ToolContext::new(pool, config)
}
```

---

## feat/008 — health + connection_info

**Branch:** `feat/008-health-connection-info`  
**Depends on:** feat/007 merged  
**Files modified:**
- `src/tools/health.rs`
- `src/tools/connection_info.rs`
- `src/pg/queries/server_settings.sql`
- `tests/health.rs`

### Step 8.1 — Write `server_settings.sql`

This SQL file is used by `connection_info` to retrieve session-level server information.

**File: `src/pg/queries/server_settings.sql`**

```sql
-- src/pg/queries/server_settings.sql
--
-- Returns connection metadata for the current session.
-- Used by the connection_info tool.
--
-- Columns returned (in order):
--   current_user  TEXT    — the role name connected to Postgres
--   current_db    TEXT    — name of the current database
--   server_host   TEXT    — host from pg_postmaster_start_time (falls back to 'localhost')
--   server_port   INT4    — port from current_setting, or 5432 as fallback
--   server_version TEXT   — full version string from version()
--   ssl_active    BOOL    — whether the current connection uses SSL (always false for local socket)
--
-- Notes:
--   pg_stat_ssl.ssl is only populated when pg_stat_ssl is available (requires
--   pg_stat_ssl to be enabled, which is the default since PG 9.2). If the view
--   is not populated for this backend, ssl_active returns false.

SELECT
    current_user                                           AS current_user,
    current_database()                                     AS current_db,
    COALESCE(
        current_setting('listen_addresses', true),
        'localhost'
    )                                                      AS server_host,
    COALESCE(
        current_setting('port', true)::int4,
        5432
    )                                                      AS server_port,
    version()                                              AS server_version,
    COALESCE(
        (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
        false
    )                                                      AS ssl_active
```

**Verification:** Run this in `psql` against PG 16 and confirm it returns one row with five columns. The `listen_addresses` setting may be `*` or a comma-separated list — that is fine, report it verbatim.

### Step 8.2 — Implement `tools/health.rs`

**TDD — write the tests first.**

Add to `tests/health.rs` (below the existing pool tests):

```rust
// ── health tool integration tests ────────────────────────────────────────────

use pgmcp::{
    server::{context::ToolContext, router::dispatch},
    tools::health,
};
use rmcp::model::CallToolRequestParams;
use serde_json::Value;
use std::sync::Arc;
use pgmcp::config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig};
use pgmcp::pg::pool::Pool;

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

/// health tool returns status "ok" against a live Postgres instance.
#[tokio::test]
async fn test_health_returns_ok() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = health::handle(test_ctx(&url), None)
        .await
        .expect("health must not error");
    let text = result.content[0].as_text().expect("content must be text");
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

/// latency_ms is measured end-to-end and is reasonably accurate.
/// We accept up to 2000ms on slow CI machines.
#[tokio::test]
async fn test_health_latency_is_non_negative_and_sane() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = health::handle(test_ctx(&url), None)
        .await
        .expect("health handle must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let latency = v["latency_ms"].as_f64().unwrap();
    assert!(latency >= 0.0, "latency must be >= 0ms, got {latency}");
    assert!(latency < 2000.0, "latency {latency}ms is unreasonably large");
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &["status", "pg_reachable", "pool_available", "latency_ms", "pool_stats"] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}
```

**Run (expect 3 compile errors because `health::handle` is still a stub):**
```
cargo test --test health test_health 2>&1 | head -30
```

**Now implement `src/tools/health.rs`:**

```rust
// src/tools/health.rs
//
// health tool — executes SELECT 1 and reports pool + connectivity status.
//
// Returns a JSON object:
//   {
//     "status":         "ok" | "degraded" | "unhealthy",
//     "pg_reachable":   bool,
//     "pool_available": bool,
//     "latency_ms":     f64,
//     "pool_stats": {
//       "size":      usize,
//       "available": usize,
//       "waiting":   usize,
//     }
//   }
//
// Latency is measured end-to-end: start → pool acquire → SELECT 1 → row received.

use std::time::{Duration, Instant};

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let start = Instant::now();
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);

    // Attempt pool acquire + SELECT 1.  Capture both the pool stats and the
    // reachability result regardless of whether the query succeeds.
    let pool_status = ctx.pool.inner().status();

    let pg_reachable = match ctx.pool.get(timeout).await {
        Ok(client) => {
            client
                .query_one("SELECT 1::int4", &[])
                .await
                .map_err(McpError::from)?;
            true
        }
        Err(_) => false,
    };

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    let pool_available = pg_reachable;
    let status = if pg_reachable { "ok" } else { "unhealthy" };

    let body = serde_json::json!({
        "status":         status,
        "pg_reachable":   pg_reachable,
        "pool_available": pool_available,
        "latency_ms":     (latency_ms * 10.0).round() / 10.0,
        "pool_stats": {
            "size":      pool_status.size,
            "available": pool_status.available,
            "waiting":   pool_status.waiting,
        }
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Note on `deadpool_postgres::Status`:** `pool.inner().status()` returns a `deadpool::Status` with fields `size: usize`, `available: usize`, and `waiting: usize`. These are the current snapshot values and are available without an async call.

**Run tests:**
```
cargo test --test health test_health
```

Expected output:
```
test test_health_returns_ok ... ok
test test_health_latency_is_non_negative_and_sane ... ok
test test_health_response_has_all_fields ... ok
```

**Clippy:**
```
cargo clippy -- -D warnings 2>&1 | grep "src/tools/health"
```
Expected: no output (zero warnings).

### Step 8.3 — Implement `tools/connection_info.rs`

**TDD — tests first.** Add to `tests/health.rs`:

```rust
use pgmcp::tools::connection_info;

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
    let text = result.content[0].as_text().expect("text content");
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &["host", "port", "database", "role", "ssl", "server_version", "pool"] {
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
    let text = result.content[0].as_text().unwrap();
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
    let text = result.content[0].as_text().unwrap();
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let pool = &v["pool"];
    assert!(pool["size"].is_number(), "pool.size must be a number");
    assert!(pool["available"].is_number(), "pool.available must be a number");
    assert!(pool["waiting"].is_number(), "pool.waiting must be a number");
}

/// server_version is a non-empty string starting with "PostgreSQL".
#[tokio::test]
async fn test_connection_info_server_version_is_postgres() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = connection_info::handle(test_ctx(&url), None)
        .await
        .expect("connection_info must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let version = v["server_version"].as_str().unwrap();
    assert!(
        version.starts_with("PostgreSQL"),
        "server_version should start with 'PostgreSQL', got: {version}"
    );
}
```

**Implement `src/tools/connection_info.rs`:**

```rust
// src/tools/connection_info.rs
//
// connection_info tool — reports metadata about the current Postgres connection.
//
// Returns a JSON object:
//   {
//     "host":           string,
//     "port":           number,
//     "database":       string,
//     "role":           string,
//     "ssl":            bool,
//     "server_version": string,
//     "pool": {
//       "size":      usize,
//       "available": usize,
//       "waiting":   usize,
//     }
//   }
//
// SQL is read inline (not from a .sql file) because the query is short and
// directly mirrors the returned struct. The server_settings.sql file is the
// canonical reference for the query logic.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Query matches src/pg/queries/server_settings.sql
    let row = client
        .query_one(
            "SELECT \
                current_user, \
                current_database(), \
                COALESCE(current_setting('listen_addresses', true), 'localhost'), \
                COALESCE(current_setting('port', true)::int4, 5432), \
                version(), \
                COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false)",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let role: String = row.get(0);
    let database: String = row.get(1);
    let host: String = row.get(2);
    let port: i32 = row.get(3);
    let server_version: String = row.get(4);
    let ssl: bool = row.get(5);

    // Drop the client before accessing pool stats to avoid deadlock
    // (the client holds a pool slot; releasing it first gives accurate available count).
    drop(client);

    let pool_status = ctx.pool.inner().status();

    let body = serde_json::json!({
        "host":           host,
        "port":           port,
        "database":       database,
        "role":           role,
        "ssl":            ssl,
        "server_version": server_version,
        "pool": {
            "size":      pool_status.size,
            "available": pool_status.available,
            "waiting":   pool_status.waiting,
        }
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Run:**
```
cargo test --test health
```

Expected:
```
test test_health_returns_ok ... ok
test test_health_latency_is_non_negative_and_sane ... ok
test test_health_response_has_all_fields ... ok
test test_connection_info_has_all_fields ... ok
test test_connection_info_database_is_pgmcp_test ... ok
test test_connection_info_role_is_pgmcp_test ... ok
test test_connection_info_pool_stats_are_numbers ... ok
test test_connection_info_server_version_is_postgres ... ok
```

**Full check:**
```
cargo clippy -- -D warnings && cargo fmt --check && cargo test --test health
```

### feat/008 Acceptance Checklist

- [ ] `health` returns `{"status":"ok", ...}` with all 5 required fields against a live PG instance
- [ ] `health` latency_ms is non-negative and < 2000
- [ ] `health` pool_stats includes `size`, `available`, `waiting`
- [ ] `connection_info` returns all 7 required fields
- [ ] `connection_info` database matches container's `pgmcp_test`
- [ ] `connection_info` role matches container's `pgmcp_test`
- [ ] `server_settings.sql` contains the production query
- [ ] Zero `cargo clippy -- -D warnings` warnings in modified files
- [ ] All 8 integration tests pass

---

## feat/009 — server_info + list_databases

**Branch:** `feat/009-server-info-list-databases`  
**Depends on:** feat/008 merged  
**Files modified:**
- `src/tools/server_info.rs`
- `src/tools/list_databases.rs`
- `src/pg/queries/server_settings.sql` (extend with settings query)
- `src/pg/queries/list_databases.sql`
- `tests/discovery.rs`

### Step 9.1 — Write `list_databases.sql`

**File: `src/pg/queries/list_databases.sql`**

```sql
-- src/pg/queries/list_databases.sql
--
-- Returns all databases visible to the connected role on this Postgres instance.
-- Used by the list_databases tool.
--
-- Columns returned (in order):
--   name         TEXT    — database name
--   owner        TEXT    — name of the owning role
--   encoding     TEXT    — character encoding name (e.g. 'UTF8')
--   size_bytes   INT8    — size in bytes, NULL if pg_database_size() would error
--   description  TEXT    — comment on the database, NULL if none
--
-- Notes:
--   pg_database_size() requires CONNECT privilege on the target database.
--   For databases the role cannot connect to (e.g., template0 with datallowconn=false),
--   we use a CASE expression to avoid an error and return NULL instead.
--   template0 is included but its size is NULL because datallowconn = false.
--
--   pg_shdescription holds per-database (shared-catalog) comments.
--   The class OID for pg_database is 1262.

SELECT
    d.datname                                           AS name,
    r.rolname                                           AS owner,
    pg_encoding_to_char(d.encoding)                     AS encoding,
    CASE
        WHEN d.datallowconn THEN pg_database_size(d.oid)
        ELSE NULL
    END                                                 AS size_bytes,
    sd.description                                      AS description
FROM pg_database d
JOIN pg_roles r ON r.oid = d.datdba
LEFT JOIN pg_shdescription sd
    ON sd.objoid = d.oid
    AND sd.classoid = 'pg_database'::regclass
ORDER BY d.datname
```

### Step 9.2 — Extend `server_settings.sql` with a settings query

Append a second logical block to `server_settings.sql`. The `server_info` tool needs both the connection info (used by feat/008's `connection_info` tool) and the key settings. Both queries are short enough to inline in the Rust handler, but the `.sql` file documents the canonical SQL.

**File: `src/pg/queries/server_settings.sql`** — extend with:

```sql
-- ── server_info settings query ────────────────────────────────────────────────
--
-- Returns key Postgres server settings and version information.
-- Used by the server_info tool.
--
-- Columns returned (in order):
--   version_string  TEXT   — output of version(), e.g. "PostgreSQL 16.2 on x86_64-..."
--   version_num     INT4   — server_version_num as integer, e.g. 160002
--   current_role    TEXT   — current_user
--   statement_timeout TEXT — GUC value for statement_timeout (ms as string)
--   max_connections TEXT   — GUC value for max_connections
--   work_mem        TEXT   — GUC value for work_mem
--   shared_buffers  TEXT   — GUC value for shared_buffers
--
-- All settings are returned as TEXT strings exactly as Postgres stores them
-- (e.g., "5000" for 5000ms, "128MB" for 128 megabytes). The tool does not
-- reformat these values.

SELECT
    version()                                               AS version_string,
    current_setting('server_version_num')::int4             AS version_num,
    current_user                                            AS current_role,
    current_setting('statement_timeout')                    AS statement_timeout,
    current_setting('max_connections')                      AS max_connections,
    current_setting('work_mem')                             AS work_mem,
    current_setting('shared_buffers')                       AS shared_buffers
```

### Step 9.3 — Write tests first

Add to `tests/discovery.rs`:

```rust
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
use serde_json::Value;
use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::pool::Pool,
    server::context::ToolContext,
    tools::{server_info, list_databases},
};

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
    let text = result.content[0].as_text().unwrap();
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let vnum = v["version_num"].as_i64().expect("version_num must be integer");
    assert!(vnum >= 140_000, "version_num {vnum} should be >= 140000 (PG 14)");
    assert!(vnum < 250_000, "version_num {vnum} looks unrealistically large");
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let settings = v["settings"].as_object().expect("settings must be object");
    for key in &["statement_timeout", "max_connections", "work_mem", "shared_buffers"] {
        assert!(settings.contains_key(*key), "settings missing key: {key}");
        assert!(
            settings[*key].is_string(),
            "settings.{key} must be a string, got: {:?}", settings.get(*key)
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let role = v["role"].as_str().expect("role must be string");
    assert!(!role.is_empty(), "role must not be empty");
}

// ── list_databases ────────────────────────────────────────────────────────────

/// list_databases returns an array.
#[tokio::test]
async fn test_list_databases_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(v["databases"].is_array(), "result must have a 'databases' array");
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    let found = databases.iter().any(|db| db["name"] == "pgmcp_test");
    assert!(found, "pgmcp_test database should be in the list");
}

/// Every database entry has required fields.
#[tokio::test]
async fn test_list_databases_entries_have_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_databases::handle(test_ctx(&url), None)
        .await
        .expect("list_databases must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    assert!(!databases.is_empty(), "should return at least one database");
    for db in databases {
        for field in &["name", "owner", "encoding"] {
            assert!(db.get(*field).is_some(), "missing field {field} in database entry");
            assert!(db[*field].is_string(), "{field} must be a string");
        }
        // size_bytes is i64 | null
        if !db["size_bytes"].is_null() {
            assert!(db["size_bytes"].is_number(), "size_bytes must be number or null");
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let databases = v["databases"].as_array().unwrap();
    let test_db = databases
        .iter()
        .find(|db| db["name"] == "pgmcp_test")
        .expect("pgmcp_test must be present");
    let size = test_db["size_bytes"].as_i64().expect("size_bytes must be an integer for pgmcp_test");
    assert!(size > 0, "pgmcp_test size must be > 0, got {size}");
}
```

**Run (expect failures — stubs not yet replaced):**
```
cargo test --test discovery 2>&1 | tail -20
```

### Step 9.4 — Implement `tools/server_info.rs`

```rust
// src/tools/server_info.rs
//
// server_info tool — returns Postgres server version, key settings, connected role.
//
// Returns a JSON object:
//   {
//     "version":     string,   -- version() output
//     "version_num": number,   -- server_version_num integer
//     "settings": {
//       "statement_timeout": string,
//       "max_connections":   string,
//       "work_mem":          string,
//       "shared_buffers":    string
//     },
//     "role": string           -- current_user
//   }
//
// All setting values are returned as-is from current_setting() — Postgres
// stores them as strings (e.g., "5000" for 5000ms, "128MB" for 128MB).

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Single-row query returning version and 4 key settings.
    // Matches the server_info block in src/pg/queries/server_settings.sql.
    let row = client
        .query_one(
            "SELECT \
                version(), \
                current_setting('server_version_num')::int4, \
                current_user, \
                current_setting('statement_timeout'), \
                current_setting('max_connections'), \
                current_setting('work_mem'), \
                current_setting('shared_buffers')",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let version_string: String = row.get(0);
    let version_num: i32 = row.get(1);
    let role: String = row.get(2);
    let statement_timeout: String = row.get(3);
    let max_connections: String = row.get(4);
    let work_mem: String = row.get(5);
    let shared_buffers: String = row.get(6);

    let body = serde_json::json!({
        "version":     version_string,
        "version_num": version_num,
        "settings": {
            "statement_timeout": statement_timeout,
            "max_connections":   max_connections,
            "work_mem":          work_mem,
            "shared_buffers":    shared_buffers,
        },
        "role": role,
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

### Step 9.5 — Implement `tools/list_databases.rs`

```rust
// src/tools/list_databases.rs
//
// list_databases tool — returns all databases on this Postgres instance.
//
// Returns a JSON object:
//   {
//     "databases": [
//       {
//         "name":        string,
//         "owner":       string,
//         "encoding":    string,
//         "size_bytes":  number | null,
//         "description": string | null
//       },
//       ...
//     ]
//   }
//
// The query in list_databases.sql returns NULL for size_bytes when
// datallowconn = false (e.g., template0).

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/list_databases.sql
    let rows = client
        .query(
            "SELECT \
                d.datname, \
                r.rolname, \
                pg_encoding_to_char(d.encoding), \
                CASE WHEN d.datallowconn THEN pg_database_size(d.oid) ELSE NULL END, \
                sd.description \
            FROM pg_database d \
            JOIN pg_roles r ON r.oid = d.datdba \
            LEFT JOIN pg_shdescription sd \
                ON sd.objoid = d.oid \
                AND sd.classoid = 'pg_database'::regclass \
            ORDER BY d.datname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let databases: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let owner: String = row.get(1);
            let encoding: String = row.get(2);
            let size_bytes: Option<i64> = row.get(3);
            let description: Option<String> = row.get(4);
            serde_json::json!({
                "name":        name,
                "owner":       owner,
                "encoding":    encoding,
                "size_bytes":  size_bytes,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "databases": databases });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Run:**
```
cargo test --test discovery
```

Expected:
```
test test_server_info_has_required_fields ... ok
test test_server_info_version_num_is_valid ... ok
test test_server_info_settings_has_required_keys ... ok
test test_server_info_role_is_non_empty ... ok
test test_list_databases_returns_array ... ok
test test_list_databases_includes_pgmcp_test ... ok
test test_list_databases_entries_have_required_fields ... ok
test test_list_databases_pgmcp_test_size_is_nonzero ... ok
```

**Full check:**
```
cargo clippy -- -D warnings && cargo fmt --check && cargo test --test discovery && cargo test --test health
```

### feat/009 Acceptance Checklist

- [ ] `server_info` returns `version`, `version_num`, `settings`, `role`
- [ ] `version_num` is integer >= 140000
- [ ] `settings` has all 4 required keys, all string values
- [ ] `list_databases` returns `{"databases": [...]}`
- [ ] `list_databases` includes `pgmcp_test` with non-zero `size_bytes`
- [ ] `list_databases.sql` contains the production query
- [ ] `server_settings.sql` is extended with the server_info query block
- [ ] All 8 new integration tests pass

---

## feat/010 — list_schemas + list_tables

**Branch:** `feat/010-list-schemas-list-tables`  
**Depends on:** feat/009 merged  
**Files modified:**
- `src/tools/list_schemas.rs`
- `src/tools/list_tables.rs`
- `src/pg/queries/list_schemas.sql`
- `src/pg/queries/list_tables.sql`
- `tests/discovery.rs`

### Step 10.1 — Write `list_schemas.sql`

**File: `src/pg/queries/list_schemas.sql`**

```sql
-- src/pg/queries/list_schemas.sql
--
-- Returns all schemas in the current database visible to the connected role,
-- excluding internal Postgres schemas.
-- Used by the list_schemas tool.
--
-- Columns returned (in order):
--   name         TEXT    — schema name
--   owner        TEXT    — owning role name
--   description  TEXT    — comment on the schema, NULL if none
--
-- Exclusions:
--   pg_catalog        — Postgres system catalog (internal)
--   information_schema — SQL standard information schema (internal)
--   pg_toast          — internal TOAST storage
--   pg_temp_*         — per-session temporary schema
--   pg_toast_temp_*   — temporary TOAST schemas
--
-- has_schema_privilege() filters to schemas the connected role can USAGE.
-- This prevents listing schemas the role has no access to, which mirrors
-- what pg_tables and information_schema.schemata would show.

SELECT
    n.nspname                                               AS name,
    r.rolname                                               AS owner,
    d.description                                           AS description
FROM pg_namespace n
JOIN pg_roles r ON r.oid = n.nspowner
LEFT JOIN pg_description d
    ON d.objoid = n.oid
    AND d.classoid = 'pg_namespace'::regclass
WHERE
    n.nspname NOT IN ('pg_catalog', 'information_schema')
    AND n.nspname NOT LIKE 'pg_toast%'
    AND n.nspname NOT LIKE 'pg_temp_%'
    AND has_schema_privilege(n.nspname, 'USAGE')
ORDER BY n.nspname
```

### Step 10.2 — Write `list_tables.sql`

**File: `src/pg/queries/list_tables.sql`**

```sql
-- src/pg/queries/list_tables.sql
--
-- Returns tables, views, and materialized views in a given schema.
-- Used by the list_tables tool.
--
-- Parameters (bound positionally):
--   $1  TEXT    — schema name (exact match against nspname)
--   $2  TEXT[]  — relkind filter: 'r' (table), 'v' (view), 'm' (mat. view)
--                 Pass ARRAY['r','v','m'] for "all".
--
-- Columns returned (in order):
--   schema       TEXT    — schema name (echoes $1)
--   name         TEXT    — table/view name
--   kind         TEXT    — 'table', 'view', or 'materialized_view'
--   row_estimate INT8    — row count estimate from pg_class.reltuples; -1 means
--                          stats not yet collected; NULL for views
--   description  TEXT    — COMMENT ON TABLE, NULL if none
--
-- Note: reltuples is -1 for tables that have never been ANALYZEd, and 0
-- immediately after CREATE TABLE. Callers should treat -1 and 0 as "unknown".

SELECT
    n.nspname                                               AS schema,
    c.relname                                               AS name,
    CASE c.relkind
        WHEN 'r' THEN 'table'
        WHEN 'v' THEN 'view'
        WHEN 'm' THEN 'materialized_view'
    END                                                     AS kind,
    CASE
        WHEN c.relkind IN ('v') THEN NULL
        ELSE c.reltuples::int8
    END                                                     AS row_estimate,
    d.description                                           AS description
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_description d
    ON d.objoid = c.oid
    AND d.objsubid = 0
    AND d.classoid = 'pg_class'::regclass
WHERE
    n.nspname = $1
    AND c.relkind = ANY($2)
    AND NOT c.relispartition              -- exclude child partition tables
    AND has_table_privilege(c.oid, 'SELECT')
ORDER BY c.relname
```

### Step 10.3 — Write tests first

Add to `tests/discovery.rs`:

```rust
use pgmcp::tools::{list_schemas, list_tables};

// ── list_schemas ──────────────────────────────────────────────────────────────

/// list_schemas returns an array.
#[tokio::test]
async fn test_list_schemas_returns_array() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    let found = schemas.iter().any(|s| s["name"] == "public");
    assert!(found, "public schema must be present");
}

/// list_schemas excludes pg_toast and pg_catalog.
#[tokio::test]
async fn test_list_schemas_excludes_internal_schemas() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    let internal = ["pg_toast", "pg_catalog", "information_schema"];
    for s in schemas {
        let name = s["name"].as_str().unwrap();
        assert!(
            !internal.contains(&name),
            "internal schema '{name}' must not be in list_schemas output"
        );
    }
}

/// Each schema entry has name and owner fields.
#[tokio::test]
async fn test_list_schemas_entries_have_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_schemas::handle(test_ctx(&url), None)
        .await
        .expect("list_schemas must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    for s in schemas {
        assert!(s["name"].is_string(), "name must be a string");
        assert!(s["owner"].is_string(), "owner must be a string");
    }
}

// ── list_tables ───────────────────────────────────────────────────────────────

/// Helper: create a test table via a raw pool connection.
async fn create_test_table(url: &str, ddl: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct connect for DDL");
    tokio::spawn(conn);
    client.execute(ddl, &[]).await.expect("DDL must succeed");
}

/// list_tables with kind=table returns tables in public schema.
#[tokio::test]
async fn test_list_tables_returns_tables() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_test (id serial PRIMARY KEY, val text)",
    )
    .await;

    let args = serde_json::from_str(r#"{"schema":"public","kind":"table"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    let found = tables.iter().any(|t| t["name"] == "phase3_lt_test");
    assert!(found, "phase3_lt_test must appear in list_tables(public, table)");
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
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_fields (id serial PRIMARY KEY)",
    )
    .await;
    let args = serde_json::from_str(r#"{"schema":"public","kind":"table"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
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
}

/// list_tables with kind=view does not return tables (only views/empty).
#[tokio::test]
async fn test_list_tables_view_filter_excludes_tables() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_test_table(
        &url,
        "CREATE TABLE IF NOT EXISTS public.phase3_lt_notview (id serial PRIMARY KEY)",
    )
    .await;
    let args = serde_json::from_str(r#"{"schema":"public","kind":"view"}"#).ok();
    let result = list_tables::handle(test_ctx(&url), args)
        .await
        .expect("list_tables must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    let any_table = tables.iter().any(|t| t["kind"] == "table");
    assert!(!any_table, "kind=view filter must not return tables");
}

/// list_tables with missing schema parameter returns param_invalid error.
#[tokio::test]
async fn test_list_tables_missing_schema_is_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_tables::handle(test_ctx(&url), None).await;
    assert!(result.is_err(), "missing schema should return an error");
    let err = result.unwrap_err();
    assert_eq!(err.code(), "param_invalid", "error code must be param_invalid");
}

/// list_tables with unknown schema returns empty tables array.
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
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert!(tables.is_empty(), "unknown schema must return empty tables array");
}
```

### Step 10.4 — Implement `tools/list_schemas.rs`

```rust
// src/tools/list_schemas.rs
//
// list_schemas tool — returns visible schemas in the current database.
//
// Returns a JSON object:
//   {
//     "schemas": [
//       {
//         "name":        string,
//         "owner":       string,
//         "description": string | null
//       },
//       ...
//     ]
//   }
//
// Excludes pg_catalog, information_schema, pg_toast*, and pg_temp_* schemas.
// Only schemas the connected role has USAGE privilege on are returned.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/list_schemas.sql
    let rows = client
        .query(
            "SELECT n.nspname, r.rolname, d.description \
            FROM pg_namespace n \
            JOIN pg_roles r ON r.oid = n.nspowner \
            LEFT JOIN pg_description d \
                ON d.objoid = n.oid AND d.classoid = 'pg_namespace'::regclass \
            WHERE \
                n.nspname NOT IN ('pg_catalog', 'information_schema') \
                AND n.nspname NOT LIKE 'pg_toast%' \
                AND n.nspname NOT LIKE 'pg_temp_%' \
                AND has_schema_privilege(n.nspname, 'USAGE') \
            ORDER BY n.nspname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let schemas: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let owner: String = row.get(1);
            let description: Option<String> = row.get(2);
            serde_json::json!({
                "name":        name,
                "owner":       owner,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "schemas": schemas });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

### Step 10.5 — Implement `tools/list_tables.rs`

```rust
// src/tools/list_tables.rs
//
// list_tables tool — returns tables, views, and materialized views in a schema.
//
// Parameters:
//   schema (string, required)  — schema name
//   kind   (string, optional)  — "table" | "view" | "materialized_view" | "all"
//                                defaults to "table"
//
// Returns a JSON object:
//   {
//     "tables": [
//       {
//         "schema":       string,
//         "name":         string,
//         "kind":         "table" | "view" | "materialized_view",
//         "row_estimate": number | null,
//         "description":  string | null
//       },
//       ...
//     ]
//   }

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Map user-facing kind string to pg_class.relkind character(s).
fn kind_to_relkinds(kind: &str) -> Result<Vec<&'static str>, McpError> {
    match kind {
        "table"            => Ok(vec!["r"]),
        "view"             => Ok(vec!["v"]),
        "materialized_view"=> Ok(vec!["m"]),
        "all"              => Ok(vec!["r", "v", "m"]),
        other => Err(McpError::param_invalid(
            "kind",
            format!("must be one of 'table', 'view', 'materialized_view', 'all'; got '{other}'"),
        )),
    }
}

pub(crate) async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let args = args.ok_or_else(|| McpError::param_invalid("schema", "required parameter missing"))?;

    let schema = args
        .get("schema")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::param_invalid("schema", "required string parameter is missing"))?
        .to_string();

    let kind_str = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("table");

    let relkinds: Vec<&str> = kind_to_relkinds(kind_str)?;

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Build the relkind array literal for $2 — tokio-postgres cannot pass
    // Vec<&str> as TEXT[] directly, so we pass individual params or build
    // a dynamic query.  The cleaner approach: use a fixed-length ANY($2)
    // with an array of text.  tokio-postgres supports Vec<String> as TEXT[].
    let relkinds_owned: Vec<String> = relkinds.into_iter().map(String::from).collect();

    // SQL matches src/pg/queries/list_tables.sql
    let rows = client
        .query(
            "SELECT \
                n.nspname, \
                c.relname, \
                CASE c.relkind \
                    WHEN 'r' THEN 'table' \
                    WHEN 'v' THEN 'view' \
                    WHEN 'm' THEN 'materialized_view' \
                END, \
                CASE WHEN c.relkind IN ('v') THEN NULL ELSE c.reltuples::int8 END, \
                d.description \
            FROM pg_class c \
            JOIN pg_namespace n ON n.oid = c.relnamespace \
            LEFT JOIN pg_description d \
                ON d.objoid = c.oid AND d.objsubid = 0 \
                AND d.classoid = 'pg_class'::regclass \
            WHERE \
                n.nspname = $1 \
                AND c.relkind = ANY($2) \
                AND NOT c.relispartition \
                AND has_table_privilege(c.oid, 'SELECT') \
            ORDER BY c.relname",
            &[&schema, &relkinds_owned],
        )
        .await
        .map_err(McpError::from)?;

    let tables: Vec<Value> = rows
        .iter()
        .map(|row| {
            let schema_name: String = row.get(0);
            let name: String = row.get(1);
            let kind: String = row.get(2);
            let row_estimate: Option<i64> = row.get(3);
            let description: Option<String> = row.get(4);
            serde_json::json!({
                "schema":       schema_name,
                "name":         name,
                "kind":         kind,
                "row_estimate": row_estimate,
                "description":  description,
            })
        })
        .collect();

    let body = serde_json::json!({ "tables": tables });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Run:**
```
cargo test --test discovery
```

Expected:
```
test test_list_schemas_returns_array ... ok
test test_list_schemas_includes_public ... ok
test test_list_schemas_excludes_internal_schemas ... ok
test test_list_schemas_entries_have_required_fields ... ok
test test_list_tables_returns_tables ... ok
test test_list_tables_entry_fields ... ok
test test_list_tables_view_filter_excludes_tables ... ok
test test_list_tables_missing_schema_is_param_invalid ... ok
test test_list_tables_unknown_schema_returns_empty ... ok
```

**Full check:**
```
cargo clippy -- -D warnings && cargo fmt --check && cargo test
```

### feat/010 Acceptance Checklist

- [ ] `list_schemas` returns `{"schemas": [...]}` excluding internal schemas
- [ ] `list_schemas` includes `public`, excludes `pg_toast`, `pg_catalog`, `information_schema`
- [ ] `list_tables` accepts `schema` (required) and `kind` (optional, default "table")
- [ ] `list_tables` `kind=view` excludes tables; `kind=table` excludes views
- [ ] `list_tables` missing `schema` returns `McpError::param_invalid`
- [ ] `list_tables` unknown schema returns empty array (not an error)
- [ ] `list_schemas.sql` and `list_tables.sql` contain production queries
- [ ] All 9 new integration tests pass

---

## feat/011 — describe_table

**Branch:** `feat/011-describe-table`  
**Depends on:** feat/010 merged  
**Files modified:**
- `src/tools/describe_table.rs`
- `src/tools/list_enums.rs`
- `src/pg/queries/describe_table.sql`
- `src/pg/queries/list_enums.sql`
- `tests/discovery.rs`

**PostgreSQL agent review required for `describe_table.sql` — this is the most complex query in Phase 3.**

### Step 11.1 — Write `describe_table.sql`

`describe_table` executes **three** parallel SQL queries inside `tokio::join!`. Each query is parameterized with `$1` = schema name and `$2` = table name.

**Query A — columns:**

```sql
-- src/pg/queries/describe_table.sql
-- Query A: Column definitions
--
-- Columns returned (in order):
--   attname        TEXT    — column name
--   type_name      TEXT    — Postgres type name (e.g. 'integer', 'text', 'timestamptz')
--   not_null       BOOL    — true if column has NOT NULL constraint
--   has_default    BOOL    — true if column has a default expression
--   default_expr   TEXT    — the default expression text, NULL if none
--   col_comment    TEXT    — COMMENT ON COLUMN text, NULL if none
--   attnum         INT2    — column ordinal position (for ordering)
--
-- Notes:
--   pg_type.typname gives the base type name; use pg_catalog.format_type() for
--   the full name including array notation and type modifiers (e.g. varchar(50)).
--   attnum > 0 excludes system columns (oid, ctid, etc.).
--   attisdropped = false excludes columns dropped with ALTER TABLE DROP COLUMN.

SELECT
    a.attname                                               AS attname,
    pg_catalog.format_type(a.atttypid, a.atttypmod)        AS type_name,
    a.attnotnull                                            AS not_null,
    a.atthasdef                                             AS has_default,
    pg_get_expr(ad.adbin, ad.adrelid)                       AS default_expr,
    col_desc.description                                    AS col_comment,
    a.attnum                                                AS attnum
FROM pg_attribute a
JOIN pg_class c
    ON c.oid = a.attrelid
JOIN pg_namespace n
    ON n.oid = c.relnamespace
LEFT JOIN pg_attrdef ad
    ON ad.adrelid = a.attrelid
    AND ad.adnum = a.attnum
LEFT JOIN pg_description col_desc
    ON col_desc.objoid = a.attrelid
    AND col_desc.objsubid = a.attnum
    AND col_desc.classoid = 'pg_class'::regclass
WHERE
    n.nspname = $1
    AND c.relname = $2
    AND a.attnum > 0
    AND NOT a.attisdropped
ORDER BY a.attnum
```

**Query B — constraints (primary key, unique, foreign keys, check):**

```sql
-- Query B: Constraints
--
-- Columns returned (in order):
--   conname        TEXT    — constraint name
--   contype        CHAR    — 'p'=PK, 'u'=unique, 'f'=FK, 'c'=check
--   constrained_cols TEXT[] — array of column names in this constraint
--   fk_schema      TEXT    — for FK: referenced schema name, NULL otherwise
--   fk_table       TEXT    — for FK: referenced table name, NULL otherwise
--   fk_cols        TEXT[]  — for FK: referenced column names, NULL otherwise
--   check_expr     TEXT    — for check constraints: human-readable expression

SELECT
    con.conname                                                     AS conname,
    con.contype                                                     AS contype,
    ARRAY(
        SELECT a.attname
        FROM pg_attribute a
        WHERE a.attrelid = con.conrelid
          AND a.attnum = ANY(con.conkey)
        ORDER BY array_position(con.conkey, a.attnum)
    )                                                               AS constrained_cols,
    fn.nspname                                                      AS fk_schema,
    fc.relname                                                      AS fk_table,
    CASE
        WHEN con.contype = 'f' THEN
            ARRAY(
                SELECT a.attname
                FROM pg_attribute a
                WHERE a.attrelid = con.confrelid
                  AND a.attnum = ANY(con.confkey)
                ORDER BY array_position(con.confkey, a.attnum)
            )
        ELSE NULL
    END                                                             AS fk_cols,
    pg_get_constraintdef(con.oid, true)                             AS check_expr
FROM pg_constraint con
JOIN pg_class c
    ON c.oid = con.conrelid
JOIN pg_namespace n
    ON n.oid = c.relnamespace
LEFT JOIN pg_class fc
    ON fc.oid = con.confrelid
LEFT JOIN pg_namespace fn
    ON fn.oid = fc.relnamespace
WHERE
    n.nspname = $1
    AND c.relname = $2
    AND con.contype IN ('p', 'u', 'f', 'c')
ORDER BY con.contype, con.conname
```

**Query C — indexes:**

```sql
-- Query C: Indexes
--
-- Columns returned (in order):
--   indexname      TEXT    — index name
--   is_unique      BOOL    — true if the index enforces uniqueness
--   is_primary     BOOL    — true if this is the primary key index
--   index_cols     TEXT[]  — indexed column names in order

SELECT
    ic.relname                                                      AS indexname,
    ix.indisunique                                                  AS is_unique,
    ix.indisprimary                                                 AS is_primary,
    ARRAY(
        SELECT a.attname
        FROM pg_attribute a
        WHERE a.attrelid = ix.indrelid
          AND a.attnum = ANY(ix.indkey)
          AND a.attnum > 0
        ORDER BY array_position(ix.indkey::int2[], a.attnum)
    )                                                               AS index_cols
FROM pg_index ix
JOIN pg_class c
    ON c.oid = ix.indrelid
JOIN pg_namespace n
    ON n.oid = c.relnamespace
JOIN pg_class ic
    ON ic.oid = ix.indexrelid
WHERE
    n.nspname = $1
    AND c.relname = $2
ORDER BY ic.relname
```

### Step 11.2 — Write `list_enums.sql`

**File: `src/pg/queries/list_enums.sql`**

```sql
-- src/pg/queries/list_enums.sql
--
-- Returns all enum types in a schema with their ordered label values.
-- Used by the list_enums tool.
--
-- Parameters:
--   $1  TEXT  — schema name (exact match)
--
-- Columns returned (in order):
--   schema  TEXT    — schema name
--   name    TEXT    — enum type name
--   values  TEXT[]  — enum labels in definition order (sorted by enumsortorder)
--
-- Notes:
--   pg_enum.enumsortorder is a float4 assigned when enum labels are added.
--   Labels added with ALTER TYPE ... ADD VALUE get a non-integer sort order to
--   maintain the insertion order without renumbering existing labels.
--   Sorting by enumsortorder ASC gives definition order.

SELECT
    n.nspname                                                       AS schema,
    t.typname                                                       AS name,
    ARRAY(
        SELECT e.enumlabel
        FROM pg_enum e
        WHERE e.enumtypid = t.oid
        ORDER BY e.enumsortorder
    )                                                               AS values
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE
    t.typtype = 'e'
    AND n.nspname = $1
ORDER BY t.typname
```

### Step 11.3 — Write tests first

Add to `tests/discovery.rs`:

```rust
use pgmcp::tools::{describe_table, list_enums};

// ── describe_table fixture setup ──────────────────────────────────────────────

/// Creates a realistic test table with columns, PK, unique constraint, FK,
/// check constraint, and a secondary index for describe_table integration tests.
async fn create_describe_table_fixtures(url: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls)
        .await
        .expect("direct connect for DDL");
    tokio::spawn(conn);

    // Parent table (referenced by FK)
    client.execute(
        "CREATE TABLE IF NOT EXISTS public.phase3_dt_parent ( \
            id serial PRIMARY KEY, \
            code text NOT NULL UNIQUE \
        )",
        &[],
    ).await.expect("parent table DDL");

    // Main test table
    client.execute(
        "CREATE TABLE IF NOT EXISTS public.phase3_dt_child ( \
            id         serial PRIMARY KEY, \
            parent_id  integer NOT NULL REFERENCES public.phase3_dt_parent(id), \
            name       text NOT NULL, \
            score      numeric(10,2) CHECK (score >= 0 AND score <= 100), \
            created_at timestamptz NOT NULL DEFAULT now(), \
            note       text \
        )",
        &[],
    ).await.expect("child table DDL");

    // Additional index on name
    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_phase3_dt_child_name \
         ON public.phase3_dt_child (name)",
        &[],
    ).await.expect("index DDL");

    // Column comment
    client.execute(
        "COMMENT ON COLUMN public.phase3_dt_child.score IS 'Score between 0 and 100'",
        &[],
    ).await.expect("column comment");
}

/// describe_table returns all top-level fields.
#[tokio::test]
async fn test_describe_table_has_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;

    let args = serde_json::from_str(
        r#"{"schema":"public","table":"phase3_dt_child"}"#
    ).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &["columns", "primary_key", "unique_constraints", "foreign_keys", "indexes", "check_constraints"] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}

/// describe_table columns array has correct structure.
#[tokio::test]
async fn test_describe_table_columns_structure() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let cols = v["columns"].as_array().expect("columns must be array");
    assert!(!cols.is_empty(), "columns must not be empty");
    for col in cols {
        assert!(col["name"].is_string(), "col.name must be string");
        assert!(col["type"].is_string(), "col.type must be string");
        assert!(col["nullable"].is_boolean(), "col.nullable must be bool");
    }
}

/// primary_key is ["id"] for phase3_dt_child.
#[tokio::test]
async fn test_describe_table_primary_key_is_id() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let pk = v["primary_key"].as_array().expect("primary_key must be array");
    assert_eq!(pk.len(), 1, "PK should have exactly one column");
    assert_eq!(pk[0], "id", "PK column must be 'id'");
}

/// foreign_keys includes the parent_id reference.
#[tokio::test]
async fn test_describe_table_foreign_key_parent_id() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let fks = v["foreign_keys"].as_array().expect("foreign_keys must be array");
    assert!(!fks.is_empty(), "phase3_dt_child has a FK on parent_id");
    let fk = fks.iter().find(|fk| {
        fk["columns"].as_array().map_or(false, |cols| cols.contains(&Value::String("parent_id".into())))
    });
    assert!(fk.is_some(), "FK on parent_id must be present");
    let fk = fk.unwrap();
    assert_eq!(fk["referenced_table"], "phase3_dt_parent");
    let ref_cols = fk["referenced_columns"].as_array().unwrap();
    assert!(ref_cols.contains(&Value::String("id".into())));
}

/// check_constraints includes the score range constraint.
#[tokio::test]
async fn test_describe_table_check_constraint_score() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let checks = v["check_constraints"].as_array().expect("check_constraints must be array");
    assert!(!checks.is_empty(), "score check constraint must be present");
    let any_score = checks.iter().any(|c| {
        c["definition"].as_str().map_or(false, |d| d.contains("score"))
    });
    assert!(any_score, "check constraint definition must reference 'score'");
}

/// indexes includes the secondary index on name.
#[tokio::test]
async fn test_describe_table_indexes_include_name_index() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let indexes = v["indexes"].as_array().expect("indexes must be array");
    let name_idx = indexes.iter().find(|i| i["name"] == "idx_phase3_dt_child_name");
    assert!(name_idx.is_some(), "idx_phase3_dt_child_name must be listed");
    let cols = name_idx.unwrap()["columns"].as_array().unwrap();
    assert!(cols.contains(&Value::String("name".into())));
}

/// column comment is returned for the score column.
#[tokio::test]
async fn test_describe_table_column_comment_score() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_describe_table_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public","table":"phase3_dt_child"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args)
        .await
        .expect("describe_table must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let cols = v["columns"].as_array().unwrap();
    let score_col = cols.iter().find(|c| c["name"] == "score").expect("score column must exist");
    let comment = score_col["comment"].as_str();
    assert_eq!(comment, Some("Score between 0 and 100"));
}

/// describe_table for nonexistent table returns table_not_found error.
#[tokio::test]
async fn test_describe_table_nonexistent_returns_table_not_found() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let args = serde_json::from_str(r#"{"schema":"public","table":"absolutely_does_not_exist_xyz"}"#).ok();
    let result = describe_table::handle(test_ctx(&url), args).await;
    assert!(result.is_err(), "nonexistent table must return error");
    let err = result.unwrap_err();
    assert_eq!(err.code(), "table_not_found");
}

/// describe_table with missing parameters returns param_invalid.
#[tokio::test]
async fn test_describe_table_missing_params_is_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = describe_table::handle(test_ctx(&url), None).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}

// ── list_enums ────────────────────────────────────────────────────────────────

async fn create_enum_fixtures(url: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls).await.expect("connect");
    tokio::spawn(conn);
    client.execute(
        "DO $$ BEGIN \
            IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'phase3_status') \
            THEN CREATE TYPE public.phase3_status AS ENUM ('pending', 'active', 'closed'); \
            END IF; \
         END $$",
        &[],
    ).await.expect("enum DDL");
}

/// list_enums returns enums in the schema.
#[tokio::test]
async fn test_list_enums_returns_phase3_status() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_enum_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public"}"#).ok();
    let result = list_enums::handle(test_ctx(&url), args)
        .await
        .expect("list_enums must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let enums = v["enums"].as_array().expect("enums must be array");
    let status = enums.iter().find(|e| e["name"] == "phase3_status");
    assert!(status.is_some(), "phase3_status enum must be present");
}

/// list_enums values are in definition order.
#[tokio::test]
async fn test_list_enums_values_in_definition_order() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_enum_fixtures(&url).await;
    let args = serde_json::from_str(r#"{"schema":"public"}"#).ok();
    let result = list_enums::handle(test_ctx(&url), args)
        .await
        .expect("list_enums must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let enums = v["enums"].as_array().unwrap();
    let status = enums.iter().find(|e| e["name"] == "phase3_status").unwrap();
    let values: Vec<&str> = status["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(values, vec!["pending", "active", "closed"]);
}
```

### Step 11.4 — Implement `tools/describe_table.rs`

```rust
// src/tools/describe_table.rs
//
// describe_table tool — returns the full definition of a table.
//
// Parameters:
//   schema (string, required)
//   table  (string, required)
//
// Returns a JSON object:
//   {
//     "columns": [
//       {
//         "name":     string,
//         "type":     string,
//         "nullable": bool,
//         "default":  string | null,
//         "comment":  string | null
//       }
//     ],
//     "primary_key":         string[],
//     "unique_constraints":  [{"name": string, "columns": string[]}],
//     "foreign_keys":        [{"name": string, "columns": string[],
//                              "referenced_table": string,
//                              "referenced_schema": string,
//                              "referenced_columns": string[]}],
//     "indexes":             [{"name": string, "columns": string[],
//                              "is_unique": bool, "is_primary": bool}],
//     "check_constraints":   [{"name": string, "definition": string}]
//   }
//
// Three queries run in parallel via tokio::join!:
//   Query A: columns (pg_attribute + pg_attrdef + pg_description)
//   Query B: constraints (pg_constraint)
//   Query C: indexes (pg_index)
//
// The table is verified to exist by checking that Query A returns at least one
// row. Zero columns means the table does not exist (or is not accessible).

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

// Column query — matches Query A in describe_table.sql
const COLUMN_QUERY: &str = "\
    SELECT \
        a.attname, \
        pg_catalog.format_type(a.atttypid, a.atttypmod), \
        a.attnotnull, \
        a.atthasdef, \
        pg_get_expr(ad.adbin, ad.adrelid), \
        col_desc.description, \
        a.attnum \
    FROM pg_attribute a \
    JOIN pg_class c ON c.oid = a.attrelid \
    JOIN pg_namespace n ON n.oid = c.relnamespace \
    LEFT JOIN pg_attrdef ad \
        ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
    LEFT JOIN pg_description col_desc \
        ON col_desc.objoid = a.attrelid \
        AND col_desc.objsubid = a.attnum \
        AND col_desc.classoid = 'pg_class'::regclass \
    WHERE n.nspname = $1 AND c.relname = $2 \
        AND a.attnum > 0 AND NOT a.attisdropped \
    ORDER BY a.attnum";

// Constraint query — matches Query B in describe_table.sql
const CONSTRAINT_QUERY: &str = "\
    SELECT \
        con.conname, \
        con.contype, \
        ARRAY( \
            SELECT a.attname \
            FROM pg_attribute a \
            WHERE a.attrelid = con.conrelid AND a.attnum = ANY(con.conkey) \
            ORDER BY array_position(con.conkey, a.attnum) \
        ), \
        fn.nspname, \
        fc.relname, \
        CASE WHEN con.contype = 'f' THEN \
            ARRAY( \
                SELECT a.attname \
                FROM pg_attribute a \
                WHERE a.attrelid = con.confrelid AND a.attnum = ANY(con.confkey) \
                ORDER BY array_position(con.confkey, a.attnum) \
            ) \
        ELSE NULL END, \
        pg_get_constraintdef(con.oid, true) \
    FROM pg_constraint con \
    JOIN pg_class c ON c.oid = con.conrelid \
    JOIN pg_namespace n ON n.oid = c.relnamespace \
    LEFT JOIN pg_class fc ON fc.oid = con.confrelid \
    LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
    WHERE n.nspname = $1 AND c.relname = $2 \
        AND con.contype IN ('p', 'u', 'f', 'c') \
    ORDER BY con.contype, con.conname";

// Index query — matches Query C in describe_table.sql
const INDEX_QUERY: &str = "\
    SELECT \
        ic.relname, \
        ix.indisunique, \
        ix.indisprimary, \
        ARRAY( \
            SELECT a.attname \
            FROM pg_attribute a \
            WHERE a.attrelid = ix.indrelid \
                AND a.attnum = ANY(ix.indkey) \
                AND a.attnum > 0 \
            ORDER BY array_position(ix.indkey::int2[], a.attnum) \
        ) \
    FROM pg_index ix \
    JOIN pg_class c ON c.oid = ix.indrelid \
    JOIN pg_namespace n ON n.oid = c.relnamespace \
    JOIN pg_class ic ON ic.oid = ix.indexrelid \
    WHERE n.nspname = $1 AND c.relname = $2 \
    ORDER BY ic.relname";

pub(crate) async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let args = args.ok_or_else(|| {
        McpError::param_invalid("schema", "required parameters 'schema' and 'table' are missing")
    })?;

    let schema = args
        .get("schema")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::param_invalid("schema", "required string parameter is missing"))?
        .to_string();

    let table = args
        .get("table")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::param_invalid("table", "required string parameter is missing"))?
        .to_string();

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);

    // Acquire ONE connection and run the three queries sequentially on it.
    // (Three separate connections with join! would saturate a small pool during
    // concurrent calls.  Sequential on one connection is simpler and still fast
    // because all three queries are catalog scans — no disk I/O.)
    let client = ctx.pool.get(timeout).await?;

    let col_rows = client
        .query(COLUMN_QUERY, &[&schema, &table])
        .await
        .map_err(McpError::from)?;

    // Zero columns means the table does not exist or is inaccessible.
    if col_rows.is_empty() {
        return Err(McpError::table_not_found(&schema, &table));
    }

    let con_rows = client
        .query(CONSTRAINT_QUERY, &[&schema, &table])
        .await
        .map_err(McpError::from)?;

    let idx_rows = client
        .query(INDEX_QUERY, &[&schema, &table])
        .await
        .map_err(McpError::from)?;

    // ── Build columns ─────────────────────────────────────────────────────────
    let columns: Vec<Value> = col_rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let type_name: String = row.get(1);
            let not_null: bool = row.get(2);
            let _has_default: bool = row.get(3);
            let default_expr: Option<String> = row.get(4);
            let comment: Option<String> = row.get(5);
            serde_json::json!({
                "name":     name,
                "type":     type_name,
                "nullable": !not_null,
                "default":  default_expr,
                "comment":  comment,
            })
        })
        .collect();

    // ── Build constraint categories ───────────────────────────────────────────
    let mut primary_key: Vec<String> = Vec::new();
    let mut unique_constraints: Vec<Value> = Vec::new();
    let mut foreign_keys: Vec<Value> = Vec::new();
    let mut check_constraints: Vec<Value> = Vec::new();

    for row in &con_rows {
        let conname: String = row.get(0);
        let contype: i8 = row.get(1);
        let cols: Vec<String> = row.get(2);
        let fk_schema: Option<String> = row.get(3);
        let fk_table: Option<String> = row.get(4);
        let fk_cols: Option<Vec<String>> = row.get(5);
        let check_expr: String = row.get(6);

        match contype as u8 as char {
            'p' => {
                primary_key = cols;
            }
            'u' => {
                unique_constraints.push(serde_json::json!({
                    "name":    conname,
                    "columns": cols,
                }));
            }
            'f' => {
                foreign_keys.push(serde_json::json!({
                    "name":               conname,
                    "columns":            cols,
                    "referenced_schema":  fk_schema,
                    "referenced_table":   fk_table,
                    "referenced_columns": fk_cols,
                }));
            }
            'c' => {
                check_constraints.push(serde_json::json!({
                    "name":       conname,
                    "definition": check_expr,
                }));
            }
            _ => {}
        }
    }

    // ── Build indexes ─────────────────────────────────────────────────────────
    let indexes: Vec<Value> = idx_rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let is_unique: bool = row.get(1);
            let is_primary: bool = row.get(2);
            let cols: Vec<String> = row.get(3);
            serde_json::json!({
                "name":       name,
                "columns":    cols,
                "is_unique":  is_unique,
                "is_primary": is_primary,
            })
        })
        .collect();

    let body = serde_json::json!({
        "columns":            columns,
        "primary_key":        primary_key,
        "unique_constraints": unique_constraints,
        "foreign_keys":       foreign_keys,
        "indexes":            indexes,
        "check_constraints":  check_constraints,
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Implementation note on `contype`:** `pg_constraint.contype` is a `char` in Postgres but maps to `i8` in tokio-postgres. The pattern `row.get::<_, i8>(1) as u8 as char` is the correct conversion. Do not match on `"p"` as a string — match on the char value via the cast shown above.

**Implementation note on `indkey`:** `pg_index.indkey` is an `int2vector` — it maps to `Vec<i16>` in tokio-postgres. The `array_position(ix.indkey::int2[], ...)` cast in the SQL converts it to a proper array for the `array_position()` function. This is the correct approach for PG 14+.

### Step 11.5 — Implement `tools/list_enums.rs`

```rust
// src/tools/list_enums.rs
//
// list_enums tool — returns all enum types in a schema with ordered values.
//
// Parameters:
//   schema (string, optional, default "public")
//
// Returns a JSON object:
//   {
//     "enums": [
//       {
//         "schema": string,
//         "name":   string,
//         "values": string[]
//       }
//     ]
//   }
//
// Values are returned in definition order (sorted by enumsortorder).

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let schema = args
        .as_ref()
        .and_then(|a| a.get("schema"))
        .and_then(|v| v.as_str())
        .unwrap_or("public")
        .to_string();

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/list_enums.sql
    let rows = client
        .query(
            "SELECT n.nspname, t.typname, \
                ARRAY( \
                    SELECT e.enumlabel \
                    FROM pg_enum e \
                    WHERE e.enumtypid = t.oid \
                    ORDER BY e.enumsortorder \
                ) \
            FROM pg_type t \
            JOIN pg_namespace n ON n.oid = t.typnamespace \
            WHERE t.typtype = 'e' AND n.nspname = $1 \
            ORDER BY t.typname",
            &[&schema],
        )
        .await
        .map_err(McpError::from)?;

    let enums: Vec<Value> = rows
        .iter()
        .map(|row| {
            let schema_name: String = row.get(0);
            let name: String = row.get(1);
            let values: Vec<String> = row.get(2);
            serde_json::json!({
                "schema": schema_name,
                "name":   name,
                "values": values,
            })
        })
        .collect();

    let body = serde_json::json!({ "enums": enums });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Run:**
```
cargo test --test discovery
```

Expected output (all prior + new tests):
```
test test_describe_table_has_required_fields ... ok
test test_describe_table_columns_structure ... ok
test test_describe_table_primary_key_is_id ... ok
test test_describe_table_foreign_key_parent_id ... ok
test test_describe_table_check_constraint_score ... ok
test test_describe_table_indexes_include_name_index ... ok
test test_describe_table_column_comment_score ... ok
test test_describe_table_nonexistent_returns_table_not_found ... ok
test test_describe_table_missing_params_is_param_invalid ... ok
test test_list_enums_returns_phase3_status ... ok
test test_list_enums_values_in_definition_order ... ok
```

**Full check:**
```
cargo clippy -- -D warnings && cargo fmt --check && cargo test
```

### PostgreSQL Agent Review Note

The `describe_table` implementation contains the most complex pg_catalog queries in Phase 3. Before merging `feat/011`, the PostgreSQL agent should review `src/pg/queries/describe_table.sql` and the inline SQL constants in `src/tools/describe_table.rs` for:

1. **Correctness of `array_position(con.conkey, a.attnum)`** — `conkey` and `confkey` are `int2[]` in PG 14+. Verify that `array_position` on `int2[]` with an `int2` operand resolves correctly without an explicit cast. If not, use `a.attnum::int2` or cast the entire array.

2. **`indkey::int2[]` cast** — `pg_index.indkey` is `int2vector`. The cast `ix.indkey::int2[]` is required for `array_position()` to accept it. Verify this cast is stable across PG 14-17.

3. **Expression indexes** — `ix.indkey` contains `0` for expression index columns. The filter `a.attnum > 0` in the index column subquery means expression indexes will appear with an empty `columns` array. Verify this is acceptable behavior for Phase 3 (full expression index support is deferred to Phase 4 schema cache).

4. **Partitioned tables** — The constraint and index queries do not filter by `relkind`. Verify they handle partitioned tables (relkind='p') correctly and do not duplicate results from child partitions.

### feat/011 Acceptance Checklist

- [ ] `describe_table` returns all 6 required top-level fields
- [ ] `columns` has `name`, `type`, `nullable`, `default`, `comment`
- [ ] `primary_key` is `["id"]` for `phase3_dt_child`
- [ ] `foreign_keys` includes the parent_id reference with correct referenced_table
- [ ] `check_constraints` includes the score range constraint
- [ ] `indexes` includes `idx_phase3_dt_child_name`
- [ ] Column comment is returned for the score column
- [ ] Nonexistent table returns `McpError::table_not_found`
- [ ] Missing params returns `McpError::param_invalid`
- [ ] `list_enums` returns `phase3_status` with values in definition order `["pending", "active", "closed"]`
- [ ] `describe_table.sql` and `list_enums.sql` contain the production queries
- [ ] PostgreSQL agent has reviewed and signed off on `describe_table.sql`
- [ ] All 11 new integration tests pass

---

## feat/012 — list_extensions + table_stats

**Branch:** `feat/012-list-extensions-table-stats`  
**Depends on:** feat/011 merged  
**Files modified:**
- `src/tools/list_extensions.rs`
- `src/tools/table_stats.rs`
- `src/pg/queries/list_extensions.sql`
- `src/pg/queries/table_stats.sql`
- `tests/discovery.rs`

### Step 12.1 — Write `list_extensions.sql`

**File: `src/pg/queries/list_extensions.sql`**

```sql
-- src/pg/queries/list_extensions.sql
--
-- Returns all extensions currently installed in the current database.
-- Used by the list_extensions tool.
--
-- Columns returned (in order):
--   name        TEXT    — extension name (e.g. 'plpgsql', 'pg_stat_statements')
--   version     TEXT    — installed version string
--   schema      TEXT    — schema where the extension objects live
--   description TEXT    — extension description from pg_available_extensions, NULL if unavailable
--
-- Notes:
--   pg_extension contains only installed extensions.
--   pg_available_extensions is a function-based view available since PG 9.1;
--   it provides the description field. LEFT JOIN handles the edge case where
--   pg_available_extensions doesn't list a locally installed extension.

SELECT
    e.extname                                                       AS name,
    e.extversion                                                    AS version,
    n.nspname                                                       AS schema,
    ae.comment                                                      AS description
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
LEFT JOIN pg_available_extensions ae ON ae.name = e.extname
ORDER BY e.extname
```

### Step 12.2 — Write `table_stats.sql`

**File: `src/pg/queries/table_stats.sql`**

```sql
-- src/pg/queries/table_stats.sql
--
-- Returns runtime statistics for a table from pg_stat_user_tables and pg_class.
-- Used by the table_stats tool.
--
-- Parameters:
--   $1  TEXT  — schema name
--   $2  TEXT  — table name
--
-- Columns returned (in order):
--   schema            TEXT    — schema name (echoes $1)
--   name              TEXT    — table name (echoes $2)
--   row_estimate      INT8    — row count estimate from pg_class.reltuples
--   live_tuples       INT8    — live tuple count from pg_stat_user_tables
--   dead_tuples       INT8    — dead (bloat) tuple count
--   seq_scans         INT8    — number of sequential scans since last stats reset
--   idx_scans         INT8    — number of index scans since last stats reset
--   last_vacuum       TIMESTAMPTZ  — last time vacuum ran, NULL if never
--   last_analyze      TIMESTAMPTZ  — last time analyze ran, NULL if never
--   table_size_bytes  INT8    — pg_table_size(oid): heap + TOAST + free space map
--   toast_size_bytes  INT8    — pg_total_relation_size - pg_indexes_size - pg_table_size
--   index_size_bytes  INT8    — pg_indexes_size(oid): total size of all indexes
--
-- Notes:
--   pg_stat_user_tables tracks per-table access statistics. It only covers
--   user tables (not pg_catalog). The view may not have an entry if the table
--   was never accessed since the last stats reset — COALESCE to 0 in that case.
--
--   pg_class.reltuples is -1 for tables never ANALYZEd, 0 immediately after
--   CREATE TABLE. Report as-is; let the caller interpret.

SELECT
    c.relnamespace::regnamespace::text                              AS schema,
    c.relname                                                       AS name,
    c.reltuples::int8                                               AS row_estimate,
    COALESCE(s.n_live_tup, 0)                                       AS live_tuples,
    COALESCE(s.n_dead_tup, 0)                                       AS dead_tuples,
    COALESCE(s.seq_scan, 0)                                         AS seq_scans,
    COALESCE(s.idx_scan, 0)                                         AS idx_scans,
    s.last_vacuum                                                   AS last_vacuum,
    s.last_analyze                                                  AS last_analyze,
    pg_table_size(c.oid)                                            AS table_size_bytes,
    pg_total_relation_size(c.oid) - pg_table_size(c.oid)
        - pg_indexes_size(c.oid)                                    AS toast_size_bytes,
    pg_indexes_size(c.oid)                                          AS index_size_bytes
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_stat_user_tables s
    ON s.relid = c.oid
WHERE
    n.nspname = $1
    AND c.relname = $2
    AND c.relkind = 'r'
```

### Step 12.3 — Write tests first

Add to `tests/discovery.rs`:

```rust
use pgmcp::tools::{list_extensions, table_stats};

// ── list_extensions ───────────────────────────────────────────────────────────

/// list_extensions returns an array including plpgsql.
#[tokio::test]
async fn test_list_extensions_includes_plpgsql() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_extensions::handle(test_ctx(&url), None)
        .await
        .expect("list_extensions must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let extensions = v["extensions"].as_array().expect("extensions must be array");
    let plpgsql = extensions.iter().find(|e| e["name"] == "plpgsql");
    assert!(plpgsql.is_some(), "plpgsql must always be installed");
}

/// Each extension entry has required fields.
#[tokio::test]
async fn test_list_extensions_entry_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_extensions::handle(test_ctx(&url), None)
        .await
        .expect("list_extensions must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let extensions = v["extensions"].as_array().unwrap();
    for ext in extensions {
        assert!(ext["name"].is_string(), "name must be string");
        assert!(ext["version"].is_string(), "version must be string");
        assert!(ext["schema"].is_string(), "schema must be string");
        // description may be null
    }
}

/// plpgsql extension has a non-empty version.
#[tokio::test]
async fn test_list_extensions_plpgsql_version_nonempty() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = list_extensions::handle(test_ctx(&url), None)
        .await
        .expect("list_extensions must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let extensions = v["extensions"].as_array().unwrap();
    let plpgsql = extensions.iter().find(|e| e["name"] == "plpgsql").unwrap();
    let version = plpgsql["version"].as_str().unwrap();
    assert!(!version.is_empty(), "plpgsql version must not be empty");
}

// ── table_stats ───────────────────────────────────────────────────────────────

async fn create_stats_fixture(url: &str) {
    use tokio_postgres::NoTls;
    let (client, conn) = tokio_postgres::connect(url, NoTls).await.expect("connect");
    tokio::spawn(conn);
    client.execute(
        "CREATE TABLE IF NOT EXISTS public.phase3_ts_target ( \
            id serial PRIMARY KEY, \
            data text \
        )",
        &[],
    ).await.expect("stats fixture DDL");
    // Insert some rows so size is non-trivial
    client.execute(
        "INSERT INTO public.phase3_ts_target (data) \
         SELECT 'value_' || g FROM generate_series(1, 100) g \
         ON CONFLICT DO NOTHING",
        &[],
    ).await.expect("stats fixture insert");
    client.execute("ANALYZE public.phase3_ts_target", &[]).await.expect("ANALYZE");
}

/// table_stats returns all required fields.
#[tokio::test]
async fn test_table_stats_has_required_fields() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_stats_fixture(&url).await;
    let args = serde_json::from_str(
        r#"{"schema":"public","table":"phase3_ts_target"}"#
    ).ok();
    let result = table_stats::handle(test_ctx(&url), args)
        .await
        .expect("table_stats must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &[
        "schema", "table", "row_estimate", "live_tuples", "dead_tuples",
        "seq_scans", "idx_scans", "table_size_bytes", "toast_size_bytes",
        "index_size_bytes",
    ] {
        assert!(v.get(field).is_some(), "missing field: {field}");
    }
}

/// table_stats table_size_bytes is greater than zero after inserting rows.
#[tokio::test]
async fn test_table_stats_size_is_nonzero() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_stats_fixture(&url).await;
    let args = serde_json::from_str(
        r#"{"schema":"public","table":"phase3_ts_target"}"#
    ).ok();
    let result = table_stats::handle(test_ctx(&url), args)
        .await
        .expect("table_stats must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    let size = v["table_size_bytes"].as_i64().expect("table_size_bytes must be integer");
    assert!(size > 0, "table_size_bytes must be > 0, got {size}");
}

/// last_vacuum and last_analyze are null or valid timestamp strings.
#[tokio::test]
async fn test_table_stats_vacuum_analyze_are_null_or_strings() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    create_stats_fixture(&url).await;
    let args = serde_json::from_str(
        r#"{"schema":"public","table":"phase3_ts_target"}"#
    ).ok();
    let result = table_stats::handle(test_ctx(&url), args)
        .await
        .expect("table_stats must succeed");
    let text = result.content[0].as_text().unwrap();
    let v: Value = serde_json::from_str(text).unwrap();
    for field in &["last_vacuum", "last_analyze"] {
        let val = &v[field];
        assert!(
            val.is_null() || val.is_string(),
            "{field} must be null or a string timestamp, got: {val:?}"
        );
    }
}

/// table_stats for a nonexistent table returns table_not_found.
#[tokio::test]
async fn test_table_stats_nonexistent_table_returns_table_not_found() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let args = serde_json::from_str(
        r#"{"schema":"public","table":"absolutely_not_there_xyz"}"#
    ).ok();
    let result = table_stats::handle(test_ctx(&url), args).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "table_not_found");
}

/// table_stats with missing schema parameter returns param_invalid.
#[tokio::test]
async fn test_table_stats_missing_schema_is_param_invalid() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };
    let result = table_stats::handle(test_ctx(&url), None).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), "param_invalid");
}
```

### Step 12.4 — Implement `tools/list_extensions.rs`

```rust
// src/tools/list_extensions.rs
//
// list_extensions tool — returns all installed Postgres extensions.
//
// Parameters: none.
//
// Returns a JSON object:
//   {
//     "extensions": [
//       {
//         "name":        string,
//         "version":     string,
//         "schema":      string,
//         "description": string | null
//       }
//     ]
//   }

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/list_extensions.sql
    let rows = client
        .query(
            "SELECT e.extname, e.extversion, n.nspname, ae.comment \
            FROM pg_extension e \
            JOIN pg_namespace n ON n.oid = e.extnamespace \
            LEFT JOIN pg_available_extensions ae ON ae.name = e.extname \
            ORDER BY e.extname",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let extensions: Vec<Value> = rows
        .iter()
        .map(|row| {
            let name: String = row.get(0);
            let version: String = row.get(1);
            let schema: String = row.get(2);
            let description: Option<String> = row.get(3);
            serde_json::json!({
                "name":        name,
                "version":     version,
                "schema":      schema,
                "description": description,
            })
        })
        .collect();

    let body = serde_json::json!({ "extensions": extensions });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

### Step 12.5 — Implement `tools/table_stats.rs`

```rust
// src/tools/table_stats.rs
//
// table_stats tool — returns runtime statistics for a table.
//
// Parameters:
//   schema (string, required)
//   table  (string, required)
//
// Returns a JSON object with statistics from pg_stat_user_tables and pg_class.
// Timestamps are serialized as ISO 8601 strings or null.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let args = args.ok_or_else(|| {
        McpError::param_invalid("schema", "required parameters 'schema' and 'table' are missing")
    })?;

    let schema = args
        .get("schema")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::param_invalid("schema", "required string parameter is missing"))?
        .to_string();

    let table = args
        .get("table")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::param_invalid("table", "required string parameter is missing"))?
        .to_string();

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // SQL matches src/pg/queries/table_stats.sql.
    // last_vacuum and last_analyze are TIMESTAMPTZ; tokio-postgres maps them to
    // Option<time::OffsetDateTime> when the "with-time-0_3" feature is enabled.
    let rows = client
        .query(
            "SELECT \
                c.relnamespace::regnamespace::text, \
                c.relname, \
                c.reltuples::int8, \
                COALESCE(s.n_live_tup, 0), \
                COALESCE(s.n_dead_tup, 0), \
                COALESCE(s.seq_scan, 0), \
                COALESCE(s.idx_scan, 0), \
                s.last_vacuum, \
                s.last_analyze, \
                pg_table_size(c.oid), \
                pg_total_relation_size(c.oid) - pg_table_size(c.oid) - pg_indexes_size(c.oid), \
                pg_indexes_size(c.oid) \
            FROM pg_class c \
            JOIN pg_namespace n ON n.oid = c.relnamespace \
            LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid \
            WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'r'",
            &[&schema, &table],
        )
        .await
        .map_err(McpError::from)?;

    let row = rows.into_iter().next().ok_or_else(|| {
        McpError::table_not_found(&schema, &table)
    })?;

    let schema_name: String = row.get(0);
    let table_name: String = row.get(1);
    let row_estimate: i64 = row.get(2);
    let live_tuples: i64 = row.get(3);
    let dead_tuples: i64 = row.get(4);
    let seq_scans: i64 = row.get(5);
    let idx_scans: i64 = row.get(6);
    // TIMESTAMPTZ columns — tokio-postgres returns Option<time::OffsetDateTime>
    let last_vacuum: Option<time::OffsetDateTime> = row.get(7);
    let last_analyze: Option<time::OffsetDateTime> = row.get(8);
    let table_size_bytes: i64 = row.get(9);
    let toast_size_bytes: i64 = row.get(10);
    let index_size_bytes: i64 = row.get(11);

    // Format timestamps as ISO 8601 strings or JSON null.
    let fmt_ts = |ts: Option<time::OffsetDateTime>| -> Value {
        match ts {
            Some(t) => {
                let s = t.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default();
                Value::String(s)
            }
            None => Value::Null,
        }
    };

    let body = serde_json::json!({
        "schema":            schema_name,
        "table":             table_name,
        "row_estimate":      row_estimate,
        "live_tuples":       live_tuples,
        "dead_tuples":       dead_tuples,
        "seq_scans":         seq_scans,
        "idx_scans":         idx_scans,
        "last_vacuum":       fmt_ts(last_vacuum),
        "last_analyze":      fmt_ts(last_analyze),
        "table_size_bytes":  table_size_bytes,
        "toast_size_bytes":  toast_size_bytes,
        "index_size_bytes":  index_size_bytes,
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
```

**Note on `time::OffsetDateTime`:** The `Cargo.toml` already includes `tokio-postgres = { features = ["with-time-0_3"] }` and `time = { version = "0.3", features = ["serde"] }`. The `time::format_description::well_known::Rfc3339` formatter produces ISO 8601 strings compatible with JSON consumers.

**Run:**
```
cargo test --test discovery
```

Expected — all discovery tests green:
```
test test_list_extensions_includes_plpgsql ... ok
test test_list_extensions_entry_fields ... ok
test test_list_extensions_plpgsql_version_nonempty ... ok
test test_table_stats_has_required_fields ... ok
test test_table_stats_size_is_nonzero ... ok
test test_table_stats_vacuum_analyze_are_null_or_strings ... ok
test test_table_stats_nonexistent_table_returns_table_not_found ... ok
test test_table_stats_missing_schema_is_param_invalid ... ok
```

**Full Phase 3 test run:**
```
cargo test 2>&1 | tail -5
```

Expected:
```
test result: ok. N passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Final clippy + format:**
```
cargo clippy -- -D warnings && cargo fmt --check
```

### feat/012 Acceptance Checklist

- [ ] `list_extensions` returns `{"extensions": [...]}` including `plpgsql`
- [ ] Each extension entry has `name`, `version`, `schema`, and nullable `description`
- [ ] `table_stats` returns all 12 required fields
- [ ] `table_stats` `table_size_bytes` > 0 after inserting rows
- [ ] `last_vacuum` and `last_analyze` are ISO 8601 strings or null
- [ ] `table_stats` nonexistent table returns `McpError::table_not_found`
- [ ] `table_stats` missing schema returns `McpError::param_invalid`
- [ ] `list_extensions.sql` and `table_stats.sql` contain production queries
- [ ] All 8 new integration tests pass

---

## Phase 3 Complete: Final Verification

After all 5 branches are merged into main:

```bash
# All unit + integration tests pass
cargo test

# Zero clippy warnings across the entire codebase
cargo clippy -- -D warnings

# Code is formatted
cargo fmt --check

# Release build succeeds
cargo build --release
```

**Expected final test count:** 82 existing tests + ~38 new Phase 3 tests = approximately 120 tests total.

### Phase 3 Test Inventory

| Test file | Tests added | Branch |
|-----------|------------|--------|
| `tests/health.rs` | 8 | feat/008 |
| `tests/discovery.rs` | ~30 | feat/009 – feat/012 |

### SQL Files Summary

| File | Tool | Branch |
|------|------|--------|
| `src/pg/queries/server_settings.sql` | `health`, `connection_info`, `server_info` | feat/008 + feat/009 |
| `src/pg/queries/list_databases.sql` | `list_databases` | feat/009 |
| `src/pg/queries/list_schemas.sql` | `list_schemas` | feat/010 |
| `src/pg/queries/list_tables.sql` | `list_tables` | feat/010 |
| `src/pg/queries/describe_table.sql` | `describe_table` | feat/011 |
| `src/pg/queries/list_enums.sql` | `list_enums` | feat/011 |
| `src/pg/queries/list_extensions.sql` | `list_extensions` | feat/012 |
| `src/pg/queries/table_stats.sql` | `table_stats` | feat/012 |

### Response Shape Summary

| Tool | Response root key | Primary fields |
|------|------------------|----------------|
| `health` | top-level object | `status`, `pg_reachable`, `pool_available`, `latency_ms`, `pool_stats` |
| `connection_info` | top-level object | `host`, `port`, `database`, `role`, `ssl`, `server_version`, `pool` |
| `server_info` | top-level object | `version`, `version_num`, `settings`, `role` |
| `list_databases` | `databases` array | `name`, `owner`, `encoding`, `size_bytes`, `description` |
| `list_schemas` | `schemas` array | `name`, `owner`, `description` |
| `list_tables` | `tables` array | `schema`, `name`, `kind`, `row_estimate`, `description` |
| `describe_table` | top-level object | `columns`, `primary_key`, `unique_constraints`, `foreign_keys`, `indexes`, `check_constraints` |
| `list_enums` | `enums` array | `schema`, `name`, `values` |
| `list_extensions` | `extensions` array | `name`, `version`, `schema`, `description` |
| `table_stats` | top-level object | 12 fields: schema, table, row_estimate, live_tuples, dead_tuples, seq_scans, idx_scans, last_vacuum, last_analyze, table_size_bytes, toast_size_bytes, index_size_bytes |

### Invariants to verify before Phase 4

1. No handler calls `unwrap()` or `expect()` outside `#[cfg(test)]`.
2. Every handler releases its pool connection before returning (connections are dropped when the `client` variable goes out of scope — verify no `client` is moved into a closure or held across an `.await` after the final query).
3. Every `Err` path maps to a specific `McpError` variant — no raw `Box<dyn Error>` escapes.
4. `describe_table` correctly returns `McpError::table_not_found` when the column query returns zero rows — this is the only existence check, and it is correct because every existing table has at least one column.

---

## Appendix: Common Pitfalls

### tokio-postgres type mappings for pg_catalog columns

| Postgres type | Rust type via tokio-postgres |
|---------------|------------------------------|
| `TEXT` / `VARCHAR` | `String` |
| `INT2` / `SMALLINT` | `i16` |
| `INT4` / `INTEGER` | `i32` |
| `INT8` / `BIGINT` | `i64` |
| `BOOL` | `bool` |
| `TEXT[]` | `Vec<String>` |
| `TIMESTAMPTZ` | `Option<time::OffsetDateTime>` (requires `with-time-0_3` feature) |
| `OID` | `u32` |
| `"char"` (single-byte char type) | `i8` — cast to `u8` then `char` |

The `pg_constraint.contype` and `pg_class.relkind` columns in pg_catalog are Postgres's single-byte `"char"` type. In tokio-postgres, `row.get::<_, i8>(n) as u8 as char` is the correct extraction pattern.

### Passing `TEXT[]` parameters with tokio-postgres

tokio-postgres accepts `Vec<String>` as a `TEXT[]` parameter. Pass it as `&relkinds_owned` where `relkinds_owned: Vec<String>`. The driver handles the array encoding automatically.

### `has_table_privilege` may error on dropped tables

If a table is being concurrently dropped during a `list_tables` call, `has_table_privilege(c.oid, 'SELECT')` may error. This is acceptable for Phase 3 — if the query fails, the entire `McpError::pg_query_failed` is returned. Cache-based protection is added in Phase 4 (feat/013).

### `pg_database_size()` requires CONNECT privilege

In `list_databases`, the `CASE WHEN d.datallowconn THEN pg_database_size(d.oid) ELSE NULL END` handles `template0` (datallowconn = false). For databases where the role has `CONNECT` privilege but `pg_database_size()` still errors (rare edge case), the error propagates as `pg_query_failed`. This is acceptable — the caller can re-issue without the size field in Phase 4.

### `array_position` with `int2vector`

`pg_index.indkey` is the `int2vector` type. To use it with `array_position()`, cast it to `int2[]` explicitly: `ix.indkey::int2[]`. Without this cast, PG raises `function array_position(int2vector, integer) does not exist`. The cast is stable across PG 14–17.
