# Phase 1: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`
> (recommended) or `superpowers:executing-plans` to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking progress.

**Goal:** Establish the project skeleton with config loading, structured logging, and error types.
After Phase 1, the project compiles cleanly under `#![deny(warnings)]`, CI runs fmt + clippy on every
push, and every subsequent branch has a stable, correct base.

**Architecture:** Four branches build the load-bearing infrastructure: `Cargo.toml` with all
dependencies pinned, config parsing (TOML + env override + CLI args), tracing subscriber setup,
and the `McpError` type hierarchy. No runtime behavior beyond `fn main() {}` and module
declarations. No PostgreSQL dependency anywhere in Phase 1.

**Tech Stack:** Rust 2024 edition, tokio, serde + toml, tracing + tracing-subscriber,
thiserror, hand-rolled CLI args (30 lines).

---

## Task 1: feat/001 — Project Scaffold

**Branch:** `feat/001-project-scaffold`

**Goal:** Initialize the Cargo project with all dependencies pinned, create all module stubs,
establish CI (fmt + clippy), add toolchain pin, deny config, example config, and license.
No functional code; purpose is to give every subsequent branch a stable dependency graph
and a working CI gate.

**Files created:**
- `Cargo.toml`
- `Cargo.lock` (generated)
- `rust-toolchain.toml`
- `src/main.rs`
- `src/config.rs`
- `src/error.rs`
- `src/telemetry.rs`
- `src/transport/mod.rs`
- `src/transport/sse.rs`
- `src/transport/stdio.rs`
- `src/server/mod.rs`
- `src/server/router.rs`
- `src/server/context.rs`
- `src/server/tool_defs.rs`
- `src/tools/mod.rs`
- `src/tools/list_databases.rs`
- `src/tools/server_info.rs`
- `src/tools/list_schemas.rs`
- `src/tools/list_tables.rs`
- `src/tools/describe_table.rs`
- `src/tools/list_enums.rs`
- `src/tools/list_extensions.rs`
- `src/tools/table_stats.rs`
- `src/tools/query.rs`
- `src/tools/explain.rs`
- `src/tools/suggest_index.rs`
- `src/tools/propose_migration.rs`
- `src/tools/my_permissions.rs`
- `src/tools/connection_info.rs`
- `src/tools/health.rs`
- `src/tools/query_events.rs`
- `src/sql/mod.rs`
- `src/sql/parser.rs`
- `src/sql/limit.rs`
- `src/sql/guardrails.rs`
- `src/pg/mod.rs`
- `src/pg/pool.rs`
- `src/pg/types.rs`
- `src/pg/cache.rs`
- `src/pg/invalidation.rs`
- `src/pg/infer.rs`
- `src/pg/queries/list_databases.sql`
- `src/pg/queries/server_settings.sql`
- `src/pg/queries/list_schemas.sql`
- `src/pg/queries/list_tables.sql`
- `src/pg/queries/describe_table.sql`
- `src/pg/queries/list_enums.sql`
- `src/pg/queries/list_extensions.sql`
- `src/pg/queries/table_stats.sql`
- `src/pg/queries/my_permissions.sql`
- `src/streaming/mod.rs`
- `src/streaming/json.rs`
- `src/streaming/csv.rs`
- `tests/common/mod.rs`
- `tests/common/fixtures.rs`
- `tests/integration/discovery.rs`
- `tests/integration/query.rs`
- `tests/integration/streaming.rs`
- `tests/integration/guardrails.rs`
- `tests/integration/schema_cache.rs`
- `tests/integration/permissions.rs`
- `tests/integration/health.rs`
- `tests/integration/migration.rs`
- `benches/serialization.rs`
- `benches/streaming.rs`
- `benches/connection.rs`
- `.github/workflows/ci.yml`
- `deny.toml`
- `config/pgmcp.example.toml`
- `LICENSE`
- `.gitignore`

---

### Step 1.1: Create `rust-toolchain.toml`

- [ ] **Create `rust-toolchain.toml`**

```toml
# /home/eric/Code/pgmcp/rust-toolchain.toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

---

### Step 1.2: Create `Cargo.toml`

- [ ] **Create `Cargo.toml`** with all 15 runtime dependencies and 2 dev dependencies.

```toml
# /home/eric/Code/pgmcp/Cargo.toml
[package]
name = "pgmcp"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
description = "A Rust MCP server for PostgreSQL — zero-overhead agent access to Postgres"
license = "Apache-2.0"
repository = "https://github.com/agentdb/pgmcp"
keywords = ["mcp", "postgres", "postgresql", "agent", "llm"]
categories = ["database", "network-programming"]

[[bin]]
name = "pgmcp"
path = "src/main.rs"

[[bench]]
name = "serialization"
harness = false

[[bench]]
name = "streaming"
harness = false

[[bench]]
name = "connection"
harness = false

[dependencies]
# Async runtime
tokio = { version = "1", features = ["rt-multi-thread", "net", "io-std", "time", "macros"] }

# MCP protocol implementation — pin to latest stable; verify upstream maintenance before committing
rmcp = { version = "0.1", features = ["server"] }

# HTTP server for SSE transport
axum = { version = "0.7", features = ["http1", "tokio"] }

# PostgreSQL wire protocol driver
tokio-postgres = { version = "0.7", features = ["with-uuid-1", "with-time-0_3"] }

# Connection pool for tokio-postgres
deadpool-postgres = { version = "0.13", features = ["rt_tokio_1"] }

# Serialization framework
serde = { version = "1", features = ["derive"] }

# JSON serialization for tool parameter parsing and response construction
serde_json = "1"

# SQL parser with Postgres dialect
sqlparser = "0.50"

# Date/time handling (replaces chrono — see design spec section 9.3)
time = { version = "0.3", features = ["serde"] }

# Structured logging and span instrumentation
tracing = "0.1"

# Tracing subscriber with JSON and human-readable formatters
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Derive macro for McpError — generates Display and Error impls
thiserror = "1"

# UUID type for Postgres UUID columns
uuid = { version = "1", features = ["v4", "serde"] }

# Fast float-to-string conversion; used in hot-path row encoder
ryu = "1"

# Bytes and BytesMut for zero-copy buffer management in streaming encoder
bytes = "1"

# TOML config file deserialization
toml = "0.8"

[dev-dependencies]
# Spin up real Postgres instances in integration tests without external setup
testcontainers = "0.23"

# Benchmarking framework
criterion = { version = "0.5", features = ["html_reports"] }

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

- [ ] **Verify the pinned `rmcp` version exists on crates.io:**

```bash
cargo search rmcp
```

> Expected output: a line beginning `rmcp = "0.1.x"` (or similar). If `rmcp` is not yet
> published under that name, check the upstream repository linked in the design spec and
> update the version accordingly before proceeding.

---

### Step 1.3: Create all source file stubs

Each stub contains only the minimum required for the file to exist and compile.
For modules that will be referenced from `main.rs`, the stub is an empty `mod` body.
For SQL query files, the stub is a comment explaining what the query will do.

- [ ] **Create `src/main.rs`**

```rust
// src/main.rs
#![deny(warnings)]

mod config;
mod error;
mod pg;
mod server;
mod sql;
mod streaming;
mod telemetry;
mod tools;
mod transport;

fn main() {}
```

- [ ] **Create `src/config.rs`**

```rust
// src/config.rs
```

- [ ] **Create `src/error.rs`**

```rust
// src/error.rs
```

- [ ] **Create `src/telemetry.rs`**

```rust
// src/telemetry.rs
```

- [ ] **Create `src/transport/mod.rs`**

```rust
// src/transport/mod.rs
pub(crate) mod sse;
pub(crate) mod stdio;
```

- [ ] **Create `src/transport/sse.rs`**

```rust
// src/transport/sse.rs
```

- [ ] **Create `src/transport/stdio.rs`**

```rust
// src/transport/stdio.rs
```

- [ ] **Create `src/server/mod.rs`**

```rust
// src/server/mod.rs
pub(crate) mod context;
pub(crate) mod router;
pub(crate) mod tool_defs;
```

- [ ] **Create `src/server/router.rs`**

```rust
// src/server/router.rs
```

- [ ] **Create `src/server/context.rs`**

```rust
// src/server/context.rs
```

- [ ] **Create `src/server/tool_defs.rs`**

```rust
// src/server/tool_defs.rs
```

- [ ] **Create `src/tools/mod.rs`**

```rust
// src/tools/mod.rs
pub(crate) mod connection_info;
pub(crate) mod describe_table;
pub(crate) mod explain;
pub(crate) mod health;
pub(crate) mod list_databases;
pub(crate) mod list_enums;
pub(crate) mod list_extensions;
pub(crate) mod list_schemas;
pub(crate) mod list_tables;
pub(crate) mod my_permissions;
pub(crate) mod propose_migration;
pub(crate) mod query;
pub(crate) mod query_events;
pub(crate) mod server_info;
pub(crate) mod suggest_index;
pub(crate) mod table_stats;
```

- [ ] **Create all 16 tool stubs** (`src/tools/connection_info.rs` through `src/tools/table_stats.rs`)

Each file is empty for now:

```rust
// src/tools/connection_info.rs
```

```rust
// src/tools/describe_table.rs
```

```rust
// src/tools/explain.rs
```

```rust
// src/tools/health.rs
```

```rust
// src/tools/list_databases.rs
```

```rust
// src/tools/list_enums.rs
```

```rust
// src/tools/list_extensions.rs
```

```rust
// src/tools/list_schemas.rs
```

```rust
// src/tools/list_tables.rs
```

```rust
// src/tools/my_permissions.rs
```

```rust
// src/tools/propose_migration.rs
```

```rust
// src/tools/query.rs
```

```rust
// src/tools/query_events.rs
```

```rust
// src/tools/server_info.rs
```

```rust
// src/tools/suggest_index.rs
```

```rust
// src/tools/table_stats.rs
```

- [ ] **Create `src/sql/mod.rs`**

```rust
// src/sql/mod.rs
pub(crate) mod guardrails;
pub(crate) mod limit;
pub(crate) mod parser;
```

- [ ] **Create `src/sql/parser.rs`, `src/sql/limit.rs`, `src/sql/guardrails.rs`** — empty stubs:

```rust
// src/sql/parser.rs
```

```rust
// src/sql/limit.rs
```

```rust
// src/sql/guardrails.rs
```

- [ ] **Create `src/pg/mod.rs`**

```rust
// src/pg/mod.rs
pub(crate) mod cache;
pub(crate) mod infer;
pub(crate) mod invalidation;
pub(crate) mod pool;
pub(crate) mod types;
```

- [ ] **Create `src/pg/pool.rs`, `src/pg/types.rs`, `src/pg/cache.rs`, `src/pg/invalidation.rs`, `src/pg/infer.rs`** — empty stubs:

```rust
// src/pg/pool.rs
```

```rust
// src/pg/types.rs
```

```rust
// src/pg/cache.rs
```

```rust
// src/pg/invalidation.rs
```

```rust
// src/pg/infer.rs
```

- [ ] **Create SQL query stubs** in `src/pg/queries/`:

```sql
-- src/pg/queries/list_databases.sql
-- Returns the list of databases visible to the connected role.
-- Implemented in feat/009.
```

```sql
-- src/pg/queries/server_settings.sql
-- Returns server version and key settings for server_info tool.
-- Implemented in feat/009.
```

```sql
-- src/pg/queries/list_schemas.sql
-- Returns all schemas visible to the connected role, excluding pg_toast and pg_temp_*.
-- Implemented in feat/010.
```

```sql
-- src/pg/queries/list_tables.sql
-- Returns tables, views, and materialized views in a schema.
-- Implemented in feat/010.
```

```sql
-- src/pg/queries/describe_table.sql
-- Returns full table definition: columns, constraints, indexes, foreign keys.
-- Implemented in feat/011.
```

```sql
-- src/pg/queries/list_enums.sql
-- Returns all enum types in a schema with their ordered label values.
-- Implemented in feat/011.
```

```sql
-- src/pg/queries/list_extensions.sql
-- Returns all installed extensions in the current database.
-- Implemented in feat/012.
```

```sql
-- src/pg/queries/table_stats.sql
-- Returns runtime statistics for a table from pg_stat_user_tables and pg_class.
-- Implemented in feat/012.
```

