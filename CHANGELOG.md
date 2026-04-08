# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-04-08

### Added

- 15 MCP tools: 8 discovery (list_databases, server_info, list_schemas, list_tables, describe_table, list_enums, list_extensions, table_stats), 4 SQL-accepting (query, explain, suggest_index, propose_migration), 3 introspection (my_permissions, connection_info, health)
- Stdio and SSE (HTTP/Server-Sent Events) transport modes via rmcp
- Connection pooling with deadpool-postgres (configurable min/max/timeout)
- Background schema cache with pg_stat_database polling for invalidation
- SQL guardrails: blocks DDL, unguarded DELETE/UPDATE, COPY PROGRAM, session SET
- Automatic LIMIT injection for unbounded SELECT queries
- Streaming JSON and CSV row encoder with OID-specific fast paths
- ~200 heuristic column description patterns for agent-readable schema discovery
- propose_migration tool with lock analysis, downtime risk, and reversibility assessment
- suggest_index tool with EXPLAIN plan walking and CREATE INDEX CONCURRENTLY suggestions
- Hand-rolled McpError with 12 error codes and agent-friendly hints
- TOML configuration with environment variable overrides (PGMCP_* prefix)
- Docker multi-stage build (scratch base, ~13 MB static binary)
- CI pipeline: fmt, clippy, test matrix (PostgreSQL 14/15/16/17), cargo-deny, benchmarks
- 464+ unit tests and 100+ integration tests via testcontainers

### Compatibility

- PostgreSQL 14, 15, 16, 17
- Rust 1.88+ (edition 2024)

[0.1.0]: https://github.com/tideandiron/pgmcp/releases/tag/v0.1.0
