# pgmcp Master Implementation Plan

> **For agentic workers:** This is the roadmap. Detailed per-phase implementation plans are in separate documents. Execute phases sequentially — do not start Phase N+1 until Phase N is fully merged.

**Project:** pgmcp — Rust MCP server for PostgreSQL  
**Spec:** docs/specs/2026-04-07-pgmcp-mvp-design.md  
**Total branches:** 29  
**Phases:** 9  
**Last updated:** 2026-04-07

---

## Branch Status Summary

| # | Branch | Phase | Title | Status |
|---|--------|-------|-------|--------|
| 1 | feat/001 | 1 | project-scaffold | Pending |
| 2 | feat/002 | 1 | config | Pending |
| 3 | feat/003 | 1 | telemetry | Pending |
| 4 | feat/004 | 1 | error-types | Pending |
| 5 | feat/005 | 2 | connection-pool | Pending |
| 6 | feat/006 | 2 | mcp-protocol | Pending |
| 7 | feat/007 | 2 | dispatcher | Pending |
| 8 | feat/008 | 3 | health-connection-info | Pending |
| 9 | feat/009 | 3 | server-info-list-databases | Pending |
| 10 | feat/010 | 3 | list-schemas-list-tables | Pending |
| 11 | feat/011 | 3 | describe-table-list-enums | Pending |
| 12 | feat/012 | 3 | list-extensions-table-stats | Pending |
| 13 | feat/013 | 4 | schema-cache | Pending |
| 14 | feat/014 | 5 | sql-parser | Pending |
| 15 | feat/015 | 5 | guardrails | Pending |
| 16 | feat/016 | 5 | limit-injection | Pending |
| 17 | feat/017 | 6 | streaming-serialization | Pending |
| 18 | feat/018 | 6 | query-tool | Pending |
| 19 | feat/019 | 7 | explain-tool | Pending |
| 20 | feat/020 | 7 | my-permissions | Pending |
| 21 | feat/021 | 7 | suggest-index | Pending |
| 22 | feat/022 | 7 | propose-migration | Pending |
| 23 | feat/023 | 7 | infer-descriptions | Pending |
| 24 | feat/024 | 8 | output-formats | Pending |
| 25 | feat/025 | 8 | tool-descriptions | Pending |
| 26 | feat/026 | 9 | integration-test-hardening | Pending |
| 27 | feat/027 | 9 | ci-pipeline-hardening | Pending |
| 28 | feat/028 | 9 | docker-packaging | Pending |
| 29 | feat/029 | 9 | readme-contributing | Pending |

---

## Phase 1: Foundation (feat/001 — feat/004)

### feat/001 — project-scaffold

**Title:** Initialize Cargo workspace, crate structure, CI skeleton

**Files created:**
- `Cargo.toml` (root, with all 15 runtime dependencies pinned)
- `Cargo.lock` (auto-generated)
- `rust-toolchain.toml` (pins stable toolchain, e.g., stable-2026-03-xx)
- `.gitignore` (standard Rust patterns)
- `deny.toml` (skeleton with license and vulnerability checks)
- `.github/workflows/ci.yml` (bootstrap: `check` job only)
- `src/main.rs` (empty async main with argument parsing)
- `src/lib.rs` (empty lib crate marker)
- `src/config.rs` (empty module stub)
- `src/error.rs` (empty module stub)
- `src/telemetry.rs` (empty module stub)
- `src/transport/mod.rs`, `src/transport/sse.rs`, `src/transport/stdio.rs` (empty stubs)
- `src/server/mod.rs`, `src/server/router.rs`, `src/server/context.rs`, `src/server/tool_defs.rs` (empty stubs)
- `src/tools/mod.rs` and 15 individual tool stubs (all empty)
- `src/sql/mod.rs`, `src/sql/parser.rs`, `src/sql/limit.rs`, `src/sql/guardrails.rs` (empty stubs)
- `src/pg/mod.rs`, `src/pg/pool.rs`, `src/pg/types.rs`, `src/pg/cache.rs`, `src/pg/invalidation.rs`, `src/pg/infer.rs` (empty stubs)
- `src/pg/queries/` directory with SQL file stubs (list_databases.sql, etc.)
- `src/streaming/mod.rs`, `src/streaming/json.rs`, `src/streaming/csv.rs` (empty stubs)
- `tests/common/mod.rs`, `tests/common/fixtures.rs` (empty stubs)
- `tests/integration/` directory with test file stubs (all empty)
- `benches/` directory with benchmark stubs (all empty)
- `config/pgmcp.example.toml` (example configuration file)

**Dependencies:** None (first branch)

**Acceptance criteria:**
- [ ] `cargo build --release` succeeds with zero warnings on stable Rust
- [ ] `cargo fmt --check` passes without formatting changes needed
- [ ] `cargo clippy -- -D warnings` produces zero warnings
- [ ] CI workflow `.github/workflows/ci.yml` exists and runs the `check` job on push
- [ ] All planned module files exist (verified by checking file list with `find src -name "*.rs"`)
- [ ] `rust-toolchain.toml` pins a stable toolchain (not `nightly` or floating version)
- [ ] `deny.toml` contains license and advisory configuration sections
- [ ] `Cargo.lock` is committed to the repository
- [ ] Project can be cloned and `cargo build` runs without network errors (offline mode)
- [ ] All 15 runtime dependencies from spec Section 9.1 are present in `Cargo.toml`
- [ ] Dependency versions match spec Section 9.1 (tokio 1, rmcp latest stable, axum 0.7, tokio-postgres 0.7, etc.)

**Review focus:**
- Verify Cargo.toml dependency versions exactly match Section 9.1 of spec
- Confirm all module structure from Section 5.1 is present (even as stubs)
- Check that `rust-toolchain.toml` uses stable, not nightly
- Verify `.gitignore` includes standard Rust patterns (`target/`, `*.swp`, `.DS_Store`, etc.)
- Confirm CI workflow runs on push and PR to main
- Ensure no production code exists in main.rs (only argument parsing scaffold)

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/002 — config

**Title:** Implement Config struct, TOML deserialization, environment overrides

**Files created/modified:**
- Modify: `src/config.rs` (full implementation)
- Create: `config/pgmcp.example.toml` (complete example configuration)

**Dependencies:** feat/001 (project-scaffold must be merged first)

**Acceptance criteria:**
- [ ] `Config` struct defined in `src/config.rs` with all fields from spec: `database_url`, `pool_max_connections`, `pool_timeout_seconds`, `transport`, `log_format`, `cache_invalidation_interval_seconds`, `guardrail_policy`
- [ ] `Config::from_file(path: &str)` function loads TOML and returns `Result<Config, McpError>` with error code `config_invalid`
- [ ] `Config::from_env()` function applies environment variable overrides (e.g., `PGMCP_DATABASE_URL`)
- [ ] All config fields are validated at load time: database URL is non-empty, pool_max_connections > 0, cache_invalidation_interval_seconds > 0
- [ ] Validation errors produce `McpError` with code `config_invalid` and descriptive message
- [ ] `pgmcp.example.toml` contains complete example with all fields and helpful comments
- [ ] Serde deserialization uses `derive(serde::Deserialize)` with `#[serde(...)]` attributes
- [ ] Enum for `transport` type: `Transport::Stdio | Transport::Sse`
- [ ] Enum for `log_format`: `LogFormat::Json | LogFormat::Human`
- [ ] Environment variable precedence: env vars override config file values
- [ ] Config can be loaded from `pgmcp.example.toml` without errors
- [ ] `cargo test` passes (unit tests for validation rules)
- [ ] All public API items have doc comments

**Review focus:**
- Verify all config fields match spec Section 2 and 3.1
- Confirm validation logic is exhaustive (no partial validation)
- Check that error messages are descriptive and include the invalid value
- Ensure environment variable naming is consistent (`PGMCP_*` prefix)
- Verify `pgmcp.example.toml` is complete and matches the struct
- Look for any use of `.unwrap()` outside of test code

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/003 — telemetry

**Title:** Implement tracing subscriber, log format selection, span instrumentation

**Files created/modified:**
- Modify: `src/telemetry.rs` (full implementation)
- Modify: `src/main.rs` (add telemetry initialization call)

**Dependencies:** feat/002 (config must exist first)