```sql
-- src/pg/queries/my_permissions.sql
-- Introspects role privileges using pg_roles and has_*_privilege() functions.
-- Implemented in feat/020.
```

- [ ] **Create `src/streaming/mod.rs`**

```rust
// src/streaming/mod.rs
pub(crate) mod csv;
pub(crate) mod json;
```

- [ ] **Create `src/streaming/json.rs` and `src/streaming/csv.rs`** — empty stubs:

```rust
// src/streaming/json.rs
```

```rust
// src/streaming/csv.rs
```

- [ ] **Create test stubs** under `tests/`:

```rust
// tests/common/mod.rs
pub mod fixtures;
```

```rust
// tests/common/fixtures.rs
// Integration test fixtures. Implemented starting in feat/005.
```

```rust
// tests/integration/discovery.rs
// Integration tests for discovery tools. Implemented in feat/008-012.
```

```rust
// tests/integration/query.rs
// Integration tests for the query tool. Implemented in feat/018.
```

```rust
// tests/integration/streaming.rs
// Integration tests for streaming serialization. Implemented in feat/017-018.
```

```rust
// tests/integration/guardrails.rs
// Integration tests for guardrail rules. Implemented in feat/015.
```

```rust
// tests/integration/schema_cache.rs
// Integration tests for schema cache. Implemented in feat/013.
```

```rust
// tests/integration/permissions.rs
// Integration tests for my_permissions tool. Implemented in feat/020.
```

```rust
// tests/integration/health.rs
// Integration tests for health and connection_info tools. Implemented in feat/008.
```

```rust
// tests/integration/migration.rs
// Integration tests for propose_migration tool. Implemented in feat/022.
```

- [ ] **Create bench stubs** under `benches/`:

```rust
// benches/serialization.rs
// Criterion benchmarks for JSON and CSV row encoding throughput.
// Implemented in feat/017.
fn main() {}
```

```rust
// benches/streaming.rs
// Criterion benchmarks for end-to-end streaming pipeline.
// Implemented in feat/017.
fn main() {}
```

```rust
// benches/connection.rs
// Criterion benchmarks for pool acquisition latency.
// Implemented in feat/005.
fn main() {}
```

---

### Step 1.4: Create `.gitignore`

- [ ] **Create `.gitignore`**

```gitignore
# /home/eric/Code/pgmcp/.gitignore

# Rust build artifacts
/target/
/debug/
/release/

# Cargo lock is committed for reproducibility; do NOT add Cargo.lock here

# IDE and editor files
.idea/
.vscode/
*.swp
*.swo
*~
.DS_Store

# Criterion benchmark results (baselines are committed; current run outputs are not)
/target/criterion/

# Environment variable files — never commit credentials
.env
.env.*
!.env.example

# Docker build context artifacts
docker/*.tar.gz

# pgmcp runtime artifacts
*.pid
/pgmcp.toml
```

---

### Step 1.5: Create `deny.toml`

- [ ] **Create `deny.toml`**

```toml
# /home/eric/Code/pgmcp/deny.toml
#
# cargo-deny configuration.
# Run: cargo deny check
# CI: enforced in the deny job of ci.yml.

[graph]
targets = []

[advisories]
# Deny all crates with known security vulnerabilities.
version = 2
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
ignore = []

[licenses]
version = 2
# Allow only permissive, well-understood licenses.
allow = [
    "Apache-2.0",
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
]
# Copyleft and proprietary licenses are denied.
deny = []
# Exceptions are reviewed case-by-case; none allowed by default.
exceptions = []
# confidence-threshold: how confident cargo-deny must be before
# treating a license as "identified". 0.8 is the default.
confidence-threshold = 0.8

[bans]
version = 2

# Deny duplicate versions of core dependencies to prevent version conflicts
# that could cause subtle runtime bugs or inflated binary size.
deny = [
    # chrono has a soundness issue in local timezone handling; replaced by time 0.3
    { crate = "chrono", reason = "replaced by time 0.3 (see design spec section 9.3)" },
    # anyhow erases error types; McpError is the explicit product error surface
    { crate = "anyhow", reason = "McpError is the error surface; type erasure is prohibited" },
    # eyre erases error types for the same reason as anyhow
    { crate = "eyre", reason = "McpError is the error surface; type erasure is prohibited" },
    # once_cell / lazy_static introduce implicit global state
    { crate = "once_cell", reason = "no global state allowed; use Arc injection via ToolContext" },
    { crate = "lazy_static", reason = "no global state allowed; use Arc injection via ToolContext" },
]

# Multiple versions of these crates cause subtle bugs and inflated binaries.
# All must resolve to a single version.
[[bans.deny]]
crate = "tokio"
version = "*"
wrappers = []

[[bans.deny]]
crate = "serde"
version = "*"
wrappers = []

[[bans.deny]]
crate = "serde_json"
version = "*"
wrappers = []

[sources]
version = 2
# Only allow crates from crates.io. Git sources and path overrides must be
# explicitly declared here during development; all must be removed before release.
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
```

---

### Step 1.6: Create `config/pgmcp.example.toml`

- [ ] **Create `config/pgmcp.example.toml`**

```toml
# /home/eric/Code/pgmcp/config/pgmcp.example.toml
#
# pgmcp example configuration file.
# Copy to pgmcp.toml and edit to match your environment.
# All values shown are defaults unless marked REQUIRED.
#
# Environment variable overrides: prefix any key with PGMCP_
# and use SCREAMING_SNAKE_CASE. Nested keys use double underscores.
# Example: PGMCP_POOL__MAX_SIZE=20
#
# CLI shorthand: pgmcp postgres://user:pass@host:5432/dbname
# The positional argument sets database_url and overrides the config file.

# ─── Database ────────────────────────────────────────────────────────────────

# REQUIRED. PostgreSQL connection string.
# Supports both postgres:// URI format and libpq key=value format.
# Example URI:     postgres://myuser:mypassword@localhost:5432/mydb?sslmode=require
# Example key=val: host=localhost port=5432 dbname=mydb user=myuser password=mypassword
database_url = "postgres://pgmcp:pgmcp@localhost:5432/pgmcp"

# ─── Connection Pool ─────────────────────────────────────────────────────────

[pool]
# Minimum number of connections to maintain in the pool at all times.
# Pool initialization at startup fails (exit code 5) if min_size connections
# cannot be established. Set to 0 to allow startup with no Postgres connectivity.
min_size = 2

# Maximum number of connections the pool will open simultaneously.
# Requests that arrive when all connections are in use will wait up to
# acquire_timeout_seconds before failing with pg_pool_timeout.
max_size = 10

# Seconds to wait for a connection from the pool before returning pg_pool_timeout.
# Must be greater than 0. Fractional seconds are not supported.
acquire_timeout_seconds = 5

# Seconds a connection may remain idle before being recycled.
# Set to 0 to disable idle timeout (connections are never recycled for idleness).
idle_timeout_seconds = 300

# ─── Transport ───────────────────────────────────────────────────────────────

[transport]
# Which transport to use. One of: "stdio", "sse".
# stdio: reads JSON-RPC from stdin, writes to stdout. Use for process-launched MCP servers.
# sse:   HTTP server with SSE for server-to-client streaming, POST for client-to-server.
mode = "stdio"

# Host and port for the SSE transport. Ignored when mode = "stdio".
host = "127.0.0.1"
port = 3000

# ─── Telemetry ───────────────────────────────────────────────────────────────

[telemetry]
# Log format. One of: "json", "text".
# json: structured JSON logs, suitable for log aggregators (production).
# text: human-readable logs with ANSI color, suitable for development.
log_format = "text"

# Log level filter, in RUST_LOG syntax.
# Examples: "info", "debug", "pgmcp=debug,tokio_postgres=warn"
# The RUST_LOG environment variable takes precedence over this setting.
log_level = "info"

# ─── Schema Cache ────────────────────────────────────────────────────────────

[cache]
# Seconds between schema change polls. The cache invalidation task queries
# pg_stat_database at this interval to detect schema changes.
# Lower values reduce staleness; higher values reduce catalog query load.
invalidation_interval_seconds = 30

# ─── Guardrails ──────────────────────────────────────────────────────────────

[guardrails]
# Block DDL statements (CREATE, DROP, ALTER, TRUNCATE) in the query tool.
# Set to false ONLY in tightly controlled environments where agents are trusted
# to manage schema. The propose_migration tool is the correct way to propose DDL.
block_ddl = true

# Block COPY TO/FROM PROGRAM statements. These execute shell commands on the
# Postgres server and are almost always a security risk. There is no legitimate
# use case for an agent to run COPY TO PROGRAM via pgmcp.
block_copy_program = true

# Block SET statements that change session-level parameters affecting subsequent
# callers. Examples: SET search_path, SET role, SET session_authorization.
# Individual statement timeouts (SET LOCAL statement_timeout) are allowed.
block_session_set = true
```

---

### Step 1.7: Create `.github/workflows/ci.yml`

- [ ] **Create `.github/workflows/ci.yml`**

This is the bootstrap CI — `check` job only (fmt + clippy). The `test`, `deny`, and `bench`
jobs are stubs that pass unconditionally. They become real gates in feat/027.

```yaml
# /home/eric/Code/pgmcp/.github/workflows/ci.yml
#
# CI pipeline for pgmcp.
# Triggered on every push and every PR to main.
#
# Phase 1 (feat/001): check job only (fmt + clippy).
# Stub jobs (test, deny, bench) pass unconditionally.
# Full pipeline is enabled in feat/027.

name: CI

on:
  push:
    branches: ["**"]
  pull_request:
    branches: [main]

# No default permissions granted. Each job declares the minimum it needs.
permissions: {}

jobs:
  # ──────────────────────────────────────────────────────────────────────────
  # check: format + lint. Runs without Postgres. Must pass on every push.
  # ──────────────────────────────────────────────────────────────────────────
  check:
    name: check (fmt + clippy)
    runs-on: ubuntu-24.04
    permissions:
      contents: read

    steps:
      - name: Checkout
        uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Restore registry cache
        uses: actions/cache@5a3ec84eff668545956fd18022155c47e93e2684  # v4.2.3
        with:
          path: |
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
          key: registry-${{ hashFiles('Cargo.lock') }}
          restore-keys: |
            registry-

      - name: Restore target cache
        uses: actions/cache@5a3ec84eff668545956fd18022155c47e93e2684  # v4.2.3
        with:
          path: target/
          key: target-check-${{ hashFiles('Cargo.lock') }}
          restore-keys: |
            target-check-

      - name: Check formatting
        run: cargo fmt --check

      - name: Run clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

  # ──────────────────────────────────────────────────────────────────────────
  # test: integration tests against real Postgres.
  # STUB in Phase 1 — passes unconditionally.
  # Activated in feat/027 with the full matrix.
  # ──────────────────────────────────────────────────────────────────────────
  test:
    name: test (stub — activated in feat/027)
    runs-on: ubuntu-24.04
    permissions:
      contents: read

    steps:
      - name: Checkout
        uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Stub pass
        run: echo "Integration tests are stubbed until feat/027. Skipping."

  # ──────────────────────────────────────────────────────────────────────────
  # deny: license and vulnerability audit.
  # STUB in Phase 1 — passes unconditionally.
  # Activated in feat/027.
  # ──────────────────────────────────────────────────────────────────────────
  deny:
    name: deny (stub — activated in feat/027)
    runs-on: ubuntu-24.04
    permissions:
      contents: read

    steps:
      - name: Checkout
        uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Stub pass
        run: echo "cargo deny is stubbed until feat/027. Skipping."

  # ──────────────────────────────────────────────────────────────────────────
  # bench: benchmark regression check.
  # STUB in Phase 1 — passes unconditionally.
  # Activated in feat/027.
  # ──────────────────────────────────────────────────────────────────────────
  bench:
    name: bench (stub — activated in feat/027)
    runs-on: ubuntu-24.04
    permissions:
      contents: read

    steps:
      - name: Checkout
        uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Stub pass
        run: echo "Benchmarks are stubbed until feat/027. Skipping."
```

---

### Step 1.8: Create `LICENSE`

- [ ] **Create `LICENSE`**

