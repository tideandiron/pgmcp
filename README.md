# pgmcp

A Rust MCP server for PostgreSQL. Zero-overhead agent access to Postgres.

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](rust-toolchain.toml)
[![CI](https://github.com/tideandiron/pgmcp/actions/workflows/ci.yml/badge.svg)](https://github.com/tideandiron/pgmcp/actions/workflows/ci.yml)

## What is this?

pgmcp is a [Model Context Protocol](https://modelcontextprotocol.io/) server that exposes PostgreSQL to AI agents. It is the fastest and most capable way for an LLM to interact with a Postgres database.

pgmcp is Postgres-only by design. This is not a limitation — it is the entire point. By supporting only PostgreSQL, pgmcp can exploit Postgres-specific features (pg_catalog introspection, advisory locks, server-side cursors, row-level security) and guarantee zero-copy row streaming where the OID type system permits.

**Performance goal:** Zero overhead over raw PostgreSQL response. Rows leave Postgres and reach the MCP client with the minimum number of copies the hardware allows.

**Status:** MVP complete. All 15 tools implemented and tested against PostgreSQL 14–17.

## Status: What Works Today

pgmcp is **MVP complete**. All 15 tools are implemented, tested, and packaged.

| Category | Status |
|----------|--------|
| MCP protocol (stdio + SSE) | Working |
| Connection pooling & PG version validation (14–17) | Working |
| Schema cache with background invalidation | Working |
| SQL parser, guardrails, LIMIT injection | Working |
| Discovery tools (8/8) | Implemented |
| SQL-accepting tools (4/4) | Implemented |
| Introspection tools (3/3) | Implemented |
| Inferred column descriptions (~200 patterns) | Working |
| Docker image (multi-stage, ~13MB) | Available |
| CI pipeline (PG 14/15/16/17 matrix) | Active |

**464+ unit tests** + comprehensive integration tests against real PostgreSQL via testcontainers.

See the [MVP design specification](docs/specs/2026-04-07-pgmcp-mvp-design.md) for the full architecture and design rationale.

## Quick Start

### Prerequisites

- Rust 1.85 or later (see [`rust-toolchain.toml`](rust-toolchain.toml))
- PostgreSQL 14–17
- A Postgres connection string

### Build from Source

```bash
git clone https://github.com/tideandiron/pgmcp.git
cd pgmcp

# Build the binary
cargo build --release

# Run with a connection string
./target/release/pgmcp postgres://user:pass@localhost:5432/mydb
```

### Docker

```bash
# Build the image (multi-stage, ~13MB)
docker build -t pgmcp:latest .

# Run with a connection string
docker run --rm \
  -e PGMCP_DATABASE_URL="postgres://user:pass@localhost:5432/mydb" \
  pgmcp:latest
```

### Docker Compose (local development)

```bash
# Start pgmcp + PostgreSQL (SSE mode on port 3000)
docker compose up -d

# View logs
docker compose logs -f pgmcp

# Connect your MCP client to http://localhost:3000
```

See [`docker-compose.yml`](docker-compose.yml) for full configuration options.

## Configuration

pgmcp reads configuration from `pgmcp.toml` in the current directory. See [`config/pgmcp.example.toml`](config/pgmcp.example.toml) for all available options with documentation.

### Environment Variable Overrides

Prefix any config key with `PGMCP_` and use `SCREAMING_SNAKE_CASE`. Nested keys use double underscores.

```bash
export PGMCP_DATABASE_URL="postgres://localhost/mydb"
export PGMCP_POOL__MAX_SIZE=20
export PGMCP_TRANSPORT__MODE=sse
export PGMCP_TRANSPORT__PORT=3000

./target/release/pgmcp
```

### Available Transports

**stdio** (default): reads JSON-RPC from stdin, writes to stdout. Use this when launching pgmcp as a subprocess from an MCP client.

```bash
pgmcp postgres://localhost/mydb
# Reads MCP messages from stdin, writes responses to stdout
```

**sse**: HTTP server with Server-Sent Events for streaming, POST for client-to-server. Use this for network-accessible deployments.

```bash
PGMCP_TRANSPORT__MODE=sse PGMCP_TRANSPORT__PORT=3000 pgmcp postgres://localhost/mydb
# MCP endpoint: http://localhost:3000/mcp
```

## Tools (15 total)

pgmcp exposes 15 tools across three categories. All tool descriptions are written for LLM consumption: each specifies exact response fields, valid parameter values, error conditions, and usage guidance.

### Discovery Tools

These read from pg_catalog via the schema cache. They never execute caller-supplied SQL.

| Tool | Description |
|------|-------------|
| `list_databases` | All databases with size, owner, encoding |
| `server_info` | Postgres version, settings, installed extensions, current role |
| `list_schemas` | All schemas visible to the connected role |
| `list_tables` | Tables, views, materialized views with kind filter |
| `describe_table` | Full schema: columns, types, constraints, indexes, inferred descriptions |
| `list_enums` | Enum types with ordered label values |
| `list_extensions` | Installed extensions with versions |
| `table_stats` | Row counts, sizes, scan counts, vacuum timestamps |

### SQL-Accepting Tools

All SQL passes through the parser, guardrail rules, and LIMIT injection before execution.

| Tool | Description |
|------|-------------|
| `query` | Execute SQL with JSON/CSV output, dry-run, transaction wrapping, LIMIT injection |
| `explain` | EXPLAIN (FORMAT JSON, ANALYZE?, BUFFERS) with plain-language diagnostics |
| `suggest_index` | Analyze query plan for seq scans, suggest CREATE INDEX CONCURRENTLY |
| `propose_migration` | Generate DDL from intent with safety analysis (locks, downtime risk) |

**Guardrail rules** (always enforced):
1. DDL statements (CREATE, DROP, ALTER, TRUNCATE) are blocked — use `propose_migration` instead
2. DELETE and UPDATE without a WHERE clause are blocked
3. COPY TO/FROM PROGRAM is blocked
4. SET statements that modify session state are blocked

### Introspection Tools

| Tool | Description |
|------|-------------|
| `my_permissions` | Role attributes, schema and table-level privileges |
| `connection_info` | Host, port, database, SSL status, pool statistics |
| `health` | Liveness check with latency measurement and pool stats |

### Response Schema

All tools return JSON responses. The `query` tool response envelope:

```json
{
  "columns":           [{"name": "id", "type": "int4"}, ...],
  "rows":              [{"id": 1, ...}, ...],
  "row_count":         N,
  "truncated":         false,
  "format":            "json",
  "sql_executed":      "SELECT id FROM t LIMIT 100",
  "limit_injected":    true,
  "execution_time_ms": 1.5,
  "plan":              null
}
```

`truncated: true` means `row_count == limit`, indicating more rows may exist. Increase `limit` or add a `WHERE` clause to retrieve the full set.

## Architecture

pgmcp is built around four core principles:

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

### Error Model

Every error has three fields:

- `code` — machine-readable, stable, lowercase snake_case (e.g., `pg_query_failed`)
- `message` — human/agent-readable description
- `hint` — actionable suggestion for the agent

Error codes: `config_invalid`, `pg_connect_failed`, `pg_version_unsupported`, `pg_query_failed`, `pg_pool_timeout`, `tool_not_found`, `param_invalid`, `guardrail_violation`, `sql_parse_error`, `schema_not_found`, `table_not_found`, `internal`.

## Building from Source

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

The release profile uses LTO (link-time optimization), single codegen unit, and binary stripping.

### Running Tests

```bash
# Run all unit tests (fast, no Docker needed)
cargo test --lib

# Run all tests (unit + integration against real PostgreSQL)
cargo test

# Run integration tests against a specific PG version
PGMCP_TEST_PG_VERSION=15 cargo test --tests

# Run a specific test file
cargo test --test query

# Run with verbose output
cargo test -- --nocapture
```

Integration tests spin up a real Postgres instance using [testcontainers](https://docs.rs/testcontainers). Docker must be running. Tests skip automatically if Docker is not available.

### Linting & Formatting

```bash
# Format code
cargo fmt

# Check formatting without changes
cargo fmt --check

# Run clippy (all warnings as errors)
cargo clippy --all-targets --all-features -- -D warnings

# Run all checks (mirrors CI)
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --lib
```

### Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run a specific benchmark
cargo bench --bench serialization

# View HTML benchmark report
open target/criterion/report/index.html
```

Benchmarks measure: serialization performance (JSON/CSV encoding), streaming throughput, connection pool acquisition.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, branch model, code quality standards, and review process.

## License

pgmcp is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

## References

- [MCP Specification](https://modelcontextprotocol.io/)
- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [MVP Design Specification](docs/specs/2026-04-07-pgmcp-mvp-design.md)