**Acceptance criteria:**
- [ ] `telemetry::init(config: &Config)` function initializes tracing subscriber
- [ ] JSON format initialization: `tracing_subscriber::fmt().json()` when `config.log_format == LogFormat::Json`
- [ ] Human-readable format initialization: `tracing_subscriber::fmt().pretty()` when `config.log_format == LogFormat::Human`
- [ ] `RUST_LOG` environment variable is parsed and applied (e.g., `RUST_LOG=pgmcp=debug`)
- [ ] `main.rs` calls `telemetry::init(&config)` immediately after config load
- [ ] Startup sequence is wrapped in `#[tracing::instrument]` span in main.rs
- [ ] Log output can be verified by running pgmcp and checking stderr for JSON or human-readable output
- [ ] Test demonstrates that `RUST_LOG=pgmcp=trace` produces trace-level logs
- [ ] `cargo test` passes (integration test starts server and verifies telemetry output)

**Review focus:**
- Confirm telemetry is initialized before any fallible code runs
- Verify log format selection correctly uses config value
- Check that `RUST_LOG` override works (not hardcoded to a single level)
- Ensure no blocking operations in the telemetry initialization path
- Verify spans are properly closed (using guard drops)

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/004 — error-types

**Title:** Define McpError enum with all error codes from spec Section 3.5

**Files created/modified:**
- Modify: `src/error.rs` (full implementation)

**Dependencies:** feat/001 (project-scaffold)

**Acceptance criteria:**
- [ ] `McpError` enum defined with all 12 error codes: `config_invalid`, `pg_connect_failed`, `pg_version_unsupported`, `pg_query_failed`, `pg_pool_timeout`, `tool_not_found`, `param_invalid`, `guardrail_violation`, `sql_parse_error`, `schema_not_found`, `table_not_found`, `internal`
- [ ] Each variant includes a human-readable message field
- [ ] `McpError` implements `std::fmt::Display` (via derive or manual impl)
- [ ] `McpError` implements `std::error::Error` (via derive via thiserror or manual impl)
- [ ] `From<tokio_postgres::Error>` conversion implemented, mapping postgres errors to `pg_query_failed` or `pg_connect_failed`
- [ ] `Display` impl produces a message suitable for returning to agents (no internal stack traces)
- [ ] `McpError` is serializable to JSON (derives serde::Serialize)
- [ ] All error codes are present and correct (match spec table)
- [ ] Unit tests verify `Display` output for each error code
- [ ] No other error type is exported from pgmcp modules (all fallible functions return `Result<T, McpError>`)

**Review focus:**
- Verify all 12 error codes from spec Section 3.5 are present
- Check that `Display` message is agent-friendly (no Rust stack traces or internal details)
- Confirm `From<tokio_postgres::Error>` correctly categorizes Postgres errors
- Verify thiserror or manual impl is used correctly
- Ensure serialization works for MCP protocol transmission
- Look for any .unwrap() or .expect() calls

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

## Phase 2: Connectivity (feat/005 — feat/007)

### feat/005 — connection-pool

**Title:** Implement connection pool with health check and version validation

**Files created/modified:**
- Modify: `src/pg/pool.rs` (full implementation)
- Create: `tests/integration/health.rs` (integration tests)

**Dependencies:** feat/001 (project-scaffold), feat/002 (config), feat/004 (error-types)

**Acceptance criteria:**
- [ ] `Pool` type wraps `deadpool_postgres::Pool` in a newtype
- [ ] `Pool::new(config: &Config)` creates a pool with `max_connections` from config, returns `Result<Arc<Pool>, McpError>` with error code `pg_connect_failed`
- [ ] `Pool::acquire(timeout: Duration)` returns a connection or `McpError` with code `pg_pool_timeout` if timeout is exceeded
- [ ] Pool startup performs version check: queries `SELECT version()`, verifies Postgres >= 14, returns `McpError` with code `pg_version_unsupported` if < 14
- [ ] Health check: `SELECT 1` executed on pool acquisition succeeds within configured timeout
- [ ] Integration test `test_pool_connects_to_postgres()` acquires a connection and executes `SELECT 1`
- [ ] Integration test `test_pool_timeout()` verifies timeout error when pool is exhausted
- [ ] Integration test `test_version_check()` verifies Postgres version >= 14
- [ ] Integration test `test_version_unsupported()` mocks version < 14 and verifies error
- [ ] All tests run against real Postgres instance (service container in CI)
- [ ] `cargo test --test health` passes

**Review focus:**
- Verify deadpool-postgres is correctly configured (connection string, pool size, timeout)
- Confirm version check is performed at startup (not deferred)
- Check that timeout is actually applied and errors correctly
- Verify pool acquisition never blocks on synchronous locks
- Ensure error messages include context (which pool, timeout duration)
- Look for connection leaks (all connections returned to pool after use)

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/006 — mcp-protocol

**Title:** Wire up rmcp protocol layer, transports, dispatcher skeleton

**Files created/modified:**
- Modify: `src/transport/stdio.rs` (full implementation)
- Modify: `src/transport/sse.rs` (full implementation)
- Modify: `src/server/router.rs` (dispatcher skeleton)
- Modify: `src/main.rs` (wire up transport selection from config)

**Dependencies:** feat/005 (connection-pool), feat/002 (config)

**Acceptance criteria:**
- [ ] `transport::stdio::run()` initializes rmcp stdio transport, reads MCP messages from stdin, writes to stdout
- [ ] `transport::sse::run()` initializes rmcp SSE transport, listens on configured port, handles HTTP SSE upgrades
- [ ] `main.rs` selects transport based on `config.transport` and calls appropriate `run()` function
- [ ] MCP handshake: server responds to `initialize` with correct protocol version and capabilities
- [ ] `tools/list` endpoint returns empty array `[]` (stubs only)
- [ ] `call_tool` endpoint returns `tool_not_found` error for any tool name
- [ ] Server starts cleanly without panics
- [ ] Server accepts and responds to MCP handshake within 50ms on localhost
- [ ] Both stdio and SSE transports produce identical MCP message format (byte-for-byte)
- [ ] Integration test confirms handshake succeeds
- [ ] Integration test confirms `tools/list` returns empty array

**Review focus:**
- Verify rmcp integration is correct (correct message format, error handling)
- Confirm transport selection works (config.transport routes to correct implementation)
- Check that MCP handshake includes required capabilities
- Verify error responses use correct MCP error format
- Ensure no transport-specific logic bleeds into tool handlers
- Check that stdout/stdin are not mixed with tracing logs

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/007 — dispatcher

**Title:** Implement full dispatcher with ToolContext, routing table, all 15 tool definitions

**Files created/modified:**
- Modify: `src/server/router.rs` (full dispatcher implementation)
- Modify: `src/server/context.rs` (full ToolContext implementation)
- Modify: `src/server/tool_defs.rs` (all 15 tool definitions)

**Dependencies:** feat/006 (mcp-protocol), feat/005 (connection-pool)

**Acceptance criteria:**
- [ ] `ToolContext` struct defined with fields: `Arc<Pool>`, `Arc<SchemaCache>`, `Arc<Config>`
- [ ] `ToolContext` is constructed once per tool call in dispatcher
- [ ] Tool routing table maps tool name to handler function for all 15 tools
- [ ] `tools/list` returns array of all 15 tool definitions with names, descriptions, parameter schemas
- [ ] Parameter schemas are valid JSON Schema (can be parsed and validated)
- [ ] All 15 tool definitions present:
  - Discovery: list_databases, server_info, list_schemas, list_tables, describe_table, list_enums, list_extensions, table_stats
  - SQL: query, explain, suggest_index, propose_migration
  - Introspection: my_permissions, connection_info, health
- [ ] Each tool definition includes: `name` (string), `description` (string), `inputSchema` (JSON Schema object)
- [ ] Parameter validation: tool handlers validate input against schema, return `param_invalid` if validation fails
- [ ] Unknown tool name returns `tool_not_found` error
- [ ] `call_tool` correctly routes to handler and passes ToolContext by value
- [ ] All 15 tool handlers are defined as `pub(crate)` functions
- [ ] Integration test: `tools/list` returns exactly 15 tools with correct names
- [ ] Integration test: unknown tool name returns tool_not_found error
- [ ] Integration test: all 15 tools accept at least one valid call (returns stub response)

**Review focus:**
- Verify routing table is exhaustive (no missing tools)
- Check parameter schemas are complete and match tool specifications
- Confirm ToolContext is passed by value (not references)
- Verify all handlers are `pub(crate)` (not public API)
- Ensure unknown tool name produces correct error code
- Check that parameter validation errors include the validation failure reason

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

## Phase 3: Discovery Tools (feat/008 — feat/012)

### feat/008 — health-connection-info