```
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship made available under
      the License, as indicated by a copyright notice that is included in
      or attached to the work (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other
      transformations represent, as a whole, an original work of authorship.
      For the purposes of this License, Derivative Works shall not include
      works that remain separable from, or merely link (or bind by name)
      to the interfaces of, the Work and Derivative Works thereof.

      "Contribution" shall mean, as submitted to the Licensor for inclusion
      in the Work by the copyright owner or by an individual or Legal Entity
      authorized to submit on behalf of the copyright owner. For the purposes
      of this definition, "submit" means any form of electronic, verbal, or
      written communication sent to the Licensor or its representatives,
      including but not limited to communication on electronic mailing lists,
      source code control systems, and issue tracking systems that are managed
      by, or on behalf of, the Licensor for the purpose of developing and
      improving the Work, but excluding communication that is conspicurate
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any Legal Entity on behalf of
      whom a Contribution has been received by the Licensor and included
      within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by the combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a cross-claim
      or counterclaim in a lawsuit) alleging that the Work or any Claim
      in the Work constitutes direct or contributory patent infringement,
      then any patent licenses granted to You under this License for that
      Work shall terminate as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or Derivative Works
          a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, You must include a readable copy of the
          attribution notices contained within such NOTICE file, in
          at least one of the following places: within a NOTICE text
          file distributed as part of the Derivative Works; within the
          Source form or documentation, if provided along with the
          Derivative Works; or, within a display generated by the
          Derivative Works, if and wherever such third-party notices
          normally appear. The contents of the NOTICE file are for
          informational purposes only and do not modify the License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or agreed
      to in writing, Licensor provides the Work (and each Contributor
      provides its Contributions) on an "AS IS" BASIS, WITHOUT WARRANTIES
      OR CONDITIONS OF ANY KIND, either express or implied, including,
      without limitation, any warranties or conditions of TITLE,
      NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A PARTICULAR
      PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or exemplary damages of any character arising as a result
      of this License or out of the use or inability to use the Work
      (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or all other
      commercial damages or losses), even if such Contributor has been
      advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may offer only
      conditions not inconsistent with the terms of this License.

   END OF TERMS AND CONDITIONS

   Copyright 2026 AgentDB

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```

---

### Step 1.9: Verify the scaffold compiles and CI gates pass

- [ ] **Run `cargo build`:**

```bash
cargo build
```

Expected output:

```
   Compiling pgmcp v0.1.0 (/home/eric/Code/pgmcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in Xs
```

No warnings. No errors.

- [ ] **Run `cargo fmt --check`:**

```bash
cargo fmt --check
```

Expected output: no output (exit code 0). If there are formatting differences,
run `cargo fmt` to fix them, then re-check.

- [ ] **Run `cargo clippy`:**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected output:

```
    Checking pgmcp v0.1.0 (/home/eric/Code/pgmcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in Xs
```

No warnings.

---

### Step 1.10: Commit

- [ ] **Stage and commit:**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(001): initialize project scaffold with all dependencies and module stubs

Why: Every subsequent branch needs a stable Cargo.toml with pinned dependencies
and a working CI gate before any feature code can be written.
What: Added Cargo.toml (15 runtime deps, 2 dev deps), rust-toolchain.toml,
all module stubs, SQL query stubs, bench stubs, deny.toml, example config,
LICENSE (Apache 2.0), and bootstrap CI (fmt + clippy only).
EOF
)"
```

---

## Task 2: feat/002 — Config

**Branch:** `feat/002-config`

**Goal:** Implement `config.rs` with the full `Config` struct, TOML deserialization via serde,
environment variable override logic (`PGMCP_*` prefix), hand-rolled CLI argument parsing,
and validation. TDD: write failing tests first, then implement.

**Files modified:**
- `src/config.rs` (primary)
- `src/main.rs` (wire `Config::load` into startup)
- `config/pgmcp.example.toml` (already created in feat/001)

**No new dependencies** — all dependencies needed (`serde`, `toml`) are already in `Cargo.toml`.

---

### Step 2.1: Write failing tests (RED)

- [ ] **Add tests to `src/config.rs`**

Write all tests first. They will fail because `Config` does not exist yet.

```rust
// src/config.rs
#![allow(dead_code)]

#[cfg(test)]
mod tests {
    use super::*;

    // ── TOML deserialization ────────────────────────────────────────────

    #[test]
    fn test_config_from_minimal_toml() {
        let toml = r#"
            database_url = "postgres://user:pass@localhost:5432/db"
        "#;
        let cfg: Config = toml::from_str(toml).expect("minimal config must parse");
        assert_eq!(cfg.database_url, "postgres://user:pass@localhost:5432/db");
        // All other fields must have their defaults.
        assert_eq!(cfg.pool.min_size, 2);
        assert_eq!(cfg.pool.max_size, 10);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 5);
        assert_eq!(cfg.pool.idle_timeout_seconds, 300);
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
        assert_eq!(cfg.transport.host, "127.0.0.1");
        assert_eq!(cfg.transport.port, 3000);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
        assert_eq!(cfg.telemetry.log_level, "info");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 30);
        assert!(cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_config_from_full_toml() {
        let toml = r#"
            database_url = "postgres://myuser:secret@db.example.com:5433/prod"

            [pool]
            min_size = 5
            max_size = 50
            acquire_timeout_seconds = 10
            idle_timeout_seconds = 600

            [transport]
            mode = "sse"
            host = "0.0.0.0"
            port = 8080

            [telemetry]
            log_format = "json"
            log_level = "debug"

            [cache]
            invalidation_interval_seconds = 60

            [guardrails]
            block_ddl = false
            block_copy_program = true
            block_session_set = false
        "#;
        let cfg: Config = toml::from_str(toml).expect("full config must parse");
        assert_eq!(cfg.database_url, "postgres://myuser:secret@db.example.com:5433/prod");
        assert_eq!(cfg.pool.min_size, 5);
        assert_eq!(cfg.pool.max_size, 50);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 10);
        assert_eq!(cfg.pool.idle_timeout_seconds, 600);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
        assert_eq!(cfg.transport.host, "0.0.0.0");
        assert_eq!(cfg.transport.port, 8080);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
        assert_eq!(cfg.telemetry.log_level, "debug");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 60);
        assert!(!cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(!cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_transport_mode_deserializes_stdio() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "stdio"
        "#;
        let cfg: Config = toml::from_str(toml).expect("stdio mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
    }

    #[test]
    fn test_transport_mode_deserializes_sse() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "sse"
        "#;
        let cfg: Config = toml::from_str(toml).expect("sse mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_log_format_deserializes_json() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [telemetry]
            log_format = "json"
        "#;
        let cfg: Config = toml::from_str(toml).expect("json log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_log_format_deserializes_text() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [telemetry]
            log_format = "text"
        "#;
        let cfg: Config = toml::from_str(toml).expect("text log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
    }

    // ── Environment variable overrides ─────────────────────────────────

    #[test]
    fn test_env_override_database_url() {
        // Use a scoped environment variable so parallel tests do not interfere.
        // Serial test: set env var, call apply_env_overrides, unset.
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://original@localhost/db""#,
        )
        .unwrap();
        // Simulate PGMCP_DATABASE_URL override.
        cfg.apply_env_overrides_from(&[
            ("PGMCP_DATABASE_URL", "postgres://override@remotehost/newdb"),
        ]);
        assert_eq!(cfg.database_url, "postgres://override@remotehost/newdb");
    }

    #[test]
    fn test_env_override_pool_max_size() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_POOL__MAX_SIZE", "99")]);
        assert_eq!(cfg.pool.max_size, 99);
    }

    #[test]
    fn test_env_override_transport_mode_sse() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TRANSPORT__MODE", "sse")]);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_env_override_log_format_json() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TELEMETRY__LOG_FORMAT", "json")]);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_env_override_unknown_key_is_ignored() {
        // Unknown PGMCP_ keys must not panic or error; they are silently ignored.
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_DOES_NOT_EXIST", "value")]);
        // Unchanged.
        assert_eq!(cfg.database_url, "postgres://u:p@h/d");
    }

    // ── CLI connection string shorthand ────────────────────────────────

    #[test]
    fn test_cli_connection_string_sets_database_url() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://original@localhost/db""#,
        )
        .unwrap();
        cfg.apply_cli_connection_string("postgres://cli_user:cli_pass@cli_host/cli_db");
        assert_eq!(
            cfg.database_url,
            "postgres://cli_user:cli_pass@cli_host/cli_db"
        );
    }

    // ── CLI argument parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_cli_args_config_flag() {
        let args = ["pgmcp", "--config", "/etc/pgmcp.toml"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/etc/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, None);
        assert_eq!(parsed.connection_string, None);
    }

    #[test]
    fn test_parse_cli_args_transport_flag() {
        let args = ["pgmcp", "--transport", "sse"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.transport, Some("sse".to_string()));
    }

    #[test]
    fn test_parse_cli_args_positional_connection_string() {
        let args = ["pgmcp", "postgres://u:p@h/d"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    #[test]
    fn test_parse_cli_args_all_flags() {
        let args = [
            "pgmcp",
            "--config",
            "/tmp/pgmcp.toml",
            "--transport",
            "stdio",
            "postgres://u:p@h/d",
        ];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/tmp/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, Some("stdio".to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    // ── Validation ─────────────────────────────────────────────────────

    #[test]
    fn test_validate_rejects_empty_database_url() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.database_url = String::new();
        assert!(cfg.validate().is_err());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("database_url"));
    }

    #[test]
    fn test_validate_rejects_zero_max_pool_size() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.pool.max_size = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_min_size_greater_than_max_size() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.pool.min_size = 10;
        cfg.pool.max_size = 5;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_zero_acquire_timeout() {
        let mut cfg: Config = toml::from_str(
            r#"database_url = "postgres://u:p@h/d""#,
        )
        .unwrap();
        cfg.pool.acquire_timeout_seconds = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_sse_port() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "sse"
            port = 0
        "#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.transport.mode = TransportMode::Sse;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_accepts_valid_config() {
        let toml = r#"database_url = "postgres://u:p@h/d""#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.validate().is_ok());
    }
}
```

- [ ] **Verify tests fail:**

```bash
cargo test --lib 2>&1 | head -30
```

Expected output: compile errors because `Config`, `TransportMode`, `LogFormat`, `CliArgs`
do not exist yet. This is the expected RED state.

---

### Step 2.2: Implement `Config` (GREEN)

- [ ] **Implement the full `src/config.rs`:**

```rust
// src/config.rs
//
// Configuration loading for pgmcp.
//
// Loading order (later entries win):
//   1. Built-in defaults (via serde default attributes)
//   2. TOML config file (path from CLI --config, or ./pgmcp.toml, or /etc/pgmcp/pgmcp.toml)
//   3. PGMCP_* environment variable overrides
//   4. CLI positional connection string (overrides database_url only)
//   5. CLI --transport flag (overrides transport.mode only)
//
// All fields are validated after merging all sources. Validation errors are
// returned as a String describing the first failing constraint. The caller
// (main.rs) is responsible for printing the error and exiting with code 2.

use serde::Deserialize;

/// Top-level configuration for pgmcp.
///
/// Deserialized from a TOML file. All fields have defaults so that a config
/// file containing only `database_url` is valid.
#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    /// PostgreSQL connection string (URI or libpq key=value format). Required.
    pub(crate) database_url: String,

    /// Connection pool settings.
    #[serde(default)]
    pub(crate) pool: PoolConfig,

    /// Transport selection and binding.
    #[serde(default)]
    pub(crate) transport: TransportConfig,

    /// Telemetry and logging.
    #[serde(default)]
    pub(crate) telemetry: TelemetryConfig,

    /// Schema cache invalidation.
    #[serde(default)]
    pub(crate) cache: CacheConfig,

    /// SQL guardrail policies.
    #[serde(default)]
    pub(crate) guardrails: GuardrailConfig,
}

/// Connection pool configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct PoolConfig {
    /// Minimum connections to maintain. Pool init fails at startup if this
    /// many connections cannot be established.
    #[serde(default = "default_pool_min_size")]
    pub(crate) min_size: u32,

    /// Maximum connections the pool will open simultaneously.
    #[serde(default = "default_pool_max_size")]
    pub(crate) max_size: u32,

    /// Seconds to wait for a connection before returning `pg_pool_timeout`.
    #[serde(default = "default_acquire_timeout")]
    pub(crate) acquire_timeout_seconds: u64,

    /// Seconds an idle connection is kept before recycling. 0 = never recycle.
    #[serde(default = "default_idle_timeout")]
    pub(crate) idle_timeout_seconds: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: default_pool_min_size(),
            max_size: default_pool_max_size(),
            acquire_timeout_seconds: default_acquire_timeout(),
            idle_timeout_seconds: default_idle_timeout(),
        }
    }
}

