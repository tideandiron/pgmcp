# pgmcp

A Rust MCP server for PostgreSQL. Zero-overhead agent access to Postgres.

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](rust-toolchain.toml)

## What is this?

pgmcp is a [Model Context Protocol](https://modelcontextprotocol.io/) server that exposes PostgreSQL to AI agents. It is the fastest and most capable way for an LLM to interact with a Postgres database.

pgmcp is Postgres-only by design. This is not a limitation — it is the entire point. By supporting only PostgreSQL, pgmcp can exploit Postgres-specific features (pg_catalog introspection, advisory locks, server-side cursors, row-level security) and guarantee zero-copy row streaming where the OID type system permits.

**Performance goal:** Zero overhead over raw PostgreSQL response. Rows leave Postgres and reach the MCP client with the minimum number of copies the hardware allows.

**Status:** Active development. pgmcp is pre-v1.0. [See current phase below.](#status--what-works-today)

## Status: What Works Today

pgmcp is in **Phase 3 of 9** complete. All 8 discovery tools are implemented and tested against real PostgreSQL. The remaining 7 tools (query, explain, suggest_index, propose_migration, my_permissions, and schema modification tools) are stubs being implemented next.

| Category | Status |
|----------|--------|
| MCP protocol (stdio + SSE) | Working |
| Connection pooling & PG version validation (14-17) | Working |
| Tool manifest (15 tools discoverable by agents) | Working |
| Discovery tools (8/8) | **Implemented** |
| SQL-accepting tools (0/4) | Stubs |
| Introspection tools (2/3) | health + connection_info implemented, my_permissions stub |

**159 tests** (84 unit + 75 integration against real PostgreSQL via testcontainers).

See the [MVP design specification](docs/specs/2026-04-07-pgmcp-mvp-design.md) for the full roadmap and architectural details.

### What's Coming Next

- Schema cache with background invalidation (Phase 4)
- SQL parser, guardrails, and LIMIT injection (Phase 5)
- Query tool with full row-streaming and adaptive batching (Phase 6)
- Intelligence tools: explain, suggest_index, propose_migration (Phase 7)
- Cross-platform binaries, Docker image (Phase 9)

## Quick Start

### Prerequisites

- Rust 1.85 or later
- PostgreSQL 14–17
- A Postgres connection string

### Build from Source

```bash
git clone https://github.com/tideandiron/pgmcp.git
cd pgmcp

# Build the binary
cargo build --release

# Run tests (requires a running Postgres instance)
cargo test

# Run the server
./target/release/pgmcp postgres://user:pass@localhost:5432/mydb
```

The server accepts a connection string as the first argument, which overrides the `database_url` in the config file.

### Configuration

pgmcp reads configuration from `pgmcp.toml` in the current directory. See `config/pgmcp.example.toml` for all available options.

Environment variable overrides: prefix any config key with `PGMCP_` and use `SCREAMING_SNAKE_CASE`. Nested keys use double underscores.

```bash
# Example environment variable overrides
export PGMCP_DATABASE_URL="postgres://localhost/test"
export PGMCP_POOL__MAX_SIZE=20
export PGMCP_TRANSPORT__MODE=sse
export PGMCP_TRANSPORT__PORT=3000

./target/release/pgmcp
```

### Available Transports

**stdio** (default): reads JSON-RPC from stdin, writes to stdout. Use this when launching pgmcp as a subprocess.

```bash
pgmcp postgres://localhost/mydb
# Reads MCP messages from stdin, writes responses to stdout
```

**sse**: HTTP server with Server-Sent Events for streaming, POST for client-to-server. Use this for network-accessible deployments.

```bash
export PGMCP_TRANSPORT__MODE=sse
export PGMCP_TRANSPORT__HOST=0.0.0.0
export PGMCP_TRANSPORT__PORT=3000
pgmcp postgres://localhost/mydb
# Server listens on http://0.0.0.0:3000
```

## Tools (MVP Surface)

pgmcp exposes 15 tools across three categories. 10 of 15 are implemented; the rest are stubs being built.

### Discovery Tools (all implemented)

These read from pg_catalog. They never execute caller-supplied SQL.

- **`list_databases`** — list all databases on the Postgres instance
- **`server_info`** — Postgres version, settings, extensions, and current role
- **`list_schemas`** — all schemas in the current database (permission-filtered)
- **`list_tables`** — tables, views, and materialized views in a schema (with kind filter)
- **`describe_table`** — full table definition: columns, types, constraints, indexes, comments
- **`list_enums`** — all enum types with their ordered values
- **`list_extensions`** — installed extensions and versions
- **`table_stats`** — runtime statistics: sizes, scan counts, cache hit ratio, vacuum timestamps

### SQL-Accepting Tools (stubs — Phase 5-7)

All SQL-accepting tools will pass queries through SQL analysis and guardrails before execution.

- **`query`** — execute SQL with streaming results, LIMIT injection, dry-run, intent declaration
- **`explain`** — run `EXPLAIN (ANALYZE, BUFFERS)` and return the plan with plain-language analysis
- **`suggest_index`** — analyze a query plan and propose indexes
- **`propose_migration`** — analyze DDL SQL and return safety warnings, reverse SQL, impact assessment

### Introspection Tools (2 of 3 implemented)

- **`my_permissions`** — introspect the current role's privileges *(stub)*
- **`connection_info`** — host, port, database, SSL status, pool stats
- **`health`** — liveness check with latency measurement and pool stats

See the [MVP design specification, section 4](docs/specs/2026-04-07-pgmcp-mvp-design.md#4-mvp-tool-surface) for full parameter and return value documentation.

## Architecture & Design

pgmcp's architecture is designed around four core principles:

1. **Postgres is the product.** Every feature exists to expose Postgres capability to agents. Postgres-specific optimizations are preferred over generic abstractions.

2. **Zero overhead.** The hot path (row encoding) is measurement-driven. Pre-allocated write buffers are reused across rows. Rows are streamed, not collected. Unnecessary deserialization is avoided.

3. **Agents are the user.** Tool names are unambiguous English verbs. Return types are JSON structures for agent readability. Error messages are written for LLM consumption.

4. **Intentional exclusion is a feature.** pgmcp does not include a migration framework, ORM, GUI, plugin system, or multi-database support. This constraint is what makes the zero-overhead goal achievable.

### Key Components

- **Connection Pool** — bounded pool of tokio-postgres connections with configurable min/max, health checking, and acquire timeout
- **Schema Cache** — in-memory snapshot of pg_catalog data (tables, columns, enum values); refreshed on detected schema changes
- **SQL Analysis + Guardrails** — parses SQL statements, classifies them, injects LIMIT clauses, rejects dangerous queries before execution
- **Streaming Encoder** — converts Postgres rows to JSON or CSV without intermediate deserialization where the OID type system permits
- **Adaptive BatchSizer** — measures row size and adjusts batch boundaries to keep memory bounded while minimizing round-trips
- **Background Cache Invalidation** — polls pg_stat_database at a configurable interval and triggers cache refresh on detected changes

See the [system architecture section](docs/specs/2026-04-07-pgmcp-mvp-design.md#3-system-architecture) of the design spec for a detailed component diagram and data flow examples.

## Building from Source

### Prerequisites

- Rust 1.85+ (see `rust-toolchain.toml`)
- PostgreSQL 14–17 for running integration tests
- `cargo` (included with Rust)

### Development Build

```bash
cargo build
./target/debug/pgmcp postgres://localhost/mydb
```

### Release Build (Optimized)

```bash
cargo build --release
./target/release/pgmcp postgres://localhost/mydb
```

The release profile uses LTO (link-time optimization), single codegen unit, and binary stripping for minimal binary size.

### Running Tests

```bash
# Run all tests (unit + integration)
cargo test

# Run integration tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name

# Run integration tests only
cargo test --test integration
```

Integration tests spin up a real Postgres instance using [testcontainers](https://docs.rs/testcontainers). Postgres 15 is used by default.

### Linting & Formatting

```bash
# Format code
cargo fmt

# Check formatting without changes
cargo fmt --check

# Run clippy (lint warnings)
cargo clippy -- -D warnings

# Run all checks (as in CI)
cargo build && cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

### Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run a specific benchmark
cargo bench --bench serialization

# View benchmark results
open target/criterion/report/index.html
```

Benchmarks measure:
- Serialization performance (JSON/CSV encoding)
- Streaming throughput
- Connection pool acquisition

Regressions > 5% block merge.

## Contributing

pgmcp is company-led open source. All work follows the development workflow in the [MVP design specification, section 6](docs/specs/2026-04-07-pgmcp-mvp-design.md#6-working-process).

**Contributing guidelines:**

- Create a feature branch named `feat/NNN-short-description` (NNN is zero-padded: `feat/001`, `feat/012`, `feat/103`)
- One feature per branch; one branch per PR
- All code must pass: `cargo build`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`
- Add integration tests for new tools or guardrail rules
- Update `config/pgmcp.example.toml` if config keys change
- Update this README if the tool surface or user-facing behavior changes

See the [code quality standards section](docs/specs/2026-04-07-pgmcp-mvp-design.md#7-code-quality-standards) for naming conventions, error handling requirements, performance standards, and other rules.

## License

pgmcp is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [MCP Specification](https://modelcontextprotocol.io/)
- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [MVP Design Specification](docs/specs/2026-04-07-pgmcp-mvp-design.md)