**Title:** Implement health and connection_info tools with server_settings SQL query

**Files created/modified:**
- Modify: `src/tools/health.rs` (full implementation)
- Modify: `src/tools/connection_info.rs` (full implementation)
- Create: `src/pg/queries/server_settings.sql` (SQL query)
- Modify: `tests/integration/health.rs` (add integration tests)

**Dependencies:** feat/007 (dispatcher)

**Acceptance criteria:**
- [ ] `health()` handler queries pool and executes `SELECT 1` within timeout
- [ ] `health()` returns JSON object with fields: `status` (string: "ok", "degraded", or "unhealthy"), `pool_available` (bool), `pg_reachable` (bool), `schema_cache_age_seconds` (number), `latency_ms` (number)
- [ ] `health()` returns `{"status": "ok", ...}` when pool and Postgres are reachable
- [ ] `health()` returns `{"status": "unhealthy", ...}` when pool cannot connect
- [ ] Latency measured as elapsed time from start of health check to completion (includes pool acquire)
- [ ] `connection_info()` handler returns JSON object with fields: `host` (string), `port` (number), `database` (string), `role` (string), `ssl` (bool), `server_version` (string), `pool` (object with `total`, `idle`, `in_use` numbers)
- [ ] `server_settings.sql` query retrieves host, port, database name from `pg_stat_activity` or `current_setting()`
- [ ] Pool stats (total, idle, in_use) queried from pool metadata
- [ ] Integration test: `health()` with reachable Postgres returns status "ok"
- [ ] Integration test: `health()` with latency measurement is accurate (within 50ms)
- [ ] Integration test: `connection_info()` returns correct host, port, database, role
- [ ] Integration test: `connection_info()` pool stats are correct
- [ ] `cargo test --test health` passes

**Review focus:**
- Verify pool stats are retrieved correctly from deadpool-postgres
- Confirm latency measurement is accurate and includes pool acquisition time
- Check that queries handle missing settings gracefully
- Verify both tools return correct JSON structure
- Ensure no blocking operations in handlers
- Look for connection leaks (all acquired connections returned)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review server_settings.sql query)
- Code reviewer (apply 12-point checklist)

---

### feat/009 — server-info-list-databases

**Title:** Implement server_info and list_databases tools

**Files created/modified:**
- Modify: `src/tools/server_info.rs` (full implementation)
- Modify: `src/tools/list_databases.rs` (full implementation)
- Create: `src/pg/queries/list_databases.sql` (SQL query)
- Modify: `tests/integration/discovery.rs` (add integration tests)

**Dependencies:** feat/008 (health-connection-info), feat/005 (connection-pool)

**Acceptance criteria:**
- [ ] `server_info()` handler returns JSON object with fields: `version` (string, e.g., "PostgreSQL 15.2"), `version_num` (number, e.g., 150002), `settings` (object), `role` (string)
- [ ] `settings` object includes keys: `statement_timeout`, `max_connections`, `work_mem`, `shared_buffers`
- [ ] Settings values retrieved via `SELECT current_setting('key')`
- [ ] `list_databases()` handler returns array of objects, each with fields: `name` (string), `owner` (string), `encoding` (string), `size_bytes` (number | null)
- [ ] `list_databases.sql` queries `pg_database` catalog with size calculation from `pg_database_size()`
- [ ] Size in bytes is calculated correctly (no NULL for accessible databases)
- [ ] Integration test: `server_info()` returns correct version and role
- [ ] Integration test: `server_info()` settings object includes all 4 keys
- [ ] Integration test: `list_databases()` returns at least the test database
- [ ] Integration test: database size is non-zero for test database
- [ ] Integration test: database owner is a non-empty string
- [ ] `cargo test --test discovery` passes

**Review focus:**
- Verify all settings keys are present and correct
- Check that size calculation handles database access permissions
- Confirm version parsing is correct (e.g., 15.2 -> 150002)
- Verify pg_catalog query handles system databases (postgres, template0, template1)
- Ensure JSON encoding of large numbers is correct (no scientific notation for sizes)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review queries)
- Code reviewer (apply 12-point checklist)

---

### feat/010 — list-schemas-list-tables

**Title:** Implement list_schemas and list_tables tools with schema/table filtering

**Files created/modified:**
- Modify: `src/tools/list_schemas.rs` (full implementation)
- Modify: `src/tools/list_tables.rs` (full implementation)
- Create: `src/pg/queries/list_schemas.sql` (SQL query)
- Create: `src/pg/queries/list_tables.sql` (SQL query)
- Modify: `tests/integration/discovery.rs` (add tests for filtering)

**Dependencies:** feat/009 (server-info-list-databases)

**Acceptance criteria:**
- [ ] `list_schemas()` handler returns array of objects: `{name: string, owner: string}`
- [ ] `list_schemas()` excludes internal schemas: `pg_toast`, `pg_temp_*`, `pg_catalog` (unless explicitly requested)
- [ ] `list_tables(schema, kind)` handler accepts required param `schema` (string) and optional `kind` (string: "table" | "view" | "materialized_view" | "all", default "table")
- [ ] `list_tables()` returns array of objects: `{schema: string, name: string, kind: string, row_estimate: number | null, description: string | null}`
- [ ] `list_tables()` filtering by `kind` works: filtering by "table" excludes views, filtering by "view" returns only views, etc.
- [ ] `list_tables()` filtering by schema works: passing schema name returns only tables in that schema
- [ ] Row estimate retrieved from `pg_class.reltuples` or `n_live_tup` from `pg_stat_user_tables`
- [ ] `list_schemas.sql` queries `pg_namespace` with WHERE clause excluding internal schemas
- [ ] `list_tables.sql` queries `pg_class` and `pg_namespace` with kind filtering via `relkind`
- [ ] Integration test: `list_schemas()` returns public schema
- [ ] Integration test: `list_schemas()` excludes pg_toast
- [ ] Integration test: `list_tables(schema="public", kind="table")` returns only tables
- [ ] Integration test: `list_tables(schema="public", kind="view")` returns only views (or empty if none)
- [ ] Integration test: `list_tables()` with invalid schema returns empty array
- [ ] Integration test: row estimates are non-null for tables with data
- [ ] `cargo test --test discovery` passes

**Review focus:**
- Verify relkind filtering is correct (tables='r', views='v', materialized_views='m')
- Confirm schema filtering works across all table kinds
- Check that row estimates handle NULL values correctly (use 0 or null?)
- Verify internal schema exclusion is complete
- Ensure queries don't timeout on large schemas

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review pg_class queries, relkind mapping)
- Code reviewer (apply 12-point checklist)

---

### feat/011 — describe-table-list-enums

**Title:** Implement describe_table and list_enums tools (complex pg_catalog queries)

**Files created/modified:**
- Modify: `src/tools/describe_table.rs` (full implementation)
- Modify: `src/tools/list_enums.rs` (full implementation)
- Create: `src/pg/queries/describe_table.sql` (SQL query)
- Create: `src/pg/queries/list_enums.sql` (SQL query)
- Modify: `tests/integration/discovery.rs` (add complex schema tests)

**Dependencies:** feat/010 (list-schemas-list-tables)

**Acceptance criteria:**
- [ ] `describe_table(schema, table)` handler returns JSON object with fields: `columns`, `primary_key`, `unique_constraints`, `foreign_keys`, `indexes`, `check_constraints`
- [ ] `columns` is array of objects: `{name: string, type: string, nullable: bool, default: string | null, comment: string | null}`
- [ ] Column types use Postgres type names (e.g., "integer", "text", "timestamptz")
- [ ] `primary_key` is array of column names (e.g., ["id"]) or empty array
- [ ] `unique_constraints` is array of objects: `{name: string, columns: string[]}`
- [ ] `foreign_keys` is array of objects: `{name: string, columns: string[], referenced_table: string, referenced_columns: string[]}`
- [ ] `indexes` is array of objects: `{name: string, columns: string[], is_unique: bool, is_primary: bool}`
- [ ] `check_constraints` is array of objects: `{name: string, definition: string}`
- [ ] `describe_table.sql` queries: `pg_attribute`, `pg_constraint`, `pg_index`, `pg_class`, with proper JOINs
- [ ] `list_enums(schema)` handler returns array of objects: `{schema: string, name: string, values: string[]}`
- [ ] Enum values are ordered (in definition order, not alphabetical)
- [ ] `list_enums.sql` queries `pg_enum` and `pg_type` with proper ordering
- [ ] Integration test: `describe_table()` on test table returns all column definitions
- [ ] Integration test: primary key is correctly identified
- [ ] Integration test: unique constraints are listed
- [ ] Integration test: foreign keys include correct references
- [ ] Integration test: indexes are listed with correct columns
- [ ] Integration test: `list_enums()` returns enum types with values in correct order
- [ ] Integration test: invalid schema/table returns error or empty result
- [ ] `cargo test --test discovery` passes