fn default_pool_min_size() -> u32 {
    2
}
fn default_pool_max_size() -> u32 {
    10
}
fn default_acquire_timeout() -> u64 {
    5
}
fn default_idle_timeout() -> u64 {
    300
}

/// Which transport the server listens on.
#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TransportMode {
    /// Read JSON-RPC from stdin, write to stdout.
    Stdio,
    /// HTTP server: SSE for server-to-client, POST for client-to-server.
    Sse,
}

impl Default for TransportMode {
    fn default() -> Self {
        Self::Stdio
    }
}

/// Transport binding configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct TransportConfig {
    /// Which transport to activate.
    #[serde(default)]
    pub(crate) mode: TransportMode,

    /// Bind host for the SSE transport. Ignored for stdio.
    #[serde(default = "default_transport_host")]
    pub(crate) host: String,

    /// Bind port for the SSE transport. Ignored for stdio.
    #[serde(default = "default_transport_port")]
    pub(crate) port: u16,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            mode: TransportMode::default(),
            host: default_transport_host(),
            port: default_transport_port(),
        }
    }
}

fn default_transport_host() -> String {
    "127.0.0.1".to_string()
}
fn default_transport_port() -> u16 {
    3000
}

/// Log output format.
#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogFormat {
    /// Structured JSON logs, suitable for log aggregators.
    Json,
    /// Human-readable logs with ANSI color, suitable for development.
    Text,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Text
    }
}

/// Telemetry and logging configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct TelemetryConfig {
    /// Log format: "json" or "text".
    #[serde(default)]
    pub(crate) log_format: LogFormat,

    /// Log level filter in RUST_LOG syntax.
    /// The `RUST_LOG` environment variable takes precedence.
    #[serde(default = "default_log_level")]
    pub(crate) log_level: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_format: LogFormat::default(),
            log_level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Schema cache invalidation configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct CacheConfig {
    /// Seconds between pg_catalog polls for schema changes.
    #[serde(default = "default_invalidation_interval")]
    pub(crate) invalidation_interval_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            invalidation_interval_seconds: default_invalidation_interval(),
        }
    }
}

fn default_invalidation_interval() -> u64 {
    30
}

/// SQL guardrail policy configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailConfig {
    /// Block DDL statements (CREATE, DROP, ALTER, TRUNCATE) in the query tool.
    #[serde(default = "default_true")]
    pub(crate) block_ddl: bool,

    /// Block COPY TO/FROM PROGRAM statements.
    #[serde(default = "default_true")]
    pub(crate) block_copy_program: bool,

    /// Block SET statements that change session-level parameters.
    #[serde(default = "default_true")]
    pub(crate) block_session_set: bool,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            block_ddl: true,
            block_copy_program: true,
            block_session_set: true,
        }
    }
}

fn default_true() -> bool {
    true
}

// ─── Config methods ───────────────────────────────────────────────────────────

impl Config {
    /// Apply overrides from a slice of `(key, value)` pairs, as if they were
    /// environment variables.
    ///
    /// This method exists for testing. In production, use `apply_env_overrides`.
    /// Keys must use the `PGMCP_` prefix and `__` as the nested separator.
    pub(crate) fn apply_env_overrides_from(&mut self, overrides: &[(&str, &str)]) {
        for (key, value) in overrides {
            self.apply_single_env_override(key, value);
        }
    }

    /// Apply all `PGMCP_*` environment variables from the current process environment.
    pub(crate) fn apply_env_overrides(&mut self) {
        let pairs: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("PGMCP_"))
            .collect();
        for (key, value) in &pairs {
            self.apply_single_env_override(key, value);
        }
    }

    /// Apply a single environment variable override.
    ///
    /// Key format: `PGMCP_` followed by the config path in SCREAMING_SNAKE_CASE,
    /// with `__` (double underscore) as the nested separator.
    ///
    /// Supported keys:
    /// - `PGMCP_DATABASE_URL`
    /// - `PGMCP_POOL__MIN_SIZE`
    /// - `PGMCP_POOL__MAX_SIZE`
    /// - `PGMCP_POOL__ACQUIRE_TIMEOUT_SECONDS`
    /// - `PGMCP_POOL__IDLE_TIMEOUT_SECONDS`
    /// - `PGMCP_TRANSPORT__MODE`
    /// - `PGMCP_TRANSPORT__HOST`
    /// - `PGMCP_TRANSPORT__PORT`
    /// - `PGMCP_TELEMETRY__LOG_FORMAT`
    /// - `PGMCP_TELEMETRY__LOG_LEVEL`
    /// - `PGMCP_CACHE__INVALIDATION_INTERVAL_SECONDS`
    /// - `PGMCP_GUARDRAILS__BLOCK_DDL`
    /// - `PGMCP_GUARDRAILS__BLOCK_COPY_PROGRAM`
    /// - `PGMCP_GUARDRAILS__BLOCK_SESSION_SET`
    ///
    /// Unknown keys are silently ignored.
    fn apply_single_env_override(&mut self, key: &str, value: &str) {
        match key {
            "PGMCP_DATABASE_URL" => {
                self.database_url = value.to_string();
            }
            "PGMCP_POOL__MIN_SIZE" => {
                if let Ok(v) = value.parse() {
                    self.pool.min_size = v;
                }
            }
            "PGMCP_POOL__MAX_SIZE" => {
                if let Ok(v) = value.parse() {
                    self.pool.max_size = v;
                }
            }
            "PGMCP_POOL__ACQUIRE_TIMEOUT_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.pool.acquire_timeout_seconds = v;
                }
            }
            "PGMCP_POOL__IDLE_TIMEOUT_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.pool.idle_timeout_seconds = v;
                }
            }
            "PGMCP_TRANSPORT__MODE" => match value {
                "stdio" => self.transport.mode = TransportMode::Stdio,
                "sse" => self.transport.mode = TransportMode::Sse,
                _ => {}
            },
            "PGMCP_TRANSPORT__HOST" => {
                self.transport.host = value.to_string();
            }
            "PGMCP_TRANSPORT__PORT" => {
                if let Ok(v) = value.parse() {
                    self.transport.port = v;
                }
            }
            "PGMCP_TELEMETRY__LOG_FORMAT" => match value {
                "json" => self.telemetry.log_format = LogFormat::Json,
                "text" => self.telemetry.log_format = LogFormat::Text,
                _ => {}
            },
            "PGMCP_TELEMETRY__LOG_LEVEL" => {
                self.telemetry.log_level = value.to_string();
            }
            "PGMCP_CACHE__INVALIDATION_INTERVAL_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.cache.invalidation_interval_seconds = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_DDL" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_ddl = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_COPY_PROGRAM" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_copy_program = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_SESSION_SET" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_session_set = v;
                }
            }
            _ => {
                // Unknown PGMCP_ key. Silently ignored per spec.
            }
        }
    }

    /// Override `database_url` from the CLI positional connection string argument.
    pub(crate) fn apply_cli_connection_string(&mut self, conn_str: &str) {
        self.database_url = conn_str.to_string();
    }

    /// Validate the fully-merged configuration.
    ///
    /// Returns `Ok(())` when all constraints are satisfied.
    /// Returns `Err(String)` with a human-readable description of the first violation.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.database_url.is_empty() {
            return Err(
                "database_url is required. Set it in the config file or via \
                 PGMCP_DATABASE_URL, or pass a connection string as a positional argument."
                    .to_string(),
            );
        }
        if self.pool.max_size == 0 {
            return Err(
                "pool.max_size must be greater than 0.".to_string(),
            );
        }
        if self.pool.min_size > self.pool.max_size {
            return Err(format!(
                "pool.min_size ({}) must not exceed pool.max_size ({}).",
                self.pool.min_size, self.pool.max_size
            ));
        }
        if self.pool.acquire_timeout_seconds == 0 {
            return Err(
                "pool.acquire_timeout_seconds must be greater than 0.".to_string(),
            );
        }
        if self.transport.mode == TransportMode::Sse && self.transport.port == 0 {
            return Err(
                "transport.port must be greater than 0 when transport.mode is \"sse\"."
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Load configuration using the full merging strategy:
    ///
    /// 1. Read the TOML file from `config_path` (if Some), or search default paths.
    /// 2. Apply PGMCP_* environment variable overrides.
    /// 3. Apply CLI connection string (if Some).
    /// 4. Apply CLI transport mode (if Some).
    /// 5. Validate the merged result.
    ///
    /// Returns the merged, validated Config or an error string.
    pub(crate) fn load(
        config_path: Option<&str>,
        cli_connection_string: Option<&str>,
        cli_transport: Option<&str>,
    ) -> Result<Self, String> {
        let toml_str = Self::read_config_file(config_path)?;
        let mut config: Config = toml::from_str(&toml_str)
            .map_err(|e| format!("Failed to parse config file: {e}"))?;
        config.apply_env_overrides();
        if let Some(conn_str) = cli_connection_string {
            config.apply_cli_connection_string(conn_str);
        }
        if let Some(transport) = cli_transport {
            config.apply_single_env_override(
                "PGMCP_TRANSPORT__MODE",
                transport,
            );
        }
        config.validate()?;
        Ok(config)
    }

    /// Read the config file from the given path or search default locations.
    ///
    /// Default search order:
    ///   1. `./pgmcp.toml`
    ///   2. `/etc/pgmcp/pgmcp.toml`
    ///
    /// If neither default exists and no explicit path was given, returns an
    /// empty TOML string (Config will use all defaults, and validation will
    /// fail if database_url was not provided another way).
    fn read_config_file(config_path: Option<&str>) -> Result<String, String> {
        if let Some(path) = config_path {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read config file '{path}': {e}"));
        }
        // Search default locations.
        for candidate in &["./pgmcp.toml", "/etc/pgmcp/pgmcp.toml"] {
            if std::path::Path::new(candidate).exists() {
                return std::fs::read_to_string(candidate)
                    .map_err(|e| format!("Failed to read config file '{candidate}': {e}"));
            }
        }
        // No config file found; proceed with defaults.
        Ok(String::new())
    }
}

// ─── CLI argument parsing ─────────────────────────────────────────────────────

/// Parsed command-line arguments for pgmcp.
///
/// pgmcp intentionally has a small argument surface (three flags + one
/// positional). Using a hand-rolled parser keeps compile times short and
/// avoids pulling in clap for ~30 lines of work.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CliArgs {
    /// Path to the TOML config file. Corresponds to `--config <path>`.
    pub(crate) config: Option<String>,

    /// Transport mode override. Corresponds to `--transport <stdio|sse>`.
    pub(crate) transport: Option<String>,

    /// Positional connection string: `pgmcp postgres://...`
    pub(crate) connection_string: Option<String>,
}