**Review focus:**
- This branch has the most complex pg_catalog queries in the codebase
- Verify query correctness thoroughly: proper constraint classification, FK targets, index details
- Check column type mapping is correct (all Postgres types handled)
- Confirm ordering of enum values (not alphabetical, definition order)
- Verify constraint names are included (for reversibility of migrations)
- Look for NULL handling in optional fields (default, comment)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (deep review of describe_table.sql - this is the critical query)
- Code reviewer (apply 12-point checklist)

---

### feat/012 — list-extensions-table-stats

**Title:** Implement list_extensions and table_stats tools

**Files created/modified:**
- Modify: `src/tools/list_extensions.rs` (full implementation)
- Modify: `src/tools/table_stats.rs` (full implementation)
- Create: `src/pg/queries/list_extensions.sql` (SQL query)
- Modify: `tests/integration/discovery.rs` (add stats tests)

**Dependencies:** feat/011 (describe-table-list-enums)

**Acceptance criteria:**
- [ ] `list_extensions()` handler returns array of objects: `{name: string, version: string, schema: string, description: string | null}`
- [ ] `list_extensions.sql` queries `pg_extension` and `pg_namespace`
- [ ] Extensions include system extensions (e.g., "plpgsql") and installed extensions
- [ ] `table_stats(schema, table)` handler returns object with fields: `schema`, `table`, `row_estimate`, `live_tuples`, `dead_tuples`, `seq_scans`, `idx_scans`, `last_vacuum`, `last_analyze`, `table_size_bytes`, `toast_size_bytes`, `index_size_bytes`
- [ ] Stats from `pg_stat_user_tables` and `pg_class`
- [ ] `row_estimate` is from `pg_class.reltuples` (or estimate if stats not collected)
- [ ] Tuple counts are from `pg_stat_user_tables` (live/dead)
- [ ] Scan counts are from `pg_stat_user_tables`
- [ ] Last vacuum/analyze timestamps are from `pg_stat_user_tables` (nullable)
- [ ] Table size calculated via `pg_total_relation_size()`, broken down into table + toast + indexes
- [ ] Integration test: `list_extensions()` returns at least "plpgsql"
- [ ] Integration test: extension version and schema are non-empty
- [ ] Integration test: `table_stats()` for table with data returns non-zero counts
- [ ] Integration test: table sizes are greater than zero (including index sizes)
- [ ] Integration test: last_vacuum/analyze are NULL or valid timestamps
- [ ] Integration test: row estimates are reasonable
- [ ] `cargo test --test discovery` passes

**Review focus:**
- Verify stats query handles tables with no stats (returns estimates)
- Check that size calculations include all components (table, toast, indexes)
- Confirm scan counts reset to 0 for new tables
- Verify NULL handling for vacuum/analyze timestamps (no indexes are newer than tables)
- Ensure query doesn't timeout on large tables

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review pg_stat_user_tables query)
- Code reviewer (apply 12-point checklist)

---

## Phase 4: Schema Cache (feat/013)

### feat/013 — schema-cache

**Title:** Implement schema cache with background invalidation, update discovery tools to use cache

**Files created/modified:**
- Modify: `src/pg/cache.rs` (full implementation)
- Modify: `src/pg/invalidation.rs` (full implementation)
- Modify: `src/tools/list_databases.rs` (use cache)
- Modify: `src/tools/list_schemas.rs` (use cache)
- Modify: `src/tools/list_tables.rs` (use cache)
- Modify: `src/tools/describe_table.rs` (use cache)
- Modify: `src/tools/list_enums.rs` (use cache)
- Modify: `src/tools/list_extensions.rs` (use cache)
- Modify: `src/tools/table_stats.rs` (use cache)
- Modify: `src/main.rs` (start invalidation background task)
- Create: `tests/integration/schema_cache.rs` (cache tests)

**Dependencies:** feat/012 (list-extensions-table-stats)

**Acceptance criteria:**
- [ ] `SchemaCache` type defined, wraps shared cached state: tables, schemas, enums, extensions, database list
- [ ] Cache is populated at startup (before first tool call) with all discovery data
- [ ] Cache is stored as `Arc<RwLock<SchemaSnapshot>>` or similar (read-heavy workload)
- [ ] `SchemaSnapshot` is an immutable struct capturing full database schema at a point in time
- [ ] Background invalidation task spawned in main.rs, configured to run at `config.cache_invalidation_interval_seconds`
- [ ] Background task acquires full snapshot and replaces cache atomically (readers never see partial state)
- [ ] All discovery tools (008-012) read from cache instead of querying pg_catalog on each call
- [ ] Discovery tools check if data is in cache before executing queries
- [ ] Cache invalidation does not block pool connections (runs in separate async task)
- [ ] Integration test: multiple calls to same discovery tool hit cache (verify via logging/instrumentation)
- [ ] Integration test: cache is refreshed after invalidation interval
- [ ] Integration test: new tables added to database appear in cache after refresh
- [ ] Integration test: schema changes (add column, etc.) appear in cache after refresh
- [ ] Integration test: cache age is reported correctly in `health()` response
- [ ] `cargo test --test schema_cache` passes

**Review focus:**
- Verify cache snapshot is truly atomic (readers see all-or-nothing updates)
- Confirm no concurrent modification issues (RwLock correctness)
- Check background task doesn't hold connections across awaits
- Verify cache miss handling (must still work if cache is empty/stale)
- Ensure cache invalidation doesn't interfere with active queries
- Look for deadlock potential (cache lock vs pool lock)

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

## Phase 5: SQL Analysis Layer (feat/014 — feat/016)

### feat/014 — sql-parser

**Title:** Implement SQL statement parser with StatementKind classification

**Files created/modified:**
- Modify: `src/sql/parser.rs` (full implementation)
- Modify: `tests/integration/guardrails.rs` (add parser unit tests)

**Dependencies:** feat/013 (schema-cache) — technically independent, but logically depends on schema cache being complete

**Acceptance criteria:**
- [ ] `StatementKind` enum defined with variants: `Select`, `Insert`, `Update`, `Delete`, `CreateTable`, `AlterTable`, `CreateIndex`, `DropTable`, `CreateView`, `DropView`, `Truncate`, `Unknown`
- [ ] `parse_statement(sql: &str)` function returns `Result<StatementKind, McpError>` with code `sql_parse_error`
- [ ] Parser uses `sqlparser` crate with Postgres dialect
- [ ] Multi-statement detection: returns error if input contains semicolon-separated statements
- [ ] Identifier validation: validates that table/schema names are valid (no SQLi injections)
- [ ] Parser handles comments (strips or ignores them)
- [ ] Parser handles leading/trailing whitespace
- [ ] Parser handles case-insensitive keywords (SELECT == select)
- [ ] Unit tests cover every `StatementKind` variant with example statements
- [ ] Unit test: multi-statement SQL returns error
- [ ] Unit test: malformed SQL returns `sql_parse_error`
- [ ] Unit test: DDL statements are classified correctly (CREATE vs ALTER vs DROP)
- [ ] Unit test: DML statements are classified correctly (SELECT, INSERT, UPDATE, DELETE)
- [ ] `cargo test` passes

**Review focus:**
- Verify sqlparser is used correctly (dialect, error handling)
- Confirm all statement types are recognized
- Check that error messages include the reason for parse failure
- Verify no panics on malformed input (all errors caught)
- Ensure multi-statement detection is reliable

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

### feat/015 — guardrails

**Title:** Implement guardrail policy with rules for query tool

**Files created/modified:**
- Modify: `src/sql/guardrails.rs` (full implementation)
- Modify: `tests/integration/guardrails.rs` (add guardrail rule tests)

**Dependencies:** feat/014 (sql-parser)

**Acceptance criteria:**
- [ ] `GuardrailPolicy` struct defined with rule set
- [ ] `evaluate(sql: &str, kind: StatementKind, policy: &GuardrailPolicy)` function returns `Result<(), McpError>` with code `guardrail_violation`
- [ ] Rule 1: No DDL in query tool (CREATE, ALTER, DROP, TRUNCATE, etc.)
  - [ ] Test: CREATE TABLE rejected with guardrail_violation
  - [ ] Test: ALTER TABLE rejected
  - [ ] Test: DROP TABLE rejected
- [ ] Rule 2: No COPY TO/FROM with PROGRAM
  - [ ] Test: COPY TO PROGRAM rejected
  - [ ] Test: COPY FROM PROGRAM rejected
- [ ] Rule 3: No SET session-level parameters (prevents affecting subsequent callers)
  - [ ] Test: SET statement_timeout rejected
  - [ ] Test: SET role rejected
- [ ] Guardrail violations return error before pool connection is acquired
- [ ] Error message includes the violated rule and the statement kind
- [ ] Unit tests for all rules
- [ ] Integration tests verify rejected statements don't reach database
- [ ] `cargo test --test guardrails` passes

**Review focus:**
- Verify all guardrail rules are implemented correctly
- Check that error messages are clear (which rule was violated)
- Ensure guardrails are evaluated before pool acquisition (no wasted resources)
- Confirm no bypasses (e.g., SQL injection via comments)
- Look for edge cases (case sensitivity, whitespace handling)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (verify guardrail rules are sufficient and correct)
- Code reviewer (apply 12-point checklist)

---

### feat/016 — limit-injection

**Title:** Implement LIMIT clause injection into SELECT statements

**Files created/modified:**
- Modify: `src/sql/limit.rs` (full implementation)
- Modify: `tests/integration/guardrails.rs` (add limit injection tests)

**Dependencies:** feat/015 (guardrails)

**Acceptance criteria:**
- [ ] `inject_limit(sql: &str, limit_value: u32)` function returns `Result<String, McpError>`
- [ ] Injects `LIMIT <value>` into SELECT statements that lack a LIMIT clause
- [ ] Preserves SELECT statements that already have a LIMIT (no modification)
- [ ] Handles subqueries: injects only into top-level SELECT, not subquery SELECTs
- [ ] Handles UNION/INTERSECT/EXCEPT: injects LIMIT after final query
- [ ] Handles ORDER BY/GROUP BY: LIMIT is placed after ORDER BY (correct semantics)
- [ ] Non-SELECT statements return unchanged
- [ ] Unit test: SELECT without LIMIT gets LIMIT injected
- [ ] Unit test: SELECT with LIMIT is unchanged
- [ ] Unit test: subquery SELECT has LIMIT injected only at top level
- [ ] Unit test: UNION query gets LIMIT at the end
- [ ] Unit test: non-SELECT (INSERT, UPDATE) returns unchanged
- [ ] Integration test: injected LIMIT correctly limits rows in query tool
- [ ] `cargo test` passes

**Review focus:**
- Verify subquery handling (injects only at top level)
- Check UNION/INTERSECT/EXCEPT placement (after the final query)
- Confirm ORDER BY clause position is preserved (LIMIT after ORDER BY)
- Verify no SQL injection in limit_value (should be u32, not string)
- Ensure error handling for invalid LIMIT values

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

## Phase 6: Query Tool (feat/017 — feat/018)

### feat/017 — streaming-serialization

**Title:** Implement row serialization and CSV/JSON encoding with benchmarks

**Files created/modified:**
- Modify: `src/streaming/mod.rs` (full implementation)
- Modify: `src/streaming/json.rs` (full implementation)
- Modify: `src/streaming/csv.rs` (full implementation)
- Modify: `src/pg/types.rs` (full OID to type mapping)
- Create: `benches/serialization.rs` (benchmarks)
- Create: `benches/streaming.rs` (streaming benchmarks)

**Dependencies:** feat/016 (limit-injection) — technically independent, but logically part of query tool infrastructure

**Acceptance criteria:**
- [ ] `RowEncoder` trait defined with methods to encode rows to JSON/CSV
- [ ] `JsonRowEncoder` implementation: encodes rows as JSON objects (one per line)
- [ ] `CsvRowEncoder` implementation: encodes rows as CSV with header row
- [ ] OID-to-Rust-type mapping in `src/pg/types.rs`: int4 -> i32, int8 -> i64, float8 -> f64, text -> String, etc.
- [ ] JSON encoding: null values as `null`, numbers as numbers (not strings), strings as strings
- [ ] JSON encoding: floats use `ryu` for fast conversion (no `format!()`)
- [ ] CSV encoding: null values as empty field, numbers as strings, strings with quotes if needed
- [ ] `BatchSizer` handles row streaming in configurable batch sizes (e.g., 100 rows per batch)
- [ ] Benchmark: JSON encoding throughput >= 10k rows/second on single core
- [ ] Benchmark: CSV encoding throughput >= 10k rows/second on single core
- [ ] Benchmark: no memory allocations in hot path per row
- [ ] Benchmark baseline saved in git
- [ ] `cargo bench` runs without regression (< 5%)
- [ ] Integration test: encoded rows are valid JSON/CSV
- [ ] `cargo test` passes

**Review focus:**
- Verify OID mapping is complete (covers all types in integration tests)
- Check JSON number encoding uses ryu (performance critical)
- Confirm CSV quoting/escaping is correct (RFC 4180 compliant)
- Verify no allocations per-row in encoder
- Benchmark baseline is committed
- Ensure streaming encoder doesn't buffer entire result set

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist, pay special attention to benchmark baseline)

---

### feat/018 — query-tool

**Title:** Implement query tool with SQL analysis, guardrails, streaming, all parameters

**Files created/modified:**
- Modify: `src/tools/query.rs` (full implementation)
- Modify: `src/tools/query_events.rs` (helper for SSE event construction)
- Create: `tests/integration/query.rs` (query tool tests)
- Create: `tests/integration/streaming.rs` (streaming format tests)

**Dependencies:** feat/017 (streaming-serialization)

**Acceptance criteria:**
- [ ] `query(sql, intent, transaction, dry_run, limit, timeout_seconds, format, explain)` handler implemented
- [ ] SQL analysis: `parse_statement()` called on input SQL
- [ ] Guardrails: `evaluate()` called, guardrail_violation error returned before pool acquire
- [ ] LIMIT injection: `inject_limit()` applied if LIMIT missing
- [ ] `dry_run: true` parameter: parses and analyzes SQL without execution, returns `{kind: "SELECT", analysis: ...}`
- [ ] `transaction: true` parameter: wraps DML in explicit transaction, rolls back after execution
- [ ] `timeout_seconds` parameter: applies via `SET LOCAL statement_timeout`, defaults from config
- [ ] `format` parameter: selects JSON or CSV encoding
- [ ] `explain: true` parameter: prepends EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) and returns both plan and rows
- [ ] Response object: `{rows: [], row_count: 123, format: "json", columns: [{name, type}], truncated: false, execution_ms: 45, plan: null}`
- [ ] `rows` field: array of arrays for CSV, array of objects for JSON
- [ ] `truncated: true` if row_count == limit (indicates more rows exist)
- [ ] Column metadata includes name and type for every column
- [ ] Error code `sql_parse_error` for malformed SQL
- [ ] Error code `guardrail_violation` for blocked statements
- [ ] Error code `pg_pool_timeout` if pool exhausted
- [ ] Error code `pg_query_failed` if SQL execution fails
- [ ] Integration test: SELECT query succeeds with JSON format
- [ ] Integration test: SELECT query succeeds with CSV format
- [ ] Integration test: dry_run returns analysis without execution
- [ ] Integration test: transaction mode works (inserts visible in result, not committed)
- [ ] Integration test: timeout enforcement works
- [ ] Integration test: LIMIT injection adds LIMIT if missing
- [ ] Integration test: explain: true returns plan and rows
- [ ] Integration test: DDL rejected with guardrail_violation
- [ ] Integration test: truncated flag set correctly
- [ ] `cargo test --test query` passes
- [ ] `cargo test --test streaming` passes

**Review focus:**
- Verify SQL analysis happens before pool acquire (no wasted connections on bad SQL)
- Check guardrails are evaluated
- Confirm LIMIT injection works
- Verify transaction isolation (changes not committed)
- Check timeout is actually enforced (not just set)
- Ensure column metadata is correct
- Verify both JSON and CSV formats work
- Check truncated flag logic
- Ensure explain output matches explain tool output

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist, benchmark regression check if hot path changes)

---

## Phase 7: Intelligence Tools (feat/019 — feat/023)

### feat/019 — explain-tool

**Title:** Implement explain tool with ANALYZE/plan output

**Files created/modified:**
- Modify: `src/tools/explain.rs` (full implementation)
- Modify: `tests/integration/query.rs` (add explain tests)