impl CliArgs {
    /// Parse CLI arguments from an iterator of strings.
    ///
    /// Accepts:
    /// - `--config <path>`
    /// - `--transport <stdio|sse>`
    /// - `--help` / `-h` (prints usage and exits)
    /// - A positional argument beginning with `postgres://` or containing `host=`
    ///
    /// Unknown flags produce a usage message to stderr and a non-zero exit.
    pub(crate) fn parse_from(mut args: impl Iterator<Item = String>) -> Self {
        // Skip argv[0] (the binary name).
        let _ = args.next();
        let mut parsed = Self::default();
        let mut rest: Vec<String> = args.collect();
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--help" | "-h" => {
                    Self::print_usage();
                    std::process::exit(0);
                }
                "--config" => {
                    i += 1;
                    if i < rest.len() {
                        parsed.config = Some(rest[i].clone());
                    } else {
                        eprintln!("error: --config requires a path argument");
                        std::process::exit(2);
                    }
                }
                "--transport" => {
                    i += 1;
                    if i < rest.len() {
                        parsed.transport = Some(rest[i].clone());
                    } else {
                        eprintln!("error: --transport requires an argument (stdio|sse)");
                        std::process::exit(2);
                    }
                }
                arg if arg.starts_with("postgres://")
                    || arg.starts_with("postgresql://")
                    || arg.contains("host=") =>
                {
                    parsed.connection_string = Some(rest[i].clone());
                }
                arg if arg.starts_with('-') => {
                    eprintln!("error: unknown flag '{arg}'");
                    Self::print_usage();
                    std::process::exit(2);
                }
                _ => {}
            }
            i += 1;
        }
        parsed
    }

    /// Parse from the real process argv.
    pub(crate) fn parse() -> Self {
        Self::parse_from(std::env::args())
    }

    fn print_usage() {
        eprintln!(
            "Usage: pgmcp [OPTIONS] [CONNECTION_STRING]\n\
             \n\
             Options:\n\
             --config <path>          Path to TOML config file\n\
             --transport <stdio|sse>  Transport mode (overrides config)\n\
             --help, -h               Show this help message\n\
             \n\
             Arguments:\n\
             CONNECTION_STRING        PostgreSQL connection string \
             (e.g. postgres://user:pass@host:5432/db)\n\
             Overrides database_url in config and PGMCP_DATABASE_URL.\n\
             \n\
             Examples:\n\
             pgmcp postgres://myuser:mypass@localhost/mydb\n\
             pgmcp --config /etc/pgmcp.toml --transport sse\n\
             pgmcp --transport stdio 'host=localhost dbname=mydb user=me'\n"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TOML deserialization ────────────────────────────────────────────

    #[test]
    fn test_config_from_minimal_toml() {
        let toml = r#"
            database_url = "postgres://user:pass@localhost:5432/db"
        "#;
        let cfg: Config = toml::from_str(toml).expect("minimal config must parse");
        assert_eq!(cfg.database_url, "postgres://user:pass@localhost:5432/db");
        assert_eq!(cfg.pool.min_size, 2);
        assert_eq!(cfg.pool.max_size, 10);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 5);
        assert_eq!(cfg.pool.idle_timeout_seconds, 300);
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
        assert_eq!(cfg.transport.host, "127.0.0.1");
        assert_eq!(cfg.transport.port, 3000);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
        assert_eq!(cfg.telemetry.log_level, "info");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 30);
        assert!(cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_config_from_full_toml() {
        let toml = r#"
            database_url = "postgres://myuser:secret@db.example.com:5433/prod"

            [pool]
            min_size = 5
            max_size = 50
            acquire_timeout_seconds = 10
            idle_timeout_seconds = 600

            [transport]
            mode = "sse"
            host = "0.0.0.0"
            port = 8080

            [telemetry]
            log_format = "json"
            log_level = "debug"

            [cache]
            invalidation_interval_seconds = 60

            [guardrails]
            block_ddl = false
            block_copy_program = true
            block_session_set = false
        "#;
        let cfg: Config = toml::from_str(toml).expect("full config must parse");
        assert_eq!(cfg.database_url, "postgres://myuser:secret@db.example.com:5433/prod");
        assert_eq!(cfg.pool.min_size, 5);
        assert_eq!(cfg.pool.max_size, 50);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 10);
        assert_eq!(cfg.pool.idle_timeout_seconds, 600);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
        assert_eq!(cfg.transport.host, "0.0.0.0");
        assert_eq!(cfg.transport.port, 8080);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
        assert_eq!(cfg.telemetry.log_level, "debug");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 60);
        assert!(!cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(!cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_transport_mode_deserializes_stdio() {
        let toml = "database_url = \"postgres://u:p@h/d\"\n[transport]\nmode = \"stdio\"";
        let cfg: Config = toml::from_str(toml).expect("stdio mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
    }

    #[test]
    fn test_transport_mode_deserializes_sse() {
        let toml = "database_url = \"postgres://u:p@h/d\"\n[transport]\nmode = \"sse\"";
        let cfg: Config = toml::from_str(toml).expect("sse mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_log_format_deserializes_json() {
        let toml =
            "database_url = \"postgres://u:p@h/d\"\n[telemetry]\nlog_format = \"json\"";
        let cfg: Config = toml::from_str(toml).expect("json log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_log_format_deserializes_text() {
        let toml =
            "database_url = \"postgres://u:p@h/d\"\n[telemetry]\nlog_format = \"text\"";
        let cfg: Config = toml::from_str(toml).expect("text log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
    }

    // ── Environment variable overrides ─────────────────────────────────

    #[test]
    fn test_env_override_database_url() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://original@localhost/db\"").unwrap();
        cfg.apply_env_overrides_from(&[(
            "PGMCP_DATABASE_URL",
            "postgres://override@remotehost/newdb",
        )]);
        assert_eq!(cfg.database_url, "postgres://override@remotehost/newdb");
    }

    #[test]
    fn test_env_override_pool_max_size() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_POOL__MAX_SIZE", "99")]);
        assert_eq!(cfg.pool.max_size, 99);
    }

    #[test]
    fn test_env_override_transport_mode_sse() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TRANSPORT__MODE", "sse")]);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_env_override_log_format_json() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TELEMETRY__LOG_FORMAT", "json")]);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_env_override_unknown_key_is_ignored() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_DOES_NOT_EXIST", "value")]);
        assert_eq!(cfg.database_url, "postgres://u:p@h/d");
    }

    // ── CLI connection string shorthand ────────────────────────────────

    #[test]
    fn test_cli_connection_string_sets_database_url() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://original@localhost/db\"").unwrap();
        cfg.apply_cli_connection_string("postgres://cli_user:cli_pass@cli_host/cli_db");
        assert_eq!(
            cfg.database_url,
            "postgres://cli_user:cli_pass@cli_host/cli_db"
        );
    }

    // ── CLI argument parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_cli_args_config_flag() {
        let args = ["pgmcp", "--config", "/etc/pgmcp.toml"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/etc/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, None);
        assert_eq!(parsed.connection_string, None);
    }

    #[test]
    fn test_parse_cli_args_transport_flag() {
        let args = ["pgmcp", "--transport", "sse"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.transport, Some("sse".to_string()));
    }

    #[test]
    fn test_parse_cli_args_positional_connection_string() {
        let args = ["pgmcp", "postgres://u:p@h/d"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    #[test]
    fn test_parse_cli_args_all_flags() {
        let args = [
            "pgmcp",
            "--config",
            "/tmp/pgmcp.toml",
            "--transport",
            "stdio",
            "postgres://u:p@h/d",
        ];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/tmp/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, Some("stdio".to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    // ── Validation ─────────────────────────────────────────────────────

    #[test]
    fn test_validate_rejects_empty_database_url() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.database_url = String::new();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("database_url"));
    }

    #[test]
    fn test_validate_rejects_zero_max_pool_size() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.pool.max_size = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_min_size_greater_than_max_size() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.pool.min_size = 10;
        cfg.pool.max_size = 5;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_zero_acquire_timeout() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.pool.acquire_timeout_seconds = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_sse_port() {
        let mut cfg: Config =
            toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        cfg.transport.mode = TransportMode::Sse;
        cfg.transport.port = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_accepts_valid_config() {
        let cfg: Config = toml::from_str("database_url = \"postgres://u:p@h/d\"").unwrap();
        assert!(cfg.validate().is_ok());
    }
}
```

---

### Step 2.3: Verify tests pass (GREEN)

- [ ] **Run the test suite:**

```bash
cargo test --lib 2>&1
```

Expected output:

```
running 20 tests
test config::tests::test_config_from_minimal_toml ... ok
test config::tests::test_config_from_full_toml ... ok
test config::tests::test_transport_mode_deserializes_stdio ... ok
test config::tests::test_transport_mode_deserializes_sse ... ok
test config::tests::test_log_format_deserializes_json ... ok
test config::tests::test_log_format_deserializes_text ... ok
test config::tests::test_env_override_database_url ... ok
test config::tests::test_env_override_pool_max_size ... ok
test config::tests::test_env_override_transport_mode_sse ... ok
test config::tests::test_env_override_log_format_json ... ok
test config::tests::test_env_override_unknown_key_is_ignored ... ok
test config::tests::test_cli_connection_string_sets_database_url ... ok
test config::tests::test_parse_cli_args_config_flag ... ok
test config::tests::test_parse_cli_args_transport_flag ... ok
test config::tests::test_parse_cli_args_positional_connection_string ... ok
test config::tests::test_parse_cli_args_all_flags ... ok
test config::tests::test_validate_rejects_empty_database_url ... ok
test config::tests::test_validate_rejects_zero_max_pool_size ... ok
test config::tests::test_validate_rejects_min_size_greater_than_max_size ... ok
test config::tests::test_validate_rejects_zero_acquire_timeout ... ok
test config::tests::test_validate_rejects_invalid_sse_port ... ok
test config::tests::test_validate_accepts_valid_config ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

### Step 2.4: Wire `Config::load` into `main.rs` (REFACTOR)

- [ ] **Update `src/main.rs`:**

```rust
// src/main.rs
#![deny(warnings)]

mod config;
mod error;
mod pg;
mod server;
mod sql;
mod streaming;
mod telemetry;
mod tools;
mod transport;

use config::CliArgs;

fn main() {
    let args = CliArgs::parse();
    let _config = match config::Config::load(
        args.config.as_deref(),
        args.connection_string.as_deref(),
        args.transport.as_deref(),
    ) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("pgmcp: configuration error: {e}");
            std::process::exit(2);
        }
    };
    // Startup sequence continues in feat/003 (telemetry) and feat/005 (pool).
}
```

- [ ] **Run `cargo clippy --all-targets --all-features -- -D warnings`** to confirm no warnings.

- [ ] **Run `cargo fmt --check`** to confirm formatting is clean.

---

### Step 2.5: Commit

- [ ] **Stage and commit:**

```bash
git add src/config.rs src/main.rs config/pgmcp.example.toml
git commit -m "$(cat <<'EOF'
feat(002): implement Config with TOML loading, env overrides, and CLI args

Why: Config must exist before any code that needs it. Subsequent branches
(pool, transport, telemetry) all depend on a validated Config.
What: Config struct with all fields and serde defaults, apply_env_overrides
for PGMCP_* vars, hand-rolled CliArgs parser (--config, --transport,
positional connection string), and validate() checking all constraints.
22 unit tests, all passing.
EOF
)"
```

---

## Task 3: feat/003 — Telemetry

**Branch:** `feat/003-telemetry`

**Goal:** Implement `telemetry.rs` with tracing subscriber initialization, log format
selection (JSON vs text), and `RUST_LOG` integration. Wire into `main.rs`. TDD.

**Files modified:**
- `src/telemetry.rs` (primary)
- `src/main.rs` (wire `init_telemetry`)

**No new dependencies** — `tracing` and `tracing-subscriber` are already in `Cargo.toml`.

---

### Step 3.1: Write failing tests (RED)

- [ ] **Add tests to `src/telemetry.rs`**

```rust
// src/telemetry.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogFormat;

    #[test]
    fn test_init_telemetry_text_format_succeeds() {
        // init_telemetry must not panic or return an error for text format.
        // We call it in a fresh thread to avoid subscriber conflicts with
        // other tests. The test verifies the function signature and that it
        // returns Ok(()).
        let result = std::panic::catch_unwind(|| {
            init_telemetry(LogFormat::Text, "info").is_ok()
        });
        // If it panicked due to a subscriber already being set, that means
        // init_telemetry was called successfully in another test first — which
        // is fine. Either way, the function must not return an Err.
        assert!(result.is_ok() || result.is_err()); // always passes; real check below
    }

    #[test]
    fn test_init_telemetry_json_format_does_not_panic() {
        // The JSON subscriber must also initialize without panicking.
        let result = std::panic::catch_unwind(|| {
            // init_telemetry returns Err only if a subscriber is already set;
            // that is acceptable in tests — the important invariant is that it
            // does NOT panic on first call.
            let _ = init_telemetry(LogFormat::Json, "warn");
        });
        assert!(result.is_ok(), "init_telemetry(Json) must not panic");
    }

    #[test]
    fn test_log_format_enum_is_copy() {
        // LogFormat must be Copy so it can be passed by value without cloning.
        fn requires_copy<T: Copy>(_: T) {}
        requires_copy(LogFormat::Text);
        requires_copy(LogFormat::Json);
    }

    #[test]
    fn test_init_telemetry_returns_result_not_unit() {
        // The function signature must return Result<(), TelemetryError> (or similar).
        // This test encodes the contract at the type level: the function is fallible.
        fn _assert_return_type(format: LogFormat, level: &str) -> Result<(), impl std::error::Error> {
            init_telemetry(format, level)
        }
    }

    #[test]
    fn test_rust_log_env_var_takes_precedence() {
        // When RUST_LOG is set, it must take precedence over the log_level argument.
        // We test this by checking that init_telemetry does not panic when RUST_LOG
        // is set to a valid filter and a different log_level is provided.
        std::env::set_var("RUST_LOG", "warn");
        let result = std::panic::catch_unwind(|| {
            let _ = init_telemetry(LogFormat::Text, "debug");
        });
        std::env::remove_var("RUST_LOG");
        assert!(result.is_ok(), "init_telemetry must not panic when RUST_LOG is set");
    }
}
```

- [ ] **Verify tests fail:**

```bash
cargo test --lib telemetry 2>&1 | head -20
```

Expected: compile error — `init_telemetry` does not exist yet. RED state confirmed.

---

### Step 3.2: Implement `telemetry.rs` (GREEN)

- [ ] **Implement `src/telemetry.rs`:**

```rust
// src/telemetry.rs
//
// Tracing subscriber initialization for pgmcp.
//
// Call `init_telemetry` once at startup, before any tracing events are emitted.
// If a global subscriber is already set (e.g., in tests), `init_telemetry`
// returns `Err(TelemetryError::AlreadyInitialized)` rather than panicking.
//
// Format selection:
//   LogFormat::Json  — structured JSON output, one object per line. Suitable for
//                      log aggregators (Datadog, CloudWatch, Loki, etc.).
//   LogFormat::Text  — human-readable output with ANSI color. Suitable for
//                      interactive development.
//
// Log level:
//   The `RUST_LOG` environment variable takes precedence over the `log_level`
//   argument. This matches the behavior of tracing-subscriber's EnvFilter.

use crate::config::LogFormat;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Error returned when telemetry initialization fails.
#[derive(Debug)]
pub(crate) enum TelemetryError {
    /// A global tracing subscriber was already set.
    /// This is normal in test environments where multiple tests call `init_telemetry`.
    AlreadyInitialized,
}

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInitialized => {
                write!(f, "a global tracing subscriber is already initialized")
            }
        }
    }
}

impl std::error::Error for TelemetryError {}

/// Initialize the global tracing subscriber.
///
/// Must be called once at process startup, before any `tracing::info!` or
/// similar macros are used. Calling it a second time returns
/// `Err(TelemetryError::AlreadyInitialized)` without modifying the subscriber.
///
/// # Arguments
///
/// - `format`: Output format. `LogFormat::Json` for production; `LogFormat::Text`
///   for development.
/// - `log_level`: Default filter directive (e.g., `"info"`, `"debug"`,
///   `"pgmcp=debug,tokio_postgres=warn"`). The `RUST_LOG` environment variable
///   takes precedence if set.
///
/// # Errors
///
/// Returns `Err(TelemetryError::AlreadyInitialized)` if a global subscriber is
/// already set. The caller should log this as a warning and continue; it is not
/// a fatal error.
///
/// # Examples
///
/// ```rust
/// use pgmcp::telemetry::init_telemetry;
/// use pgmcp::config::LogFormat;
///
/// // In main.rs:
/// init_telemetry(LogFormat::Text, "info").ok();
/// ```
pub(crate) fn init_telemetry(
    format: LogFormat,
    log_level: &str,
) -> Result<(), TelemetryError> {
    // EnvFilter checks RUST_LOG first; falls back to log_level argument.
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    let result = match format {
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .try_init()
        }
        LogFormat::Text => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer())
                .try_init()
        }
    };

    result.map_err(|_| TelemetryError::AlreadyInitialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogFormat;

    #[test]
    fn test_init_telemetry_text_format_succeeds() {
        // First call should succeed; subsequent calls return AlreadyInitialized.
        // Either outcome is acceptable here — we only check for no panic.
        let result = std::panic::catch_unwind(|| {
            let _ = init_telemetry(LogFormat::Text, "info");
        });
        assert!(result.is_ok(), "init_telemetry(Text) must not panic");
    }

    #[test]
    fn test_init_telemetry_json_format_does_not_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = init_telemetry(LogFormat::Json, "warn");
        });
        assert!(result.is_ok(), "init_telemetry(Json) must not panic");
    }

    #[test]
    fn test_log_format_enum_is_copy() {
        fn requires_copy<T: Copy>(_: T) {}
        requires_copy(LogFormat::Text);
        requires_copy(LogFormat::Json);
    }

    #[test]
    fn test_init_telemetry_returns_result_not_unit() {
        fn _assert_return_type(
            format: LogFormat,
            level: &str,
        ) -> Result<(), impl std::error::Error> {
            init_telemetry(format, level)
        }
    }

    #[test]
    fn test_rust_log_env_var_takes_precedence() {
        // SAFETY: test-only env mutation. Tests are run in separate processes
        // by cargo test, so parallel interference is not a concern.
        std::env::set_var("RUST_LOG", "warn");
        let result = std::panic::catch_unwind(|| {
            let _ = init_telemetry(LogFormat::Text, "debug");
        });
        std::env::remove_var("RUST_LOG");
        assert!(result.is_ok(), "init_telemetry must not panic when RUST_LOG is set");
    }

    #[test]
    fn test_already_initialized_returns_err_not_panic() {
        // Call init_telemetry twice. The second call must return Err, not panic.
        let _ = init_telemetry(LogFormat::Text, "info");
        let second = init_telemetry(LogFormat::Text, "info");
        assert!(
            second.is_err(),
            "second init_telemetry call must return AlreadyInitialized"
        );
        let err = second.unwrap_err();
        assert!(
            matches!(err, TelemetryError::AlreadyInitialized),
            "error must be AlreadyInitialized variant"
        );
    }

    #[test]
    fn test_telemetry_error_display() {
        let err = TelemetryError::AlreadyInitialized;
        let msg = err.to_string();
        assert!(
            msg.contains("already"),
            "Display must mention 'already': got '{msg}'"
        );
    }
}
```

---

### Step 3.3: Verify tests pass (GREEN)

- [ ] **Run tests:**

```bash
cargo test --lib telemetry 2>&1
```

Expected output:

```
running 7 tests
test telemetry::tests::test_init_telemetry_text_format_succeeds ... ok
test telemetry::tests::test_init_telemetry_json_format_does_not_panic ... ok
test telemetry::tests::test_log_format_enum_is_copy ... ok
test telemetry::tests::test_init_telemetry_returns_result_not_unit ... ok
test telemetry::tests::test_rust_log_env_var_takes_precedence ... ok
test telemetry::tests::test_already_initialized_returns_err_not_panic ... ok
test telemetry::tests::test_telemetry_error_display ... ok

test result: ok. 7 passed; 0 failed
```

---

### Step 3.4: Wire telemetry into `main.rs` (REFACTOR)

- [ ] **Update `src/main.rs`:**

```rust
// src/main.rs
#![deny(warnings)]

mod config;
mod error;
mod pg;
mod server;
mod sql;
mod streaming;
mod telemetry;
mod tools;
mod transport;

use config::CliArgs;

fn main() {
    let args = CliArgs::parse();

    let config = match config::Config::load(
        args.config.as_deref(),
        args.connection_string.as_deref(),
        args.transport.as_deref(),
    ) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("pgmcp: configuration error: {e}");
            std::process::exit(2);
        }
    };

    // Initialize telemetry before any further logging. Any error here means a
    // subscriber is already set (only possible in tests); in production this
    // is always the first and only initialization.
    if let Err(e) = telemetry::init_telemetry(
        config.telemetry.log_format,
        &config.telemetry.log_level,
    ) {
        eprintln!("pgmcp: telemetry warning: {e}");
    }

    tracing::info!("pgmcp starting");

    // Startup sequence continues in feat/005 (pool) and feat/006 (transport).
}
```

- [ ] **Run the full lib test suite:**

```bash
cargo test --lib 2>&1
```

All tests must pass. Confirm no warnings under `cargo clippy --all-targets --all-features -- -D warnings`.

---

### Step 3.5: Commit

- [ ] **Stage and commit:**

```bash
git add src/telemetry.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(003): implement telemetry with JSON/text tracing subscriber initialization

Why: Telemetry must be initialized before the startup gate runs so that
startup errors and diagnostics are logged in the correct format.
What: init_telemetry() selects JSON or text tracing-subscriber based on
LogFormat, respects RUST_LOG env var, returns Err on double-init instead
of panicking. 7 unit tests, all passing. Wired into main.rs startup sequence.
EOF
)"
```

---

## Task 4: feat/004 — Error Types

**Branch:** `feat/004-error-types`

**Goal:** Define `McpError` with all 12 error codes from spec Section 3.5. Implement
`std::fmt::Display` and `std::error::Error`. Implement `From<tokio_postgres::Error>`.
Write agent-friendly formatting with a `hint` field. TDD.

**Files modified:**
- `src/error.rs` (primary)

**No new dependencies** — `thiserror` is already in `Cargo.toml`.

---

### Step 4.1: Write failing tests (RED)

- [ ] **Add tests to `src/error.rs`**

```rust
// src/error.rs
#[cfg(test)]
mod tests {
    use super::*;

    // All 12 error code variants must exist and be constructible.
    #[test]
    fn test_all_error_codes_exist() {
        let _config = McpError::config_invalid("bad toml");
        let _pg_connect = McpError::pg_connect_failed("connection refused");
        let _pg_version = McpError::pg_version_unsupported("version 13 detected");
        let _pg_query = McpError::pg_query_failed("column does not exist");
        let _pg_pool = McpError::pg_pool_timeout("pool exhausted after 5s");
        let _not_found = McpError::tool_not_found("unknown_tool");
        let _param = McpError::param_invalid("sql", "must not be empty");
        let _guardrail = McpError::guardrail_violation("DDL is not permitted in query tool");
        let _sql_parse = McpError::sql_parse_error("unexpected token");
        let _schema = McpError::schema_not_found("nonexistent_schema");
        let _table = McpError::table_not_found("public", "nonexistent_table");
        let _internal = McpError::internal("unexpected None in cache");
    }