**Dependencies:** feat/018 (query-tool)

**Acceptance criteria:**
- [ ] `explain(sql, analyze, buffers)` handler implemented
- [ ] Prepends `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` if `analyze: true` (default)
- [ ] Prepends `EXPLAIN (FORMAT JSON)` if `analyze: false` (plan only, no execution)
- [ ] `buffers: true` parameter: includes BUFFERS statistics (requires analyze: true)
- [ ] Response object: `{plan: {...}, planning_ms: 5, execution_ms: 45}`
- [ ] `plan` is raw Postgres EXPLAIN JSON output (parsed and returned as object, not string)
- [ ] `planning_ms` is time to plan statement (if measured)
- [ ] `execution_ms` is time to execute statement (only if analyze: true)
- [ ] SQL analysis: input SQL is parsed (validates structure before explain)
- [ ] Guardrails: SQL analysis layer checks for blocked statements (e.g., COPY, DDL)
- [ ] Error code `sql_parse_error` for malformed SQL
- [ ] Error code `pg_query_failed` if EXPLAIN itself fails
- [ ] Integration test: `explain()` with analyze: true returns plan with execution stats
- [ ] Integration test: `explain()` with analyze: false returns plan without execution_ms
- [ ] Integration test: plan JSON structure is valid (can be parsed)
- [ ] Integration test: buffers flag controls BUFFERS in output
- [ ] `cargo test --test query` passes

**Review focus:**
- Verify EXPLAIN JSON parsing is correct (not returned as string)
- Check that planning vs execution time is measured correctly
- Confirm analyze/buffers parameters work correctly
- Verify guardrails are applied (no DDL EXPLAIN)
- Ensure error messages are clear

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (verify EXPLAIN output format)
- Code reviewer (apply 12-point checklist)

---

### feat/020 — my-permissions

**Title:** Implement my_permissions tool using has_*_privilege functions

**Files created/modified:**
- Modify: `src/tools/my_permissions.rs` (full implementation)
- Create: `src/pg/queries/my_permissions.sql` (SQL query)
- Create: `tests/integration/permissions.rs` (permission tests)

**Dependencies:** feat/019 (explain-tool) — logically independent, but groups with intelligence tools

**Acceptance criteria:**
- [ ] `my_permissions(schema, table)` handler returns JSON object with fields: `role`, `superuser`, `create_db`, `schemas`, `tables`
- [ ] `role` is the current connected role (string)
- [ ] `superuser` is boolean (from pg_roles)
- [ ] `create_db` is boolean (from pg_roles)
- [ ] `schemas` is array of objects: `{name, usage, create}` (booleans)
- [ ] `schema` param filters to specific schema or all schemas (optional)
- [ ] `table` param adds table-level privilege detail: `tables` is array `{name, select, insert, update, delete}` (booleans)
- [ ] Privileges determined using `has_schema_privilege()`, `has_table_privilege()`, `has_column_privilege()`
- [ ] Integration test: current role privileges are correctly reported
- [ ] Integration test: superuser flag is accurate
- [ ] Integration test: schema/table filtering works
- [ ] Integration test: privilege bits match actual allowed operations
- [ ] `cargo test --test permissions` passes

**Review focus:**
- Verify has_*_privilege functions are used correctly
- Check that all privilege types are reported (SELECT, INSERT, UPDATE, DELETE)
- Confirm role detection is correct
- Verify schema/table filtering works
- Ensure NULL handling for missing schema/table

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review has_*_privilege usage)
- Code reviewer (apply 12-point checklist)

---

### feat/021 — suggest-index

**Title:** Implement suggest_index tool with heuristic rules

**Files created/modified:**
- Modify: `src/tools/suggest_index.rs` (full implementation)
- Modify: `tests/integration/query.rs` (add index suggestion tests)

**Dependencies:** feat/020 (my-permissions)

**Acceptance criteria:**
- [ ] `suggest_index(sql, schema)` handler returns JSON object with fields: `suggestions`, `existing_indexes`
- [ ] `suggestions` is array of objects: `{ddl: string, rationale: string, estimated_benefit: string}`
- [ ] `existing_indexes` is array of objects: `{table, name, columns, type}`
- [ ] Heuristic Rule 1: Missing index on foreign key columns
  - [ ] Test: table with FK on non-indexed column suggests index
- [ ] Heuristic Rule 2: Missing index on columns used in WHERE clause
  - [ ] Test: WHERE column with no index suggests index
- [ ] Heuristic Rule 3: Detect redundant indexes (e.g., both (a) and (a, b))
  - [ ] Test: redundant index is flagged
- [ ] `schema` parameter provides context for unqualified table references
- [ ] SQL is parsed to extract table/column references
- [ ] Current indexes queried from `pg_index` and `pg_attribute`
- [ ] Suggestions prioritized by estimated_benefit
- [ ] Integration test: creates test table with FK on unindexed column, suggests index
- [ ] Integration test: creates test table with WHERE on unindexed column, suggests index
- [ ] Integration test: detects redundant indexes
- [ ] Integration test: existing_indexes lists all current indexes
- [ ] `cargo test --test query` passes

**Review focus:**
- Verify heuristic rules are sound (no false positives for common patterns)
- Check that suggestions are actually beneficial (not just any index)
- Confirm WHERE clause parsing extracts columns correctly
- Verify FK detection works
- Ensure estimated_benefit is reasonable (not meaningless)
- Look for edge cases (partial indexes, expression indexes)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (review index heuristics, pg_index queries)
- Code reviewer (apply 12-point checklist)

---

### feat/022 — propose-migration

**Title:** Implement propose_migration tool with heuristic patterns

**Files created/modified:**
- Modify: `src/tools/propose_migration.rs` (full implementation)
- Modify: `tests/integration/migration.rs` (migration tests)

**Dependencies:** feat/021 (suggest-index)

**Acceptance criteria:**
- [ ] `propose_migration(intent, context_tables, schema)` handler returns JSON object with fields: `statements`, `warnings`
- [ ] `intent` parameter: natural language description of desired migration (e.g., "add user email field")
- [ ] `context_tables` parameter: optional array of table names for context
- [ ] `schema` parameter: default schema for unqualified table names
- [ ] `statements` is array of objects: `{sql: string, explanation: string, reversible: boolean, reverse_sql: string | null}`
- [ ] Heuristic patterns cover: CREATE TABLE, ALTER TABLE ADD COLUMN, CREATE INDEX, CREATE VIEW
- [ ] Each proposed statement includes explanation of why it's needed
- [ ] `reversible` flag is true for statements that can be undone (DDL with DROP equivalent)
- [ ] `reverse_sql` is provided for reversible statements (e.g., DROP INDEX to reverse CREATE INDEX)
- [ ] All proposed SQL statements are syntactically valid (can be parsed by parser)
- [ ] PostgreSQL agent reviews heuristic patterns for correctness
- [ ] Integration test: propose_migration("add email column to users") generates valid ALTER TABLE
- [ ] Integration test: all proposed statements parse correctly (via sql/parser)
- [ ] Integration test: reversible statements have reverse_sql
- [ ] Integration test: warnings are generated for ambiguous intents
- [ ] `cargo test --test migration` passes

**Review focus:**
- PostgreSQL agent: verify heuristic patterns produce correct SQL
- Verify all proposed statements parse correctly (not just syntactically valid)
- Check that reverse SQL is correct and actually reverses the statement
- Ensure explanations are clear and helpful
- Look for edge cases (column name conflicts, type mismatches)
- Verify warnings cover common pitfalls (losing data, etc.)

**Agents:**
- Rust engineer (implement)
- PostgreSQL agent (deep review of migration heuristics and generated SQL)
- Code reviewer (apply 12-point checklist)

---

### feat/023 — infer-descriptions

**Title:** Implement column description inference from naming patterns, wire into tools

**Files created/modified:**
- Modify: `src/pg/infer.rs` (full implementation)
- Modify: `src/tools/describe_table.rs` (integrate inference)
- Modify: `src/tools/propose_migration.rs` (integrate inference)
- Modify: `tests/integration/discovery.rs` (add inference tests)

**Dependencies:** feat/022 (propose-migration)

**Acceptance criteria:**
- [ ] `infer_description(column_name, column_type, column_default)` function returns `Option<String>`
- [ ] Pattern 1: Foreign key convention (`user_id` -> "Foreign key to users table")
  - [ ] Test: `user_id` infers FK description
  - [ ] Test: `post_user_id` infers FK description