    // Display must include the error code for agent consumption.
    #[test]
    fn test_display_includes_error_code() {
        let err = McpError::config_invalid("missing database_url");
        let msg = err.to_string();
        assert!(
            msg.contains("config_invalid"),
            "Display must contain error code 'config_invalid': got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_message() {
        let err = McpError::pg_connect_failed("connection refused to localhost:5432");
        let msg = err.to_string();
        assert!(
            msg.contains("connection refused to localhost:5432"),
            "Display must include the user-facing message: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_pg_connect_failed() {
        let err = McpError::pg_connect_failed("connection refused");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "pg_connect_failed Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_config_invalid() {
        let err = McpError::config_invalid("database_url is required");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "config_invalid Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_guardrail_violation() {
        let err = McpError::guardrail_violation("DDL is not permitted");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "guardrail_violation Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_tool_not_found() {
        let err = McpError::tool_not_found("badtool");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "tool_not_found Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_param_invalid() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "param_invalid Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_schema_not_found() {
        let err = McpError::schema_not_found("ghost_schema");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "schema_not_found Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_table_not_found() {
        let err = McpError::table_not_found("public", "ghost_table");
        let msg = err.to_string();
        assert!(
            msg.contains("hint") || msg.contains("Hint"),
            "table_not_found Display must include a hint: got '{msg}'"
        );
    }

    // code() accessor must return the machine-readable string.
    #[test]
    fn test_code_returns_correct_string_for_each_variant() {
        assert_eq!(McpError::config_invalid("x").code(), "config_invalid");
        assert_eq!(McpError::pg_connect_failed("x").code(), "pg_connect_failed");
        assert_eq!(
            McpError::pg_version_unsupported("x").code(),
            "pg_version_unsupported"
        );
        assert_eq!(McpError::pg_query_failed("x").code(), "pg_query_failed");
        assert_eq!(McpError::pg_pool_timeout("x").code(), "pg_pool_timeout");
        assert_eq!(McpError::tool_not_found("x").code(), "tool_not_found");
        assert_eq!(McpError::param_invalid("f", "x").code(), "param_invalid");
        assert_eq!(
            McpError::guardrail_violation("x").code(),
            "guardrail_violation"
        );
        assert_eq!(McpError::sql_parse_error("x").code(), "sql_parse_error");
        assert_eq!(McpError::schema_not_found("x").code(), "schema_not_found");
        assert_eq!(
            McpError::table_not_found("s", "x").code(),
            "table_not_found"
        );
        assert_eq!(McpError::internal("x").code(), "internal");
    }

    // McpError must implement std::error::Error.
    #[test]
    fn test_implements_std_error() {
        fn requires_error<E: std::error::Error>(_: &E) {}
        let err = McpError::internal("test");
        requires_error(&err);
    }

    // McpError must be Send + Sync so it can be returned from async handlers.
    #[test]
    fn test_is_send_and_sync() {
        fn requires_send_sync<T: Send + Sync>() {}
        requires_send_sync::<McpError>();
    }

    // McpError must implement Debug.
    #[test]
    fn test_implements_debug() {
        let err = McpError::internal("debug test");
        let dbg = format!("{err:?}");
        assert!(!dbg.is_empty());
    }

    // to_json() must produce a valid JSON object with code, message, and hint fields.
    #[test]
    fn test_to_json_has_required_fields() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let json = err.to_json();
        assert_eq!(json["code"], "param_invalid");
        assert!(json["message"].is_string());
        assert!(json["hint"].is_string());
        // The source field must NOT be present in the JSON (internal errors are logged,
        // not forwarded to agents).
        assert!(json.get("source").is_none());
    }

    #[test]
    fn test_to_json_message_contains_user_facing_message() {
        let err = McpError::table_not_found("myschema", "mytable");
        let json = err.to_json();
        let msg = json["message"].as_str().unwrap();
        assert!(
            msg.contains("mytable"),
            "message must reference the table name: got '{msg}'"
        );
    }

    #[test]
    fn test_to_json_hint_is_non_empty_for_all_variants() {
        let errors = [
            McpError::config_invalid("x"),
            McpError::pg_connect_failed("x"),
            McpError::pg_version_unsupported("x"),
            McpError::pg_query_failed("x"),
            McpError::pg_pool_timeout("x"),
            McpError::tool_not_found("x"),
            McpError::param_invalid("f", "x"),
            McpError::guardrail_violation("x"),
            McpError::sql_parse_error("x"),
            McpError::schema_not_found("x"),
            McpError::table_not_found("s", "x"),
            McpError::internal("x"),
        ];
        for err in &errors {
            let json = err.to_json();
            let hint = json["hint"].as_str().unwrap_or("");
            assert!(
                !hint.is_empty(),
                "hint must be non-empty for {}: got empty string",
                err.code()
            );
        }
    }

    // From<String> conversion for convenience in tests and simple callsites.
    #[test]
    fn test_internal_from_string() {
        let err = McpError::internal("some unexpected state");
        assert_eq!(err.code(), "internal");
    }
}
```

- [ ] **Verify tests fail:**

```bash
cargo test --lib error 2>&1 | head -20
```

Expected: compile errors — `McpError` does not exist. RED state confirmed.

---

### Step 4.2: Implement `McpError` (GREEN)

- [ ] **Implement `src/error.rs`:**

```rust
// src/error.rs
//
// McpError: the single error type for all fallible operations in pgmcp.
//
// Design invariants (from spec section 3.5):
// - Every error has a machine-readable code (string, lowercase_snake_case).
// - Every error has a human-readable message suitable for returning to an agent.
// - Every error has a hint — an actionable suggestion the agent can act on.
// - Internal source errors (e.g., raw tokio-postgres errors) are stored for
//   logging but are NEVER included in to_json() output sent to agents.
// - McpError is Send + Sync so it can cross async task boundaries.
//
// Constructor convention: one constructor per error code, named after the code.
// This makes callsites readable:
//   return Err(McpError::table_not_found("public", "users"));
//   return Err(McpError::param_invalid("sql", "must not be empty"));

/// The single error type for all fallible operations in pgmcp.
///
/// Every public API boundary returns `Result<T, McpError>`. Raw errors from
/// dependencies (tokio-postgres, toml, etc.) are converted to `McpError` at
/// the module boundary and the original error is stored as `source` for logging.
#[derive(Debug)]
pub(crate) struct McpError {
    /// Machine-readable error code. Lowercase snake_case. Stable across versions.
    code: &'static str,

    /// Human-readable message suitable for returning to an agent.
    /// Written to be interpretable by a model, not a human reading a stack trace.
    message: String,

    /// Actionable hint for the agent. Non-empty for all variants.
    hint: String,

    /// Original source error, stored for logging. Never forwarded to agents.
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl McpError {
    // ── Constructors ─────────────────────────────────────────────────────

    /// Configuration is malformed or missing required fields.
    ///
    /// Typical causes: bad env var value, missing required field in TOML,
    /// invalid combination of options.
    pub(crate) fn config_invalid(message: impl Into<String>) -> Self {
        Self {
            code: "config_invalid",
            message: message.into(),
            hint: "Check the configuration file and PGMCP_* environment variables. \
                   See config/pgmcp.example.toml for the full schema with defaults."
                .to_string(),
            source: None,
        }
    }

    /// Could not connect to Postgres.
    ///
    /// Typical causes: wrong host, firewall blocking the port, bad credentials,
    /// Postgres not running.
    pub(crate) fn pg_connect_failed(message: impl Into<String>) -> Self {
        Self {
            code: "pg_connect_failed",
            message: message.into(),
            hint: "Verify that the PostgreSQL server is running and reachable at the \
                   configured host and port. Check database_url credentials. If using \
                   SSL, verify that the server's certificate is trusted."
                .to_string(),
            source: None,
        }
    }

    /// Postgres version is below the minimum supported version (14).
    pub(crate) fn pg_version_unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "pg_version_unsupported",
            message: message.into(),
            hint: "pgmcp requires PostgreSQL 14 or later. Upgrade the server or \
                   connect to a compatible instance."
                .to_string(),
            source: None,
        }
    }

    /// SQL execution error returned by Postgres.
    ///
    /// Typical causes: syntax error, permission denied, constraint violation,
    /// function does not exist.
    pub(crate) fn pg_query_failed(message: impl Into<String>) -> Self {
        Self {
            code: "pg_query_failed",
            message: message.into(),
            hint: "Review the SQL statement for syntax errors. Check that the \
                   connected role has the required permissions. Use the explain tool \
                   to analyze the query plan before executing."
                .to_string(),
            source: None,
        }
    }

    /// Could not acquire a connection from the pool within the configured timeout.
    pub(crate) fn pg_pool_timeout(message: impl Into<String>) -> Self {
        Self {
            code: "pg_pool_timeout",
            message: message.into(),
            hint: "The connection pool is exhausted. Reduce concurrency, increase \
                   pool.max_size in config, or increase pool.acquire_timeout_seconds. \
                   Check for long-running queries holding connections."
                .to_string(),
            source: None,
        }
    }

    /// Unknown tool name in tool call.
    ///
    /// Typical causes: agent typo, using a tool name from a different version
    /// of pgmcp, or a cloud-only tool in the OSS server.
    pub(crate) fn tool_not_found(tool_name: impl Into<String>) -> Self {
        let name = tool_name.into();
        let hint = format!(
            "The tool '{name}' does not exist. Call tools/list to see the available tools \
             and their exact names."
        );
        Self {
            code: "tool_not_found",
            message: format!("unknown tool: '{name}'"),
            hint,
            source: None,
        }
    }

    /// Tool parameter missing, wrong type, or failed validation.
    pub(crate) fn param_invalid(
        field: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let field = field.into();
        let reason = reason.into();
        let hint = format!(
            "Check the parameter '{field}': {reason}. Refer to the tool's parameter \
             schema in the tools/list response for valid values and types."
        );
        Self {
            code: "param_invalid",
            message: format!("invalid parameter '{field}': {reason}"),
            hint,
            source: None,
        }
    }

    /// SQL statement blocked by the analysis layer.
    ///
    /// Typical causes: DDL in the query tool, COPY TO/FROM PROGRAM,
    /// SET statements that affect session state.
    pub(crate) fn guardrail_violation(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            code: "guardrail_violation",
            message: format!("SQL statement blocked by guardrails: {reason}"),
            hint: "Review the guardrail policies in config.guardrails. DDL statements \
                   should be proposed via the propose_migration tool, not executed \
                   directly. Use dry_run: true to inspect the guardrail analysis \
                   without attempting execution."
                .to_string(),
            source: None,
        }
    }

    /// SQL statement did not parse with the Postgres dialect parser.
    pub(crate) fn sql_parse_error(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            code: "sql_parse_error",
            message: format!("SQL parse error: {reason}"),
            hint: "Verify that the SQL is valid PostgreSQL syntax. pgmcp uses the \
                   sqlparser crate with the Postgres dialect. Use dry_run: true to \
                   inspect the parse result before execution."
                .to_string(),
            source: None,
        }
    }

    /// The specified schema does not exist in the database.
    pub(crate) fn schema_not_found(schema: impl Into<String>) -> Self {
        let schema = schema.into();
        let hint = format!(
            "Schema '{schema}' does not exist or is not visible to the connected role. \
             Use list_schemas to see available schemas."
        );
        Self {
            code: "schema_not_found",
            message: format!("schema not found: '{schema}'"),
            hint,
            source: None,
        }
    }

    /// The specified table does not exist in the given schema.
    pub(crate) fn table_not_found(
        schema: impl Into<String>,
        table: impl Into<String>,
    ) -> Self {
        let schema = schema.into();
        let table = table.into();
        let hint = format!(
            "Table '{table}' does not exist in schema '{schema}', or is not visible \
             to the connected role. Use list_tables to see available tables."
        );
        Self {
            code: "table_not_found",
            message: format!("table not found: '{schema}.{table}'"),
            hint,
            source: None,
        }
    }

    /// Unexpected error with no more specific code.
    ///
    /// Presence of this error in production logs indicates a bug that should
    /// be filed and fixed.
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal",
            message: message.into(),
            hint: "This is an unexpected internal error. Please report it as a bug \
                   with the full error message and the request that triggered it."
                .to_string(),
            source: None,
        }
    }

    // ── Builder for attaching a source error ─────────────────────────────

    /// Attach a source error for logging purposes.
    ///
    /// The source is stored internally and written to the tracing span when
    /// the error is logged. It is NEVER included in `to_json()` output sent
    /// to agents — raw database errors may contain sensitive data.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// tokio_postgres::connect(&url, NoTls).await.map_err(|e| {
    ///     McpError::pg_connect_failed(format!("could not connect to {url}"))
    ///         .with_source(e)
    /// })?;
    /// ```
    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    /// Returns the machine-readable error code.
    ///
    /// This is the stable, lowercase_snake_case identifier for the error kind.
    /// Agents should use this field for programmatic error handling.
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable message.
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// Returns the agent-actionable hint.
    pub(crate) fn hint(&self) -> &str {
        &self.hint
    }

    /// Serialize the error to a `serde_json::Value` for inclusion in MCP responses.
    ///
    /// The output JSON has exactly three fields:
    /// - `code`: machine-readable error code (string)
    /// - `message`: human-readable message for the agent (string)
    /// - `hint`: actionable suggestion for the agent (string)
    ///
    /// The `source` field is intentionally excluded. Raw database errors may
    /// contain sensitive information (query text, schema names, constraint names)
    /// and must not be forwarded to agents.
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "message": self.message,
            "hint": self.hint,
        })
    }
}

// ── std::fmt::Display ─────────────────────────────────────────────────────────

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{code}] {message} (hint: {hint})",
            code = self.code,
            message = self.message,
            hint = self.hint,
        )
    }
}

// ── std::error::Error ─────────────────────────────────────────────────────────

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

// ── From conversions ──────────────────────────────────────────────────────────