- [ ] Pattern 2: Timestamp conventions (`created_at`, `updated_at` -> "Created/Updated timestamp")
  - [ ] Test: `created_at` infers timestamp description
  - [ ] Test: `updated_at` infers timestamp description
- [ ] Pattern 3: Boolean conventions (`is_active`, `enabled` -> "Boolean flag")
  - [ ] Test: `is_active` infers boolean description
  - [ ] Test: `enabled` infers boolean description
- [ ] Pattern 4: Status enum conventions (`status`, `state` -> "Status enumeration")
  - [ ] Test: `status` column infers status description
- [ ] Inferred descriptions are marked as inferred (in output, e.g., with `inferred: true` flag)
- [ ] User-provided descriptions (comments) take precedence over inference
- [ ] `describe_table` output includes inferred descriptions
- [ ] `propose_migration` output includes inferred descriptions for proposed columns
- [ ] Unit tests cover every pattern category
- [ ] Integration test: `describe_table()` returns inferred descriptions for test table
- [ ] Integration test: user-provided comments override inference
- [ ] `cargo test` passes

**Review focus:**
- Verify inference patterns are sound (no false positives)
- Check that user descriptions take precedence
- Ensure inferred flag is set correctly
- Look for edge cases (column names matching multiple patterns)
- Verify integration with describe_table and propose_migration

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist)

---

## Phase 8: Enrichment (feat/024 — feat/025)

### feat/024 — output-formats

**Title:** Validate and polish JSON/CSV output formats, verify column metadata, benchmark

**Files created/modified:**
- Modify: `src/streaming/json.rs` (validation and refinement)
- Modify: `src/streaming/csv.rs` (validation and refinement)
- Modify: `src/tools/query.rs` (validate responses)
- Modify: `benches/serialization.rs` (update benchmarks)
- Modify: `benches/streaming.rs` (update benchmarks)
- Modify: `tests/integration/streaming.rs` (comprehensive format tests)

**Dependencies:** feat/023 (infer-descriptions)

**Acceptance criteria:**
- [ ] JSON format: rows are array of objects (not strings)
- [ ] JSON format: column metadata matches actual result columns
- [ ] JSON format: null values are `null`, not `"null"` strings
- [ ] JSON format: numbers have correct precision (floats use ryu, ints use integer representation)
- [ ] CSV format: RFC 4180 compliant
- [ ] CSV format: column types are in header row (if included)
- [ ] CSV format: null values are empty strings (not `"null"`)
- [ ] CSV format: strings with commas are properly quoted
- [ ] Both formats: `truncated: true` is set when row_count == limit
- [ ] Both formats: `row_count` is accurate (reflects actual number of rows returned)
- [ ] Column metadata: `name` and `type` are present for every column
- [ ] Column metadata: type strings match Postgres type names (int, text, timestamptz, etc.)
- [ ] Benchmark baseline: JSON encoding throughput unchanged (no regression > 5%)
- [ ] Benchmark baseline: CSV encoding throughput unchanged (no regression > 5%)
- [ ] Integration test: JSON and CSV both return same number of rows and columns
- [ ] Integration test: JSON and CSV both accurately represent all data types
- [ ] Integration test: null values are handled correctly in both formats
- [ ] Integration test: large result sets (10k+ rows) encode without memory issues
- [ ] `cargo bench` shows no regression
- [ ] `cargo test --test streaming` passes

**Review focus:**
- Verify JSON/CSV encode the same data (spot-check specific values)
- Confirm column metadata is accurate
- Check that truncated flag is set correctly
- Verify benchmark baseline is committed and no regression
- Ensure both formats handle large result sets
- Look for edge cases (special characters, very large numbers)

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist, benchmark regression check required)

---

### feat/025 — tool-descriptions

**Title:** Finalize all tool and parameter descriptions for LLM consumption

**Files created/modified:**
- Modify: `src/server/tool_defs.rs` (all descriptions)
- Create: `docs/tool-surface.md` (tool reference for review)

**Dependencies:** feat/024 (output-formats)

**Acceptance criteria:**
- [ ] All 15 tool descriptions finalized and LLM-optimized:
  - [ ] `list_databases`: clear, mentions current database
  - [ ] `server_info`: clear, mentions available settings
  - [ ] `list_schemas`: clear, explains internal schema filtering
  - [ ] `list_tables`: clear, explains kind parameter
  - [ ] `describe_table`: clear, lists all returned fields
  - [ ] `list_enums`: clear, explains enum values
  - [ ] `list_extensions`: clear, mentions system extensions
  - [ ] `table_stats`: clear, explains stat sources
  - [ ] `query`: comprehensive, covers all parameters and behavior
  - [ ] `explain`: clear, distinguishes from query with explain:true
  - [ ] `suggest_index`: clear, explains heuristics
  - [ ] `propose_migration`: clear, mentions heuristics and warnings
  - [ ] `my_permissions`: clear, explains privilege checking
  - [ ] `connection_info`: clear, explains pool stats
  - [ ] `health`: clear, explains status values
- [ ] All parameter descriptions are unambiguous
- [ ] Valid values are specified explicitly (e.g., kind: "table" | "view" | "materialized_view" | "all")
- [ ] Edge cases are documented (e.g., describe_table on missing table returns error)
- [ ] Default values are clearly stated (e.g., limit defaults to 1000)
- [ ] Return value descriptions are complete and match actual output
- [ ] Tool descriptions avoid ambiguity and jargon (or define it)
- [ ] Examples are included for non-obvious tools (suggest_index, propose_migration)
- [ ] `tools/list` response includes all finalized descriptions
- [ ] Review checklist: all descriptions are clear and unambiguous
- [ ] `cargo test` passes (verify tools/list output)

**Review focus:**
- This branch is prose-heavy; focus on clarity and completeness
- Verify descriptions are written for LLM consumption (unambiguous, explicit values)
- Check that all parameters are documented
- Ensure edge cases are covered
- Look for undefined jargon
- Verify return value descriptions match actual response structure

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist, pay special attention to prose clarity)

---

## Phase 9: Hardening and Distribution (feat/026 — feat/029)

### feat/026 — integration-test-hardening

**Title:** Expand integration tests to cover all error codes and edge cases

**Files created/modified:**
- Modify: `tests/integration/discovery.rs` (add edge case tests)
- Modify: `tests/integration/query.rs` (add error path tests)
- Modify: `tests/integration/guardrails.rs` (expand guardrail tests)
- Modify: `tests/integration/streaming.rs` (add error tests)
- Modify: `tests/integration/permissions.rs` (add error tests)
- Modify: `tests/integration/health.rs` (add error tests)
- Modify: `tests/integration/migration.rs` (add error tests)
- Modify: `tests/common/fixtures.rs` (additional test data)
- Modify: `.github/workflows/ci.yml` (enable PG version matrix in test job)

**Dependencies:** feat/025 (tool-descriptions)

**Acceptance criteria:**
- [ ] Every error code from spec Section 3.5 has at least one test case:
  - [ ] `config_invalid`: test invalid TOML
  - [ ] `pg_connect_failed`: test unreachable Postgres
  - [ ] `pg_version_unsupported`: test Postgres < 14 (if possible)
  - [ ] `pg_query_failed`: test SQL execution error
  - [ ] `pg_pool_timeout`: test pool exhaustion scenario
  - [ ] `tool_not_found`: test unknown tool name
  - [ ] `param_invalid`: test missing required parameter
  - [ ] `guardrail_violation`: test DDL in query tool
  - [ ] `sql_parse_error`: test malformed SQL
  - [ ] `schema_not_found`: test nonexistent schema
  - [ ] `table_not_found`: test nonexistent table
  - [ ] `internal`: (should be rare; at least document test expectation)
- [ ] Edge case tests:
  - [ ] Empty result set
  - [ ] Very large result set (10k+ rows)
  - [ ] Special characters in column names/values
  - [ ] NULL values in all positions
  - [ ] All Postgres data types covered in serialization
  - [ ] Concurrent tool calls (multiple simultaneous requests)
  - [ ] Schema with 100+ tables
  - [ ] Deeply nested FK relationships
- [ ] Error path tests:
  - [ ] Pool exhaustion: multiple threads acquiring connections
  - [ ] Query timeout: verify timeout is enforced
  - [ ] Missing parameter: each required parameter tested
  - [ ] Invalid parameter value: type mismatches