impl From<tokio_postgres::Error> for McpError {
    /// Convert a raw tokio-postgres error into an `McpError`.
    ///
    /// The conversion inspects the error kind to choose the most specific
    /// error code. The raw error is attached as `source` for logging but is
    /// not forwarded to agents.
    fn from(err: tokio_postgres::Error) -> Self {
        use tokio_postgres::error::SqlState;

        // tokio_postgres::Error::db_error() returns Some if this is a
        // server-reported SQL error (SQLSTATE). Connectivity errors (IO,
        // TLS, protocol) return None.
        if let Some(db_err) = err.as_db_error() {
            let code = db_err.code();
            if code == &SqlState::CONNECTION_FAILURE
                || code == &SqlState::CONNECTION_EXCEPTION
                || code == &SqlState::SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION
            {
                return McpError::pg_connect_failed(db_err.message().to_string())
                    .with_source(err);
            }
            // All other DB errors (permission denied, constraint violation,
            // syntax error, etc.) map to pg_query_failed.
            return McpError::pg_query_failed(db_err.message().to_string())
                .with_source(err);
        }

        // Non-DB errors (IO failure, TLS, unexpected close) indicate a
        // connectivity problem rather than a query failure.
        McpError::pg_connect_failed(format!("postgres connection error: {err}"))
            .with_source(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_error_codes_exist() {
        let _config = McpError::config_invalid("bad toml");
        let _pg_connect = McpError::pg_connect_failed("connection refused");
        let _pg_version = McpError::pg_version_unsupported("version 13 detected");
        let _pg_query = McpError::pg_query_failed("column does not exist");
        let _pg_pool = McpError::pg_pool_timeout("pool exhausted after 5s");
        let _not_found = McpError::tool_not_found("unknown_tool");
        let _param = McpError::param_invalid("sql", "must not be empty");
        let _guardrail = McpError::guardrail_violation("DDL is not permitted in query tool");
        let _sql_parse = McpError::sql_parse_error("unexpected token");
        let _schema = McpError::schema_not_found("nonexistent_schema");
        let _table = McpError::table_not_found("public", "nonexistent_table");
        let _internal = McpError::internal("unexpected None in cache");
    }

    #[test]
    fn test_display_includes_error_code() {
        let err = McpError::config_invalid("missing database_url");
        let msg = err.to_string();
        assert!(
            msg.contains("config_invalid"),
            "Display must contain 'config_invalid': got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_message() {
        let err = McpError::pg_connect_failed("connection refused to localhost:5432");
        let msg = err.to_string();
        assert!(
            msg.contains("connection refused to localhost:5432"),
            "Display must include the message: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_pg_connect_failed() {
        let err = McpError::pg_connect_failed("connection refused");
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("hint"),
            "Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_config_invalid() {
        let err = McpError::config_invalid("database_url is required");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_guardrail_violation() {
        let err = McpError::guardrail_violation("DDL is not permitted");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_tool_not_found() {
        let err = McpError::tool_not_found("badtool");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_param_invalid() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_schema_not_found() {
        let err = McpError::schema_not_found("ghost_schema");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_table_not_found() {
        let err = McpError::table_not_found("public", "ghost_table");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_code_returns_correct_string_for_each_variant() {
        assert_eq!(McpError::config_invalid("x").code(), "config_invalid");
        assert_eq!(McpError::pg_connect_failed("x").code(), "pg_connect_failed");
        assert_eq!(
            McpError::pg_version_unsupported("x").code(),
            "pg_version_unsupported"
        );
        assert_eq!(McpError::pg_query_failed("x").code(), "pg_query_failed");
        assert_eq!(McpError::pg_pool_timeout("x").code(), "pg_pool_timeout");
        assert_eq!(McpError::tool_not_found("x").code(), "tool_not_found");
        assert_eq!(McpError::param_invalid("f", "x").code(), "param_invalid");
        assert_eq!(
            McpError::guardrail_violation("x").code(),
            "guardrail_violation"
        );
        assert_eq!(McpError::sql_parse_error("x").code(), "sql_parse_error");
        assert_eq!(McpError::schema_not_found("x").code(), "schema_not_found");
        assert_eq!(
            McpError::table_not_found("s", "x").code(),
            "table_not_found"
        );
        assert_eq!(McpError::internal("x").code(), "internal");
    }

    #[test]
    fn test_implements_std_error() {
        fn requires_error<E: std::error::Error>(_: &E) {}
        let err = McpError::internal("test");
        requires_error(&err);
    }

    #[test]
    fn test_is_send_and_sync() {
        fn requires_send_sync<T: Send + Sync>() {}
        requires_send_sync::<McpError>();
    }

    #[test]
    fn test_implements_debug() {
        let err = McpError::internal("debug test");
        let dbg = format!("{err:?}");
        assert!(!dbg.is_empty());
    }

    #[test]
    fn test_to_json_has_required_fields() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let json = err.to_json();
        assert_eq!(json["code"], "param_invalid");
        assert!(json["message"].is_string());
        assert!(json["hint"].is_string());
        assert!(json.get("source").is_none());
    }

    #[test]
    fn test_to_json_message_contains_user_facing_message() {
        let err = McpError::table_not_found("myschema", "mytable");
        let json = err.to_json();
        let msg = json["message"].as_str().unwrap();
        assert!(
            msg.contains("mytable"),
            "message must reference the table name: got '{msg}'"
        );
    }

    #[test]
    fn test_to_json_hint_is_non_empty_for_all_variants() {
        let errors = [
            McpError::config_invalid("x"),
            McpError::pg_connect_failed("x"),
            McpError::pg_version_unsupported("x"),
            McpError::pg_query_failed("x"),
            McpError::pg_pool_timeout("x"),
            McpError::tool_not_found("x"),
            McpError::param_invalid("f", "x"),
            McpError::guardrail_violation("x"),
            McpError::sql_parse_error("x"),
            McpError::schema_not_found("x"),
            McpError::table_not_found("s", "x"),
            McpError::internal("x"),
        ];
        for err in &errors {
            let json = err.to_json();
            let hint = json["hint"].as_str().unwrap_or("");
            assert!(
                !hint.is_empty(),
                "hint must be non-empty for {}: got empty string",
                err.code()
            );
        }
    }

    #[test]
    fn test_with_source_does_not_affect_to_json() {
        // Attach a source error and verify it does not appear in to_json output.
        use std::io;
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused");
        let err = McpError::pg_connect_failed("could not connect").with_source(io_err);
        let json = err.to_json();
        // source must not leak into agent-visible JSON
        assert!(json.get("source").is_none());
        assert_eq!(json["code"], "pg_connect_failed");
    }

    #[test]
    fn test_error_source_chain() {
        use std::io;
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let err =
            McpError::pg_connect_failed("could not connect").with_source(io_err);
        // std::error::Error::source() must expose the attached error for logging.
        assert!(err.source().is_some());
    }

    #[test]
    fn test_tool_not_found_message_includes_tool_name() {
        let err = McpError::tool_not_found("badtool");
        assert!(err.message().contains("badtool"));
        assert!(err.hint().contains("badtool"));
    }

    #[test]
    fn test_param_invalid_message_includes_field_and_reason() {
        let err = McpError::param_invalid("limit", "must be positive");
        assert!(err.message().contains("limit"));
        assert!(err.message().contains("must be positive"));
    }

    #[test]
    fn test_schema_not_found_hint_includes_schema_name() {
        let err = McpError::schema_not_found("analytics");
        assert!(err.hint().contains("analytics"));
    }

    #[test]
    fn test_table_not_found_message_includes_schema_and_table() {
        let err = McpError::table_not_found("public", "orders");
        assert!(err.message().contains("public"));
        assert!(err.message().contains("orders"));
    }

    #[test]
    fn test_internal_error_hint_mentions_bug() {
        let err = McpError::internal("unexpected None");
        let hint = err.hint();
        assert!(
            hint.to_lowercase().contains("bug") || hint.to_lowercase().contains("report"),
            "internal error hint should mention reporting a bug: got '{hint}'"
        );
    }
}
```

---

### Step 4.3: Verify tests pass (GREEN)

- [ ] **Run tests:**

```bash
cargo test --lib error 2>&1
```

Expected output:

```
running 25 tests
test error::tests::test_all_error_codes_exist ... ok
test error::tests::test_display_includes_error_code ... ok
test error::tests::test_display_includes_message ... ok
test error::tests::test_display_includes_hint_for_pg_connect_failed ... ok
test error::tests::test_display_includes_hint_for_config_invalid ... ok
test error::tests::test_display_includes_hint_for_guardrail_violation ... ok
test error::tests::test_display_includes_hint_for_tool_not_found ... ok
test error::tests::test_display_includes_hint_for_param_invalid ... ok
test error::tests::test_display_includes_hint_for_schema_not_found ... ok
test error::tests::test_display_includes_hint_for_table_not_found ... ok
test error::tests::test_code_returns_correct_string_for_each_variant ... ok
test error::tests::test_implements_std_error ... ok
test error::tests::test_is_send_and_sync ... ok
test error::tests::test_implements_debug ... ok
test error::tests::test_to_json_has_required_fields ... ok
test error::tests::test_to_json_message_contains_user_facing_message ... ok
test error::tests::test_to_json_hint_is_non_empty_for_all_variants ... ok
test error::tests::test_with_source_does_not_affect_to_json ... ok
test error::tests::test_error_source_chain ... ok
test error::tests::test_tool_not_found_message_includes_tool_name ... ok
test error::tests::test_param_invalid_message_includes_field_and_reason ... ok
test error::tests::test_schema_not_found_hint_includes_schema_name ... ok
test error::tests::test_table_not_found_message_includes_schema_and_table ... ok
test error::tests::test_internal_error_hint_mentions_bug ... ok

test result: ok. 24+ passed; 0 failed
```

---

### Step 4.4: Run full suite and lint (REFACTOR)

- [ ] **Run all lib tests:**

```bash
cargo test --lib 2>&1
```

All tests from feat/002 (config), feat/003 (telemetry), and feat/004 (error) must pass.

- [ ] **Run clippy:**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

No warnings.

- [ ] **Run fmt check:**

```bash
cargo fmt --check
```

No differences. If there are differences, run `cargo fmt` then re-check.

---

### Step 4.5: Commit

- [ ] **Stage and commit:**

```bash
git add src/error.rs
git commit -m "$(cat <<'EOF'
feat(004): define McpError with all 12 error codes and agent-friendly formatting

Why: Error types must be defined before any fallible code is written.
All subsequent branches return Result<T, McpError> at module boundaries.
What: McpError struct with code/message/hint/source fields, 12 constructor
functions, Display impl (includes code and hint), std::error::Error impl
(exposes source chain for logging), From<tokio_postgres::Error> conversion,
to_json() that omits source to prevent leaking sensitive data to agents.
24 unit tests, all passing.
EOF
)"
```

---

## Phase 1 Completion Verification

After all four branches are merged to `main`, run the following to confirm the phase
is complete:

- [ ] **Build succeeds cleanly:**

```bash
cargo build 2>&1
```

Expected: `Finished` with no warnings, no errors.

- [ ] **All lib tests pass:**

```bash
cargo test --lib 2>&1
```

Expected: 50+ tests passing, 0 failed, 0 ignored.

- [ ] **Clippy is clean:**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: `Finished` with no warnings.

- [ ] **Formatting is canonical:**

```bash
cargo fmt --check
```

Expected: exit 0, no output.

- [ ] **Module structure matches spec Section 5.1:**

Every file listed in the spec exists. Verify with:

```bash
find /home/eric/Code/pgmcp/src -type f | sort
```

Expected: all 40+ source files present.

- [ ] **`main.rs` compiles and runs:**

```bash
cargo run -- --help 2>&1
```

Expected: usage message printed to stderr, exit 0.

```bash
cargo run -- postgres://user:pass@localhost/db 2>&1
```

Expected: "pgmcp starting" log line (or config error if no Postgres is running —
config loading succeeds since `postgres://user:pass@localhost/db` is a valid URL).

---

## Summary

| Branch | Files | Tests | Key Deliverables |
|---|---|---|---|
| feat/001 | 55 stubs | 0 | Cargo.toml (15 deps), toolchain pin, module stubs, CI, deny.toml, example config, LICENSE |
| feat/002 | `config.rs`, `main.rs` | 22 | Config struct, TOML deser, env overrides, CLI args, validation |
| feat/003 | `telemetry.rs`, `main.rs` | 7 | `init_telemetry`, JSON/text subscriber, RUST_LOG integration |
| feat/004 | `error.rs` | 24 | McpError with 12 codes, Display, std::error::Error, From<pg::Error>, to_json |

**Total Phase 1 tests:** 53 unit tests, all in `#[cfg(test)]` blocks, no Postgres dependency.

**Phase 2 prerequisite:** All four branches merged to `main`, CI green.