- [ ] Postgres version matrix: tests pass on PG 14, 15, 16, 17
- [ ] CI workflow updated: test job matrix includes `pg_version: [14, 15, 16, 17]`
- [ ] All tests pass on all PG versions
- [ ] `cargo test --all-features` passes
- [ ] CI green on main

**Review focus:**
- Verify all error codes are actually tested (not just mentioned)
- Check edge case coverage is comprehensive
- Confirm Postgres version matrix works correctly
- Ensure tests don't create test data that persists (cleanup)
- Look for flaky tests (timing issues, order dependencies)

**Agents:**
- Rust engineer (implement)
- DevOps engineer (verify CI matrix, test infrastructure)
- Code reviewer (apply 12-point checklist)

---

### feat/027 — ci-pipeline-hardening

**Title:** Finalize CI pipeline: enable test matrix, benchmarks, deny, cross-compilation checks

**Files created/modified:**
- Modify: `.github/workflows/ci.yml` (full implementation)
- Modify: `.github/workflows/release.yml` (full implementation)
- Modify: `deny.toml` (full implementation)

**Dependencies:** feat/026 (integration-test-hardening)

**Acceptance criteria:**
- [ ] `.github/workflows/ci.yml` complete and all jobs functional:
  - [ ] `check` job: `cargo fmt --check`, `cargo clippy -- -D warnings` on ubuntu-24.04
  - [ ] `test` job: matrix over PG versions [14, 15, 16, 17], `cargo test --all-features`
  - [ ] `deny` job: `cargo deny check` with advisories + licenses + bans enabled
  - [ ] `bench` job: `criterion` benchmarks on hot path (serialization, streaming), baseline persisted
  - [ ] `check-targets` job: `cross build --target x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (advisory)
- [ ] All jobs run in parallel (no inter-job dependencies)
- [ ] Gate rules on PR to main: check, test, deny must pass (bench is advisory unless hot-path change)
- [ ] `.github/workflows/release.yml` complete:
  - [ ] `validate-tag` job: verifies tag format (vX.Y.Z)
  - [ ] `build` job: builds on ubuntu, macOS for multiple targets
  - [ ] `docker` job: builds Docker image
  - [ ] `publish` job: publishes to crates.io (if in scope; defer if not)
  - [ ] `release` job: creates GitHub release with binaries
- [ ] `deny.toml` configured:
  - [ ] `[advisories]`: deny all known-vulnerable crates
  - [ ] `[licenses]`: allow Apache-2.0, MIT, BSD-*, ISC, Unicode-DFS-2016; deny others
  - [ ] `[bans]`: deny duplicate tokio/serde/serde_json; deny chrono, anyhow, eyre
  - [ ] `[sources]`: allow only crates.io
- [ ] Benchmark regression detection: 5% threshold enforced
- [ ] All CI checks pass on main
- [ ] PR gates work correctly (blocks merge on failure)
- [ ] `cargo test` passes locally
- [ ] `cargo deny check` passes locally

**Review focus:**
- Verify all CI jobs are functional (test on actual CI run, not just config syntax)
- Check matrix coverage (PG versions, platforms)
- Confirm gate rules are correct (required vs advisory)
- Verify deny.toml configuration is complete
- Ensure benchmark baseline is committed and regression detection works
- Look for missing security checks (e.g., supply chain hardening)

**Agents:**
- DevOps engineer (implement CI/CD)
- Security agent (review deny.toml and supply chain hardening)
- Code reviewer (apply 12-point checklist)

---

### feat/028 — docker-packaging

**Title:** Write Dockerfile, docker-compose for local dev, verify image builds and runs

**Files created/modified:**
- Create: `docker/Dockerfile` (multi-stage, distroless)
- Create: `docker-compose.yml` (pgmcp + Postgres for local dev)
- Create: `docker/entrypoint.sh` (startup script, optional)

**Dependencies:** feat/027 (ci-pipeline-hardening)

**Acceptance criteria:**
- [ ] `Dockerfile` is multi-stage:
  - [ ] Builder stage: Rust nightly for compile, builds release binary
  - [ ] Final stage: distroless `gcr.io/distroless/base-debian12`, copies binary only
- [ ] Dockerfile is small: final image < 50 MB
- [ ] Dockerfile runs binary as non-root user
- [ ] Health check configured: `health` tool endpoint
- [ ] `docker-compose.yml` includes:
  - [ ] `pgmcp` service: builds from Dockerfile, exposes ports
  - [ ] `postgres` service: official postgres:latest image with test database setup
  - [ ] Environment variables: pgmcp database URL points to postgres service
- [ ] Docker image builds cleanly: `docker build -t pgmcp .`
- [ ] Docker image runs: `docker run pgmcp` starts without errors
- [ ] Health check passes: `docker exec <container> curl http://localhost:8765/health` returns 200 (or similar)
- [ ] docker-compose up starts both services and they connect
- [ ] Docker image can be tested locally: `docker-compose up && curl localhost:8765/health`
- [ ] Integration test via Docker verified (manual or CI step)

**Review focus:**
- Verify Dockerfile is optimized (multi-stage, minimal final image)
- Check that security best practices are followed (non-root user)
- Confirm health check is configured correctly
- Verify docker-compose networking works
- Ensure build is reproducible
- Look for unneeded files in final image

**Agents:**
- DevOps engineer (implement Docker)
- Security agent (review Dockerfile security practices)
- Code reviewer (apply 12-point checklist)

---

### feat/029 — readme-contributing

**Title:** Write README and CONTRIBUTING documentation

**Files created/modified:**
- Create: `README.md` (project overview, quick start, tool reference)
- Create: `CONTRIBUTING.md` (development setup, branch model, review process)

**Dependencies:** feat/028 (docker-packaging)

**Acceptance criteria:**
- [ ] `README.md` includes:
  - [ ] Project overview: what pgmcp is, why you'd use it
  - [ ] Installation: build from source, Docker, download binary
  - [ ] Quick start: configure, run, example tool call
  - [ ] Configuration reference: all config keys with descriptions
  - [ ] Tool reference: all 15 tools with parameters and examples
  - [ ] Examples: JSON and CSV output, query with limit, explain, etc.
  - [ ] Contributing: link to CONTRIBUTING.md
  - [ ] License: MIT or Apache-2.0
- [ ] `CONTRIBUTING.md` includes:
  - [ ] Development setup: Rust toolchain, Postgres, running tests
  - [ ] Project structure: overview of src/ layout
  - [ ] Branch model: feature branches, naming convention (feat/NNN)
  - [ ] Review process: 9-step lifecycle, checklist, review criteria
  - [ ] Code quality standards: reference to spec Section 7
  - [ ] Running tests: `cargo test`, integration test setup
  - [ ] Benchmarking: when to benchmark, how to compare
  - [ ] Commit message format: example from spec Section 6.5
- [ ] Examples in README are executable (can be copy-pasted)
- [ ] Tool descriptions in README match tool_defs.rs
- [ ] All links are valid (no 404s)
- [ ] Prose is clear and jargon is minimal
- [ ] Code samples compile and run (or include disclaimer if not tested)

**Review focus:**
- This is the final branch; focus on clarity and completeness
- Verify all documentation is accurate and up-to-date
- Check that examples work (or are clearly marked as pseudo-code)
- Ensure CONTRIBUTING covers all essential setup steps
- Look for missing sections (e.g., release process)
- Verify tone is welcoming and inclusive

**Agents:**
- Rust engineer (implement)
- Code reviewer (apply 12-point checklist, focus on prose clarity)

---

## Summary and Next Steps

**All 29 branches are now defined.** Each branch includes:
1. Files created/modified
2. Dependencies on prior branches
3. Specific, testable acceptance criteria
4. Review focus areas
5. Agent assignments

**Before starting implementation:**
1. Verify this plan aligns with Eric's vision (spec Section 1)
2. Confirm agent availability and capacity
3. Establish CI infrastructure for early branches (feat/001-004)
4. Create tracking issue for each branch with acceptance criteria as checklist

**During implementation:**
1. Follow the 9-step branch lifecycle (PLAN, IMPLEMENT, SELF-TEST, REVIEW, etc.)
2. Implement branches sequentially; do not skip dependencies
3. Merge to main only when all acceptance criteria pass and review is complete
4. Monitor CI: main must always be green

**After MVP completion:**
1. Tag the release (v1.0.0 or similar)
2. Generate release notes from commit messages
3. Publish Docker image and binaries
4. Announce to users

---

**Document generated:** 2026-04-07  
**Spec version:** docs/specs/2026-04-07-pgmcp-mvp-design.md
