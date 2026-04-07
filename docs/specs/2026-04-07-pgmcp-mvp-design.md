# pgmcp MVP Design Specification

**Version:** MVP (M1 + M2)
**Date:** 2026-04-07
**Status:** Approved

---

## Table of Contents

1. [Overview](#1-overview)
2. [Design Principles](#2-design-principles)
3. [System Architecture](#3-system-architecture)
4. [MVP Tool Surface](#4-mvp-tool-surface)
5. [Project Structure](#5-project-structure)
6. [Working Process](#6-working-process)
7. [Code Quality Standards](#7-code-quality-standards)
8. [Feature Branch Sequence](#8-feature-branch-sequence)
9. [Dependency Manifest](#9-dependency-manifest)
10. [CI Pipeline](#10-ci-pipeline)
11. [Agents and Tooling](#11-agents-and-tooling)
12. [What We Exclude (and Why)](#12-what-we-exclude-and-why)

---

## 1. Overview

pgmcp is a Rust MCP (Model Context Protocol) server for PostgreSQL. It is the fastest, most capable way for an AI agent to interact with Postgres.

**Performance goal:** Zero overhead over raw PostgreSQL response — as close to zero penalty as the physics of computers will allow. No intermediate deserialization, no ORM mapping, no reflection. Rows leave Postgres and reach the MCP client with the minimum number of copies the hardware allows.

**Postgres-only.** pgmcp does not support Redis, ClickHouse, MySQL, SQLite, or any other database. This is a deliberate constraint, not a limitation. Being Postgres-only means the implementation can exploit every Postgres-specific feature: pg_catalog introspection, LISTEN/NOTIFY, advisory locks, row-level security, logical replication slots, extended query protocol, and server-side cursors.

**License:** Apache 2.0.

**Go-to-market role:** pgmcp is the open-source GTM engine for the broader AgentDB cloud product. OSS users get a production-grade MCP server; AgentDB extends it with multi-tenancy, branching, and managed infrastructure.

**MVP scope:** M1 (Core) + M2 (Schema + Intelligence). M3 (LISTEN/NOTIFY, cancel) and cloud-only features (search_schema, cross_query, branch_*) are explicitly out of scope.

---

## 2. Design Principles

### 1. Postgres is the product

Every feature exists to expose Postgres capability to agents. When a design decision requires choosing between a generic abstraction and a Postgres-specific one, the Postgres-specific one wins. The schema cache reads pg_catalog directly. The type mapper understands OIDs. The SQL analysis layer knows Postgres syntax.

### 2. Zero overhead

Measurement defines this principle. If a feature adds latency or allocation that does not appear in a benchmark, it does not exist. Pre-allocate write buffers. Reuse them. Stream rows without collecting. Do not deserialize what does not need to be deserialized. The hot path contains no heap allocations beyond what tokio-postgres itself performs.

### 3. Agents are the user, not developers

The tool surface is designed for LLM consumption. Tool names are unambiguous English verbs. Return types are JSON structured for agent readability. Error messages are written to be interpretable by a model, not a human reading a stack trace. Descriptions on tools and parameters are part of the product.

### 4. Intentional exclusion is a feature

Every feature not in pgmcp is a decision, not an omission. No migration framework, no ORM, no GUI, no plugin API, no multi-database. Scope discipline is what makes the zero-overhead goal achievable and the codebase maintainable.

---

## 3. System Architecture

### 3.1 Canonical Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                          pgmcp process                              │
│                                                                     │
│  ┌─────────────────┐                                                │
│  │  Startup Gate   │  validates config, runs preflight checks,     │
│  │                 │  refuses to start on fatal misconfiguration    │
│  └────────┬────────┘                                                │
│           │                                                         │
│           ▼                                                         │
│  ┌─────────────────────────────────────────┐                       │
│  │              Transports                  │                       │
│  │  ┌──────────────┐  ┌──────────────────┐ │                       │
│  │  │  stdio (MCP) │  │  SSE (HTTP/MCP)  │ │                       │
│  │  └──────┬───────┘  └────────┬─────────┘ │                       │
│  └─────────┼──────────────────┼────────────┘                       │
│            │                  │                                     │
│            └────────┬─────────┘                                     │
│                     │ JSON-RPC 2.0 messages                         │
│                     ▼                                               │
│           ┌──────────────────┐                                      │
│           │      rmcp        │  MCP protocol state machine,        │
│           │  (protocol layer)│  capability negotiation,            │
│           └────────┬─────────┘  request/response routing           │
│                    │                                                │
│                    ▼                                                │
│           ┌──────────────────┐                                      │
│           │    Dispatcher    │  routes tool calls to handlers,     │
│           │                  │  injects ToolContext,               │
│           └────────┬─────────┘  enforces concurrency limits        │
│                    │                                                │
│          ┌─────────┴──────────────────────────────────┐            │
│          │              Tool Handlers                   │            │
│          │  (one per tool, 15 total, plus query_events.rs helper)  │            │
│          │  pub(crate) async fn handle_*(ctx, params)  │            │
│          └──────────────────┬──────────────────────────┘            │
│                             │                                       │
│          ┌──────────────────┼───────────────────────┐              │
│          │                  │                        │              │
│          ▼                  ▼                        ▼              │
│  ┌───────────────┐  ┌──────────────────┐  ┌────────────────────┐  │
│  │ SQL Analysis  │  │  Schema Cache    │  │   Connection Pool  │  │
│  │ + Guardrails  │  │  (pg_catalog     │  │  (deadpool-        │  │
│  │               │  │   snapshots)     │  │   postgres)        │  │
│  │ - parse stmt  │  │                  │  │                    │  │
│  │ - classify    │  │ - table defs     │  │ - min/max conns    │  │
│  │ - inject LIMIT│  │ - column types   │  │ - health checks    │  │
│  │ - reject DDL  │  │ - enum values    │  │ - acquire timeout  │  │
│  └───────┬───────┘  └────────┬─────────┘  └────────┬───────────┘  │
│          │                   │                      │               │
│          └───────────────────┴──────────────────────┘              │
│                                          │                          │
│                                          ▼                          │
│                               ┌─────────────────────┐              │
│                               │     PostgreSQL       │              │
│                               │  (tokio-postgres,    │              │
│                               │   extended protocol) │              │
│                               └─────────┬────────────┘              │
│                                         │                           │
│                                         ▼                           │
│                          ┌──────────────────────────┐              │
│                          │  Streaming + Serialization│              │
│                          │                           │              │
│                          │  - BatchSizer (adaptive)  │              │
│                          │  - json.rs (row encoder)  │              │
│                          │  - csv.rs (row encoder)   │              │
│                          │  - zero-copy where OID    │              │
│                          │    mapping permits        │              │
│                          └──────────────────────────┘              │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Background Tasks                                            │   │
│  │  ┌──────────────────────────────────────────────────────┐   │   │
│  │  │  Cache Invalidation Task                              │   │   │
│  │  │  polls pg_catalog for schema changes at interval,    │   │   │
│  │  │  triggers cache refresh on detected changes          │   │   │
│  │  └──────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Observability                                               │   │
│  │  tracing spans on every tool call, pool events, SQL parse   │   │
│  │  structured JSON logs, configurable RUST_LOG filter         │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 Component Descriptions

#### Startup Gate

**What it does:** Reads and validates configuration, resolves environment variables, runs preflight connectivity checks against Postgres, initializes telemetry, constructs shared state (pool, schema cache), and either proceeds to serve or exits with a diagnostic error code.

**Depends on:** config.rs, telemetry.rs, pg/pool.rs, pg/cache.rs

**Depended on by:** main.rs only

**Constraints:** Must complete before any transport accepts connections. Any fatal error (invalid config, unreachable Postgres, version too old) must produce a human-readable message to stderr and exit non-zero.

---

#### Transports

**What it does:** Accepts MCP connections via two mechanisms. `stdio` reads newline-delimited JSON-RPC from stdin and writes to stdout — the canonical MCP transport for process-launched servers. `sse` listens on a configurable HTTP port, accepts SSE connections for server-to-client streaming and POST for client-to-server messages — the canonical MCP transport for network-accessible servers.

**Depends on:** rmcp (hands off to), axum (for SSE transport)

**Depended on by:** Nothing; transports are entry points.

**Constraints:** Only one transport is active per process instance. Transport selection is determined at startup from config. Both transports produce identical MCP message streams to rmcp.

---

#### rmcp (Protocol Layer)

**What it does:** Implements the MCP protocol state machine. Handles capability negotiation (tools/list, initialization handshake), request/response correlation, progress notifications, and error framing per the MCP spec. pgmcp uses the rmcp crate; this layer is not custom code.

**Depends on:** transports (message source/sink)

**Depended on by:** Dispatcher

**Constraints:** rmcp version is pinned. pgmcp does not fork or patch rmcp; any required behavior is achieved through the rmcp API surface.

---

#### Dispatcher

**What it does:** Receives decoded tool-call requests from rmcp. Validates that the tool name is known. Constructs a `ToolContext` (injects Arc clones of pool, schema cache, config). Routes to the appropriate handler function. Enforces a per-connection concurrency limit. Returns handler results or error responses back to rmcp.

**Depends on:** rmcp, ToolContext, all tool handlers

**Depended on by:** rmcp (calls into it)

**Constraints:** No business logic in the dispatcher. It is a routing table and context factory. Unknown tool names return a structured McpError with code `tool_not_found`. Concurrency limit prevents pool exhaustion under adversarial load.

---

#### Tool Handlers

**What it does:** Fifteen handler functions, one per tool. Each function signature is `pub(crate) async fn handle_<tool>(ctx: ToolContext, params: serde_json::Value) -> Result<ToolResponse, McpError>`. Each handler is responsible for: parameter parsing and validation, acquiring a pool connection if needed, executing the operation, serializing the result.

**Depends on:** ToolContext (pool, cache, config), SQL Analysis + Guardrails (for SQL-accepting tools), pg/queries/*.sql

**Depended on by:** Dispatcher

**Constraints:** No handler may hold a pool connection across an await point it does not control. All handlers are `pub(crate)`, never `pub`. No handler may call `tokio::spawn` unless the operation is explicitly fire-and-forget.

---

#### SQL Analysis + Guardrails

**What it does:** Parses SQL statements using the `sqlparser` crate with Postgres dialect. Classifies statement kind (SELECT, INSERT, UPDATE, DELETE, DDL, CALL, COPY, etc.) via the `StatementKind` enum. Validates parameters (rejects multi-statement input, validates identifiers). Injects a LIMIT clause when the statement is a SELECT and no LIMIT is present and the caller has not explicitly opted out. Enforces guardrail policies (e.g., blocks DDL when the tool contract prohibits it, blocks COPY TO/FROM PROGRAM).

**Depends on:** sqlparser crate

**Depended on by:** query handler, explain handler, propose_migration handler, suggest_index handler

**Constraints:** SQL analysis never executes SQL. It is a pure transformation/validation pass. The guardrail decision is made before any pool connection is acquired. Guardrail rejections return `McpError` with code `guardrail_violation` and a message describing why the statement was rejected.

---

#### Connection Pool

**What it does:** Manages a bounded pool of tokio-postgres connections to the target Postgres instance. Provides connection acquisition with timeout, health checking, and connection recycling. Configured with min/max pool size, acquire timeout, and idle timeout from pgmcp config.

**Depends on:** deadpool-postgres, tokio-postgres

**Depended on by:** All tool handlers that execute SQL, Schema Cache, Cache Invalidation Task

**Constraints:** Pool size is bounded. Acquisition timeout is mandatory — no handler may wait indefinitely. Pool configuration is validated at startup; a pool that cannot reach min connections at startup is a fatal error by default (configurable). The pool is shared via `Arc`; handlers clone the Arc, they do not borrow it.

---

#### PostgreSQL

**What it does:** The target database. pgmcp connects via the extended query protocol (prepared statements, binary wire format where beneficial). All pg_catalog queries use the extended protocol for parameter binding. Row streaming uses tokio-postgres's `query_raw` to avoid collecting rows into a Vec.

**Depends on:** External; the Postgres server is not part of the pgmcp process.

**Depended on by:** Connection Pool

**Constraints:** Minimum supported Postgres version: 14. Version is checked at startup via `SHOW server_version`. Features gated by version (e.g., pg_catalog views added in PG 15) must use version-conditional queries.

---

#### Streaming + Serialization

**What it does:** Converts tokio-postgres row streams into the output format requested by the caller (JSON or CSV). `BatchSizer` computes an adaptive batch size based on measured row size to keep memory usage bounded while minimizing round-trip count. `json.rs` encodes rows directly from tokio-postgres column values to JSON bytes without intermediate deserialization into Rust types where the OID mapping permits. `csv.rs` provides equivalent CSV encoding. The 64KB write buffer is pre-allocated and reused across rows in a single query.

BatchSizer specification:
  - Initial batch size: 100 rows
  - After encoding the first batch, compute avg_row_bytes = total_encoded_bytes / row_count
  - Subsequent batch size: clamp(target_event_bytes / avg_row_bytes, 1, 1000)
    where target_event_bytes = 65536 (64KB, matching the write buffer size)
  - Recalculate after every batch to adapt to variable-width rows
  - For result sets < 100 rows: send all rows in a single batch
  - Adaptation is per-query, reset on each new tool call

**Depends on:** tokio-postgres (row type), pg/types.rs (OID mappings), bytes, ryu (for float formatting)

**Depended on by:** query handler

**Constraints:** The hot loop (per-row encoding) must contain no heap allocations beyond the pre-allocated write buffer growth. `BatchSizer` tuning is driven by benchmarks, not intuition. CSV and JSON encoders must produce identical row counts for the same query.

---

#### Schema Cache

**What it does:** Holds an in-memory snapshot of pg_catalog data for the connected database: table definitions, column names and types, enum values, extension list, schema list. Snapshots are keyed by schema + object name. Cache entries are `Arc`-wrapped to allow zero-copy reads from multiple concurrent handlers.

**Depends on:** Connection Pool (for initial load and refresh), pg/queries/*.sql

**Depended on by:** Discovery tools (list_tables, describe_table, list_enums, etc.), Cache Invalidation Task, infer.rs

**Constraints:** The cache is populated synchronously during startup before the server accepts any tool calls. All cache reads are lock-free reads of an `Arc<RwLock<CacheInner>>` where the write lock is held only during refresh. Cache entries are never mutated in place; refresh replaces the inner Arc. Stale reads during refresh are acceptable; partial reads (seeing old schema for table A and new schema for table B) are not — the entire refresh is atomic from the caller's perspective.

---

#### Cache Invalidation Background Task

**What it does:** Polls pg_catalog at a configurable interval (default: 30 seconds) to detect schema changes. Polls using: `SELECT max(xact_commit) FROM pg_stat_database WHERE datname = current_database()`. If the transaction commit count has increased since last poll, run a full schema snapshot comparison: `SELECT oid, relname, relkind, reltuples FROM pg_class WHERE relnamespace NOT IN (SELECT oid FROM pg_namespace WHERE nspname IN ('pg_catalog', 'information_schema', 'pg_toast'))`. Compare against cached snapshot; invalidate entries where oid set, reltuples, or relname changed. On detecting changes, triggers a full cache refresh via the Schema Cache. Runs as a `tokio::spawn`'d task that holds a weak reference to the cache; if the cache is dropped (process shutdown), the task exits cleanly.

**Depends on:** Connection Pool, Schema Cache

**Depended on by:** Nothing; it is a background task.

**Constraints:** The polling connection must not occupy a pool slot permanently. It acquires and releases a connection per poll cycle. The task must not panic; any error during a poll cycle is logged and the next cycle is attempted. The task does not signal failure to the caller; the worst case is stale cache data, not a crash.

---

#### Observability

**What it does:** Structured tracing spans and events at every significant boundary: tool call entry/exit (with tool name, duration, success/failure), SQL parse result (statement kind, guardrail decision), pool events (acquire, release, connection error), cache events (hit, miss, refresh). Log level is controlled by `RUST_LOG`. Log format is structured JSON in production, human-readable in development (detected by `RUST_LOG` or explicit config flag).

**Depends on:** tracing, tracing-subscriber

**Depended on by:** All components (via `tracing::instrument` and manual span/event calls)

**Constraints:** Observability must have zero measurable overhead on the hot path when the trace level is filtered out. No log line in the hot path may allocate a String unless the log level is enabled. Use `tracing::enabled!` guards before constructing expensive log payloads.

---

### 3.3 Data Flows

#### describe_table

1. Dispatcher receives `tools/call` for `describe_table` with params `{schema, table}`.
2. Dispatcher constructs `ToolContext` and calls `handle_describe_table(ctx, params)`.
3. Handler deserializes params, validates schema and table are non-empty and contain no SQL metacharacters.
4. Handler calls `ctx.cache.get_table(schema, table)`.
5. Cache hit: returns `Arc<TableDef>` immediately, no SQL executed.
6. Cache miss: handler acquires a pool connection, executes `pg/queries/describe_table.sql` with schema and table as parameters, builds `TableDef`, populates cache, releases connection.
7. Handler serializes `TableDef` to `ToolResponse` JSON.
8. Handler returns `Ok(ToolResponse)` to dispatcher.
9. Dispatcher returns result to rmcp for framing and delivery to transport.

#### Streaming Query

1. Dispatcher receives `tools/call` for `query` with params including `sql`, `limit`, `format`, and optional `timeout_seconds`.
2. Handler deserializes params.
3. SQL Analysis parses the statement, classifies as SELECT, injects LIMIT if absent and `limit` param is set.
4. Guardrails validate the statement (no DDL, no COPY TO PROGRAM, etc.).
5. Handler acquires a pool connection.
6. Handler begins a tokio-postgres `query_raw` stream with the validated SQL.
7. `BatchSizer` determines initial batch size.
8. Streaming encoder (json.rs or csv.rs) encodes rows from the tokio-postgres row stream into the pre-allocated write buffer.
9. Completed batches are emitted as MCP progress notifications (for large result sets) or accumulated for a single response (for small result sets).
10. On stream exhaustion or timeout, handler releases the pool connection and returns final `ToolResponse` with row count and format metadata.
11. On error at any step after connection acquisition, handler releases the connection before propagating the McpError.

#### propose_migration

1. Dispatcher receives `tools/call` for `propose_migration` with params `{intent, context_tables}`.
2. Handler deserializes params. Validates `intent` is non-empty. Validates `context_tables` are known (via schema cache lookup).
3. Handler queries schema cache for full definitions of all `context_tables`.
4. Handler acquires a pool connection and queries `pg_catalog` for existing indexes, constraints, and foreign key relationships on the context tables.
5. Handler constructs a structured analysis of the current schema state.
6. Handler applies heuristic rules (see section 4.5 for pattern categories) to generate candidate migration statements (CREATE INDEX, ALTER TABLE, etc.).
7. Handler runs each candidate statement through SQL Analysis (parse + validate) — any statement that does not parse cleanly is dropped with a log warning.
8. Handler serializes the candidate migration statements with explanatory notes as `ToolResponse`.
9. Handler releases pool connection.
10. Handler returns `Ok(ToolResponse)`.

**Note:** `propose_migration` never executes migration SQL against the database. It proposes; the agent or user decides whether to execute.

---

### 3.4 Startup Sequence

1. Parse command-line arguments (transport mode, config path).
2. Load config from file + environment variable overrides. Validate all required fields are present and well-formed. Exit with code 2 on config error.
3. Initialize telemetry (tracing subscriber, log format). This must succeed before any further logging.
4. Check Postgres connectivity: attempt to connect once using the configured connection string. Exit with code 3 on failure, with a diagnostic message to stderr.
5. Check Postgres version: execute `SHOW server_version`. Parse version. Exit with code 4 if version < 14, with a message identifying the detected version and minimum required.
6. Initialize connection pool. Establish min connections. Exit with code 5 if pool initialization fails. Pool initialization failure at this step indicates a misconfiguration (e.g., min_connections exceeds Postgres max_connections), not a connectivity failure — connectivity was verified in step 4.
7. Perform initial schema cache load: execute all pg_catalog queries, populate cache. This is synchronous and blocks startup.
8. Spawn cache invalidation background task.
9. Bind transport (open stdin/stdout for stdio, bind HTTP port for SSE). Exit with code 6 if bind fails (e.g., port already in use).
10. Begin serving requests.

---

### 3.5 Error Propagation Model

All errors in pgmcp propagate as `McpError`. `McpError` is the single error type for all fallible operations. It is defined in `error.rs` and is the only error type that crosses module boundaries.

`McpError` carries:
- A machine-readable error code (see table below)
- A human-readable message suitable for returning to an agent
- An optional source error (for logging; never sent to the agent)

**Error Code Table**

| Code | Meaning | Typical Cause |
|---|---|---|
| `config_invalid` | Configuration is malformed or missing required fields | Bad env var, missing file |
| `pg_connect_failed` | Could not connect to Postgres | Wrong host, firewall, bad credentials |
| `pg_version_unsupported` | Postgres version < 14 | Old server |
| `pg_query_failed` | SQL execution error from Postgres | Syntax error, permission denied, constraint violation |
| `pg_pool_timeout` | Could not acquire connection within timeout | Pool exhausted, slow queries |
| `tool_not_found` | Unknown tool name in tool call | Agent typo, version mismatch |
| `param_invalid` | Tool parameter missing, wrong type, or failed validation | Agent error |
| `guardrail_violation` | SQL blocked by analysis layer | DDL in query tool, COPY TO PROGRAM |
| `sql_parse_error` | SQL statement did not parse | Malformed SQL from agent |
| `schema_not_found` | Schema does not exist | Agent named wrong schema |
| `table_not_found` | Table does not exist in specified schema | Agent named wrong table |
| `internal` | Unexpected error with no more specific code | Bug; should be rare |

**Propagation rules:**
- tokio-postgres errors are wrapped into `pg_query_failed` or `pg_connect_failed` at the pool/handler boundary. Raw postgres errors are logged but not forwarded to the agent.
- Validation errors in handlers become `param_invalid`.
- Guardrail rejections become `guardrail_violation`.
- Any `unwrap()` or `expect()` that fires is a bug; all known failure modes have explicit error codes.
- `internal` is the catch-all of last resort; its presence in production logs indicates a bug to be filed and fixed.

---

### 3.6 Architectural Invariants

The following invariants must hold at all times. A PR that violates any invariant must be rejected in review.

1. **No SQL is executed without passing through SQL Analysis + Guardrails first**, for any tool that accepts a SQL string as input. Discovery tools that construct their own SQL from pg_catalog queries are exempt because they never execute caller-supplied SQL.

2. **Pool connections are never held across uncontrolled await points.** A handler may hold a connection across awaits that are part of the same logical operation (e.g., streaming rows from a query). A handler must not hold a connection while awaiting a channel, a lock, or a sleep.

3. **The schema cache is always in a consistent state.** Concurrent reads see either the previous full snapshot or the new full snapshot, never a partial mix of the two.

4. **All public-API errors are McpError.** No `anyhow::Error`, no `Box<dyn Error>`, no raw `String` errors escape from any module boundary.

5. **The hot path (row encoding) contains no blocking operations.** No file I/O, no synchronous locks, no DNS lookups. Every operation in the streaming path is either purely computational or async.

6. **No global mutable state.** No `static mut`, no `lazy_static!`, no `once_cell::sync::Lazy` holding mutable state. All shared state is passed through `Arc` clones injected via `ToolContext`.

7. **Transport-layer behavior is identical.** The stdio and SSE transports must produce byte-for-byte identical MCP responses for identical tool calls. No tool handler may be transport-aware.

---

## 4. MVP Tool Surface

The MVP exposes 15 tools in three categories.

### 4.1 Discovery Tools

Read from `pg_catalog`. Results are served from the schema cache where available. These tools never execute caller-supplied SQL.

---

#### `list_databases`

**What it does:** Returns the list of databases visible to the connected role on the Postgres instance.

**Parameters:** None.

**Returns:** Array of objects `{name: string, owner: string, encoding: string, size_bytes: number | null}`.

---

#### `server_info`

**What it does:** Returns Postgres server version, server settings relevant to agent interaction (statement_timeout, max_connections, work_mem, shared_buffers), and the connected role.

**Parameters:** None.

**Returns:** Object `{version: string, version_num: number, settings: Record<string, string>, role: string}`.

---

#### `list_schemas`

**What it does:** Returns all schemas in the current database visible to the connected role, excluding `pg_toast` and `pg_temp_*` internal schemas.

**Parameters:** None.

**Returns:** Array of objects `{name: string, owner: string}`.

---

#### `list_tables`

**What it does:** Returns tables, views, and materialized views in a schema.

**Parameters:**
- `schema` (string, required): schema name
- `kind` (string, optional, default `"table"`): one of `"table"`, `"view"`, `"materialized_view"`, `"all"`

**Returns:** Array of objects `{schema: string, name: string, kind: string, row_estimate: number | null, description: string | null}`.

---

#### `describe_table`

**What it does:** Returns the full definition of a table: columns (name, type, nullable, default, comment), primary key, unique constraints, foreign keys, indexes, and check constraints.

**Parameters:**
- `schema` (string, required)
- `table` (string, required)

**Returns:** Object with fields `columns`, `primary_key`, `unique_constraints`, `foreign_keys`, `indexes`, `check_constraints`. Column objects include `{name, type, nullable, default, comment}`.

---

#### `list_enums`

**What it does:** Returns all enum types defined in a schema with their ordered label values.

**Parameters:**
- `schema` (string, optional, default `"public"`): schema name

**Returns:** Array of objects `{schema: string, name: string, values: string[]}`.

---

#### `list_extensions`

**What it does:** Returns all extensions installed in the current database.

**Parameters:** None.

**Returns:** Array of objects `{name: string, version: string, schema: string, description: string | null}`.

---

#### `table_stats`

**What it does:** Returns runtime statistics for a table from `pg_stat_user_tables` and `pg_class`: row estimate, live/dead tuple counts, sequential scan count, index scan count, last vacuum, last analyze, table size.

**Parameters:**
- `schema` (string, required)
- `table` (string, required)

**Returns:** Object `{schema, table, row_estimate, live_tuples, dead_tuples, seq_scans, idx_scans, last_vacuum, last_analyze, table_size_bytes, toast_size_bytes, index_size_bytes}`.

---

### 4.2 SQL-Accepting Tools

All SQL-accepting tools pass the caller-supplied SQL through SQL Analysis + Guardrails before execution. Guardrail violations return `McpError` with code `guardrail_violation` before any pool connection is acquired.

---

#### `query`

**What it does:** Executes a SQL query and returns results. The primary workhorse tool for agent data access.

**Parameters:**
- `sql` (string, required): the SQL statement to execute
- `intent` (string, optional): natural language description of what the agent is trying to accomplish — used for logging and observability, not for query modification
- `transaction` (boolean, optional, default `false`): if true, wrap the statement in an explicit transaction that is rolled back after execution (for dry-run inspection of DML effects without committing)

The `transaction` parameter does not affect guardrail evaluation. DDL statements remain blocked by the query tool's guardrail policy regardless of the `transaction` parameter value. The `transaction` parameter applies only to DML (INSERT, UPDATE, DELETE).

- `dry_run` (boolean, optional, default `false`): if true, parse and analyze the statement but do not execute it; returns the parsed statement kind and guardrail analysis
- `limit` (number, optional, default `1000`): maximum rows to return; injected as LIMIT clause if not present in the SQL
- `timeout_seconds` (number, optional, default from config): statement timeout applied via `SET LOCAL statement_timeout`
- `format` (string, optional, default `"json"`): output format, one of `"json"` or `"csv"`
- `explain` (boolean, optional, default `false`): if true, prepend `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` to the statement and return the plan alongside results

When `explain: true`, the server runs `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` wrapping the original SQL. The plan is returned alongside the result rows from the same execution — ANALYZE executes the statement, so rows are produced as a side effect. The `plan` object is identical in structure to the `explain` tool's response. Use the standalone `explain` tool when you want plan inspection without row results or when you want `analyze: false` (plan estimation without execution). Use `query` with `explain: true` when you want both data and the execution plan.

**Returns:** Object `{rows: array | string, row_count: number, format: string, columns: {name, type}[], truncated: boolean, execution_ms: number | null, plan: object | null}`.

---

#### `explain`

**What it does:** Runs `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` on a SQL statement and returns the query plan with execution statistics. Does not return result rows.

**Parameters:**
- `sql` (string, required): the SQL statement to explain
- `analyze` (boolean, optional, default `true`): if false, runs `EXPLAIN` without `ANALYZE` (no execution, estimates only)
- `buffers` (boolean, optional, default `true`): include buffer usage statistics (requires `analyze: true`)

**Returns:** Object `{plan: object, planning_ms: number | null, execution_ms: number | null}`. `plan` is the raw Postgres EXPLAIN JSON output.

---

#### `suggest_index`

**What it does:** Analyzes a SQL statement and the current index state of the referenced tables, and proposes indexes that would improve the query. Uses heuristic rules, not external LLM calls.

**Parameters:**
- `sql` (string, required): the SQL statement to analyze
- `schema` (string, optional, default `"public"`): schema context for unqualified table references

**Returns:** Object `{suggestions: [{ddl: string, rationale: string, estimated_benefit: string}], existing_indexes: [{table, name, columns, type}]}`.

---

#### `propose_migration`

**What it does:** Given a description of intent and a set of context tables, proposes a migration (CREATE TABLE, ALTER TABLE, CREATE INDEX, etc.) as a set of SQL statements with explanations. Uses heuristic patterns. Does not execute any SQL.

**Parameters:**
- `intent` (string, required): natural language description of what the migration should accomplish
- `context_tables` (string[], optional): table names (schema-qualified or unqualified) to include as context
- `schema` (string, optional, default `"public"`): default schema for unqualified names

**Returns:** Object `{statements: [{sql: string, explanation: string, reversible: boolean, reverse_sql: string | null}], warnings: string[]}`.

---

### 4.3 Introspection Tools

---

#### `my_permissions`

**What it does:** Introspects the actual Postgres role connected by pgmcp and reports its privileges. Uses `pg_roles`, `has_table_privilege()`, `has_schema_privilege()`, and related catalog functions. Reports what the role can and cannot do, so the agent knows what operations are safe to attempt.

**Parameters:**
- `schema` (string, optional, default `"public"`): schema to introspect privileges for
- `table` (string, optional): if specified, include table-level privilege detail for this table

**Returns:** Object `{role: string, superuser: boolean, create_db: boolean, schemas: [{name, usage, create}], tables: [{name, select, insert, update, delete}] | null}`.

---

#### `connection_info`

**What it does:** Returns information about the current pgmcp connection to Postgres: host, port, database, role, SSL status, server version, pool stats (total, idle, in-use connections).

**Parameters:** None.

**Returns:** Object `{host: string, port: number, database: string, role: string, ssl: boolean, server_version: string, pool: {total: number, idle: number, in_use: number}}`.

---

#### `health`

**What it does:** Liveness and readiness check. Verifies that pgmcp can acquire a pool connection and execute a trivial query (`SELECT 1`). Intended for use by orchestration infrastructure.

**Parameters:** None.

**Returns:** Object `{status: "ok" | "degraded" | "unhealthy", pool_available: boolean, pg_reachable: boolean, schema_cache_age_seconds: number, latency_ms: number}`.

---

### 4.4 Tools Not in MVP

The following tools are explicitly excluded from MVP scope:

- `cancel` — cancels an in-flight query by backend PID. Deferred to M3.
- `listen` / `notify` — LISTEN/NOTIFY support. Deferred to M3.
- `search_schema` — semantic/full-text schema search. Cloud-only feature.
- `cross_query` — federated query across multiple databases. Cloud-only feature.
- `branch_*` — database branching operations. Cloud-only feature.

### 4.5 Inferred Descriptions

The `describe_table` and `propose_migration` tools include heuristically inferred descriptions for tables and columns that lack explicit `COMMENT ON` annotations. These inferences use a set of heuristic patterns covering at minimum the following categories: foreign key conventions (*_id → 'Reference to {table}'), timestamp conventions (*_at, *_on → 'Timestamp when...'), boolean conventions (is_*, has_* → 'Whether...'), status/state enums, monetary amounts (*_cents, *_amount), email/phone/url detection, counter/sequence fields, JSON/JSONB metadata columns, array columns, and size/measurement fields (*_bytes, *_count, *_size). The pattern set is expected to reach 150-250 entries; the exact count is an implementation outcome, not a target.

This inference is not LLM-based. It is a pure Rust function in `pg/infer.rs` that maps column names and types to description strings using pattern matching. The pattern set is version-controlled and reviewable. Inferred descriptions are marked as inferred in the output so the agent can distinguish them from explicit database comments.

---

## 5. Project Structure

### 5.1 Crate Layout

```
pgmcp/
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── error.rs
│   ├── telemetry.rs
│   ├── transport/
│   │   ├── sse.rs
│   │   └── stdio.rs
│   ├── server/
│   │   ├── mod.rs
│   │   ├── router.rs
│   │   ├── context.rs
│   │   └── tool_defs.rs
│   ├── tools/
│   │   ├── mod.rs
│   │   ├── list_databases.rs
│   │   ├── server_info.rs
│   │   ├── list_schemas.rs
│   │   ├── list_tables.rs
│   │   ├── describe_table.rs
│   │   ├── list_enums.rs
│   │   ├── list_extensions.rs
│   │   ├── table_stats.rs
│   │   ├── query.rs
│   │   ├── explain.rs
│   │   ├── suggest_index.rs
│   │   ├── propose_migration.rs
│   │   ├── my_permissions.rs
│   │   ├── connection_info.rs
│   │   ├── health.rs
│   │   └── query_events.rs         # helper: SSE event construction for query tool (not a standalone tool)
│   ├── sql/
│   │   ├── mod.rs
│   │   ├── parser.rs
│   │   ├── limit.rs
│   │   └── guardrails.rs
│   ├── pg/
│   │   ├── mod.rs
│   │   ├── pool.rs
│   │   ├── types.rs
│   │   ├── cache.rs
│   │   ├── invalidation.rs
│   │   ├── infer.rs
│   │   └── queries/
│   │       ├── list_databases.sql
│   │       ├── server_settings.sql
│   │       ├── list_schemas.sql
│   │       ├── list_tables.sql
│   │       ├── describe_table.sql
│   │       ├── list_enums.sql
│   │       ├── list_extensions.sql
│   │       ├── table_stats.sql
│   │       └── my_permissions.sql
│   └── streaming/
│       ├── mod.rs
│       ├── json.rs
│       └── csv.rs
├── tests/
│   ├── common/
│   │   ├── mod.rs
│   │   └── fixtures.rs
│   └── integration/
│       ├── discovery.rs
│       ├── query.rs
│       ├── streaming.rs
│       ├── guardrails.rs
│       ├── schema_cache.rs
│       ├── permissions.rs
│       ├── health.rs
│       └── migration.rs
├── benches/
│   ├── serialization.rs
│   ├── streaming.rs
│   └── connection.rs
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── deny.toml
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── release.yml
├── docker/
│   └── Dockerfile
└── config/
    └── pgmcp.example.toml
```

### 5.2 Design Rationale for Key Structural Decisions

**One file per tool.** Each of the 15 tools lives in its own file in `tools/`. This makes it easy to find, read, and review any single tool in isolation. A tool file is expected to fit within approximately 300 lines. When a tool grows past that, the handler is likely doing too much and should be refactored.

**`sql/` separate from `tools/`.** The SQL analysis and guardrail logic is a shared layer used by multiple tools (`query`, `explain`, `suggest_index`, `propose_migration`). Placing it in `tools/` would create inter-tool dependencies. Placing it in `pg/` would conflate SQL parsing (a language concern) with Postgres connectivity (a network/driver concern). `sql/` is its own module with a clear contract: it takes a SQL string, returns a classification or an error, and never touches the network.

**`pg/` owns all PostgreSQL-specific concerns.** The `pg/` module is the boundary between pgmcp logic and the Postgres driver. Connection pooling, OID-to-type mapping, schema cache, cache invalidation, and column name inference all live here. Nothing outside `pg/` imports `tokio_postgres` directly. This boundary makes it possible to swap the driver or pool implementation (see the deadpool-postgres deferred decision) without touching tool handlers.

**`ToolContext` owns `Arc` clones, not references.** `ToolContext` is constructed in the dispatcher and passed by value into each handler. It holds `Arc<Pool>`, `Arc<SchemaCache>`, and `Arc<Config>`. It does not hold references with lifetimes. This design eliminates lifetime parameters from all 15 handler signatures, making them easier to read and easier to store in data structures if needed. The cost is an `Arc::clone()` per tool call, which is negligible.

**`pub(crate)` for tool handlers.** Handler functions are not part of pgmcp's public API. They are callable from the dispatcher (same crate) but must not be reachable from external crates. `pub(crate)` enforces this at the compiler level and signals to reviewers that these are internal implementation details.

**`pg/queries/*.sql` with `include_str!`.** All pg_catalog queries are stored as `.sql` files embedded via `include_str!` at compile time. This enables syntax highlighting and linting in editors, makes queries reviewable as standalone files (auditable by a Postgres expert without reading Rust code), and keeps query strings out of the Rust source. The `include_str!` call site in `pg/` documents exactly which query corresponds to which operation.

---

## 6. Working Process

### 6.1 Branch Model

- **Sequential numbering:** branches follow the pattern `feat/NNN-short-description` where NNN is zero-padded to three digits (e.g., `feat/001-project-scaffold`, `feat/029-readme-contributing`).
- **One branch, one concern:** a branch addresses exactly one feature, fix, or structural change. If work requires two unrelated changes, it requires two branches.
- **Main always releasable:** main is never broken. CI must be green on main at all times. A failing main is a P0 incident.
- **No direct commits to main:** all changes arrive via pull request with the full review lifecycle completed.

### 6.2 Feature Branch Lifecycle

Each branch follows this nine-step lifecycle:

1. **PLAN** — Before writing code, the implementing agent produces a written plan: what the branch will do, what files it will touch, what tests it will add, and what dependencies it will add (if any). This plan is posted as a comment on the tracking issue or in the PR description.

2. **IMPLEMENT** — Write the code. Follow all code quality standards in section 7. Commit frequently with meaningful commit messages. The branch may have multiple commits before the PR is opened.

3. **SELF-TEST** — Before requesting review, the implementing agent runs the full local test suite (`cargo test`), clippy (`cargo clippy -- -D warnings`), and formatting (`cargo fmt --check`). Integration tests must run against a real Postgres instance. No PR is opened with known failures.

4. **REVIEW** — The reviewing agent applies the 12-point checklist (section 6.3). The review is thorough and opinionated. The reviewer may request changes on any checklist point.

5. **RESPOND** — The implementing agent addresses every review comment. Each comment is either resolved with a code change or responded to with a clear explanation of why no change is needed. Unresolved comments block merge.

6. **RE-REVIEW** — The reviewing agent reviews the updated code. Steps 5 and 6 iterate until the reviewer explicitly signals `APPROVED` in the review comment.

7. **APPROVE** — Eric reviews the approved PR. Eric's approval is required for merge. Eric may add additional requirements that restart the cycle.

8. **MERGE** — Squash merge to main. The squash commit message follows the format in section 6.5. The feature branch is deleted after merge.

9. **VERIFY** — CI runs on main after the merge. Green CI confirms the merge was clean. A failing CI after merge is treated as a P0 incident and the commit is reverted if not immediately fixable.

### 6.3 Review Checklist

Every PR must pass all 12 checklist points:

1. **Compiles clean** — `cargo build` with `#![deny(warnings)]` enabled produces no warnings.
2. **All tests pass** — `cargo test` passes, including integration tests against real Postgres.
3. **Clippy clean** — `cargo clippy -- -D warnings` produces no warnings.
4. **Formatted** — `cargo fmt --check` reports no differences.
5. **No new unsafe** — No `unsafe` block is introduced without explicit justification in the PR description and a comment in the code explaining the invariant that makes the unsafe code sound.
6. **No new deps without justification** — Every new dependency added to `Cargo.toml` is justified in the PR description: what it does, why an existing dep or hand-written code does not suffice, and what its maintenance status is.
7. **Error handling** — All fallible functions return `Result<T, McpError>`. No `.unwrap()` outside of test code. No `.expect()` in production code paths. No panics except on genuine programmer error.
8. **Test coverage** — New functionality is covered by integration tests against a real Postgres instance. Unit tests are acceptable for pure functions (SQL parser, serializer). Coverage is not measured by a tool; it is assessed by the reviewer reading the test files.
9. **Benchmarks don't regress** — If the PR touches the hot path (streaming encoder, SQL parser, pool acquisition), benchmark results must be included in the PR description. A regression of more than 5% blocks merge.
10. **Documentation** — All public-API items (structs, enums, trait impls, functions) have doc comments. Doc comments include an example where the usage is non-obvious.
11. **Commit message quality** — Each commit message explains why the change was made, not just what changed. The format in section 6.5 is followed.
12. **Code elegance** — The code is readable on first pass. Functions are not longer than approximately 50 lines. Files are not longer than approximately 300 lines. Control flow is linear (early returns, no deeply nested conditionals). Names are self-documenting. No premature abstraction.

### 6.4 Beyond-Code Review

In addition to the 12-point technical checklist, the reviewer evaluates:

- **CHANGELOG** — user-facing changes are logged.
- **Example config** — if config keys are added or changed, `config/pgmcp.example.toml` is updated.
- **README** — if the tool surface or behavior visible to users changes, the README is updated.
- **Integration tests** — new tools and new guardrail rules have integration tests. "It compiles" is not sufficient coverage.
- **No scope creep** — the PR does exactly what the branch plan described and nothing else. Extra improvements are split to a separate branch.

### 6.5 Commit Message Format

```
feat(NNN): short imperative description

Why: one or two sentences explaining the motivation for this change.
What: brief summary of the approach, if not obvious from the diff.

Reviewed-by: <reviewer agent identifier>
Co-Authored-By: <implementing agent name> <noreply@anthropic.com>
```

The subject line is imperative mood ("add pool timeout handling", not "added pool timeout handling" and not "adds pool timeout handling"). The subject line does not end in a period. The subject line is 72 characters or fewer.

### 6.6 Agent Roles

- **Implementing agent:** Executes PLAN, IMPLEMENT, SELF-TEST, RESPOND steps. Responsible for code quality. Uses the Rust engineer agent profile.
- **Reviewing agent:** Executes REVIEW and RE-REVIEW steps. Applies the 12-point checklist without mercy. Uses the code reviewer agent profile.
- **Eric (final approver):** Executes the APPROVE step. Has final authority on scope and design. May override the reviewing agent on subjective elegance questions. May not override safety or correctness findings.

---

## 7. Code Quality Standards

### 7.1 Error Handling

- All fallible functions return `Result<T, McpError>`.
- No `anyhow` or `eyre`. The error type is part of the product surface.
- `.unwrap()` is forbidden outside of `#[cfg(test)]` blocks.
- `.expect()` is forbidden in production code paths. If a condition is truly impossible to violate, encode it in the type system instead.
- Panics are bugs, not error handling. The only acceptable panics are from integer overflow in debug builds and from `unreachable!()` in match arms that are statically verified to be unreachable.

### 7.2 Naming Conventions

- Types: `PascalCase`
- Functions and methods: `snake_case`, verb-first (`get_table`, `parse_statement`, `encode_row`)
- Constants: `SCREAMING_SNAKE_CASE`
- Modules: `snake_case`
- Tool handler functions: `pub(crate) async fn handle_{tool_name}(ctx: ToolContext, params: serde_json::Value) -> Result<ToolResponse, McpError>`
- Acceptable abbreviations: `sql`, `pg`, `sse`, `id`, `ctx`. All other abbreviations are written out in full.

### 7.3 Types Over Stringly-Typing

- Fixed sets of values are `enum`s, not `String`s. Example: output format is `enum OutputFormat { Json, Csv }`, not `String`.
- Where ambiguity between identically-typed values causes bugs, use newtypes. Example: `SchemaName(String)` and `TableName(String)` are distinct types even though both wrap a `String`, preventing them from being swapped accidentally.
- Parse, don't validate. Raw strings from parameter input are converted to validated types at the boundary (in the handler's parameter parsing section). Once inside the handler, only validated types are used.

### 7.4 Async Discipline

- No `tokio::spawn` in handlers unless the operation is explicitly fire-and-forget and the spawned task's lifetime is well-understood.
- Use `tokio::join!` for intra-handler parallelism (e.g., fetching two independent schema cache entries concurrently).
- No `block_on` or `block_in_place`. If a blocking operation is genuinely required, `spawn_blocking` is used and the reason is documented.
- Minimize connection lease duration. Acquire the pool connection as late as possible. Release it (drop it) as early as possible, before any further async work that does not require the connection.

### 7.5 Dependencies

- Every new dependency is justified in the PR description (what it does, why existing code does not suffice, maintenance status).
- Major versions are pinned in `Cargo.toml`.
- `cargo deny check` runs in CI. The `deny.toml` config disallows known-vulnerable crates, duplicate versions of core deps, and non-permissive licenses.

### 7.6 Elegance Rules

1. **Function fits on one screen.** A function is approximately 50 lines or fewer. A function that exceeds this is a signal to decompose.
2. **File fits in context.** A source file is approximately 300 lines or fewer. A file that exceeds this is a signal to split the module.
3. **Linear control flow.** Use early returns and the `?` operator. Avoid deeply nested `if`/`match` blocks. The happy path is the leftmost code.
4. **Self-documenting names.** Variable and function names communicate intent. A name like `r` is not acceptable where `row` or `result` would be clear.
5. **Duplicate rather than abstract prematurely.** Two instances of similar code are acceptable. Three or more real use cases justify a shared abstraction. Abstractions are not written speculatively.
6. **No magic.** No procedural macros in the pgmcp crate itself. No `lazy_static!`. No global state. Code is readable without knowing about hidden macro-generated code.
7. **Comments explain why, never what.** A comment that says "increment the counter" above `count += 1` is noise. A comment that says "PostgreSQL returns rows in pg_class order, which is not guaranteed to be stable — sort explicitly to ensure deterministic output" is signal.

### 7.7 Performance Standards

- **Zero-copy where possible.** The row encoding path does not copy bytes that do not need to be copied. Where the OID type mapping allows direct encoding from the wire representation, it is used.
- **Pre-allocate and reuse.** Write buffers are pre-allocated at 64KB. They are reused across rows within a single query execution. They are not dropped and reallocated per row.
- **Profile before optimizing.** No optimization is made without a benchmark demonstrating the problem. Code clarity is not sacrificed for speculative performance.
- **No allocations in the hot loop.** The per-row encoding path (the inner loop of the streaming encoder) contains no heap allocations beyond amortized write buffer growth.

---

## 8. Feature Branch Sequence

### 8.1 Phase 1: Foundation (001-004)

**feat/001 — project-scaffold**
Initialize the Cargo workspace and crate, establish `rust-toolchain.toml`, create the top-level module structure (empty modules for all planned files), add `Cargo.toml` with all planned dependencies at their pinned versions, add `.gitignore`, set up `deny.toml` skeleton. Set up `.github/workflows/ci.yml` with check job (fmt + clippy). Integration test and bench jobs are stubs that pass unconditionally until test infrastructure exists. This ensures every subsequent branch runs under CI. No functional code. Purpose: ensure every subsequent branch has a stable base to branch from and that the dependency graph is decided up front.

**feat/002 — config**
Implement `config.rs`: the `Config` struct with all fields (database URL, pool settings, transport selection, log format, cache invalidation interval, guardrail policy), TOML deserialization via `serde`, environment variable override logic, validation logic, and `pgmcp.example.toml`. Purpose: config must exist before any code that needs it.

**feat/003 — telemetry**
Implement `telemetry.rs`: tracing subscriber initialization, log format selection (JSON vs. human-readable), `RUST_LOG` parsing. Add tracing spans to `main.rs` startup sequence. Purpose: telemetry must be initialized before the startup gate runs so that startup errors are logged.

**feat/004 — error-types**
Define `McpError` in `error.rs` with all error codes from the table in section 3.5. Implement `std::fmt::Display`, `std::error::Error`, and any `From` conversions needed from `tokio_postgres::Error`. Purpose: error types must be defined before any fallible code is written.

---

### 8.2 Phase 2: Connectivity (005-007)

**feat/005 — connection-pool**
Implement `pg/pool.rs`: connection pool construction from config, acquire with timeout, health check, version check at startup. Write integration tests in `tests/integration/health.rs` that verify connectivity against a real Postgres instance. This branch also resolves the deadpool-postgres vs. hand-rolled pool decision (see section 9). Purpose: all subsequent branches that touch Postgres need a working pool.

**feat/006 — mcp-protocol**
Wire up `rmcp` as the MCP protocol layer. Implement `transport/stdio.rs` and `transport/sse.rs`. Implement the dispatcher skeleton in `server/router.rs` with a stub that returns `tool_not_found` for all tool calls. The server must start, accept an MCP handshake, and respond to `tools/list` with an empty list. Purpose: validates that the protocol plumbing works before any tools are implemented.

**feat/007 — dispatcher**
Implement the full dispatcher in `server/router.rs`: `ToolContext` construction in `server/context.rs`, tool name routing table, `tool_defs.rs` with tool definitions (names, descriptions, parameter schemas) for all 15 tools. Tools still return stub responses. `tools/list` must return all 15 tool definitions. Purpose: validates the routing and context injection before tool logic is written.

---

### 8.3 Phase 3: Discovery Tools (008-012)

**feat/008 — health-connection-info**
Implement `tools/health.rs` and `tools/connection_info.rs`. Add `pg/queries/server_settings.sql`. Integration tests verify that health returns `"ok"` against a reachable Postgres and `"unhealthy"` when the pool cannot connect.

**feat/009 — server-info-list-databases**
Implement `tools/server_info.rs` and `tools/list_databases.rs`. Add corresponding `.sql` query files. Integration tests verify correct output against the test database.

**feat/010 — list-schemas-list-tables**
Implement `tools/list_schemas.rs` and `tools/list_tables.rs`. Add corresponding `.sql` query files. Integration tests cover the `kind` parameter filtering and schema filtering.

**feat/011 — describe-table-list-enums**
Implement `tools/describe_table.rs` and `tools/list_enums.rs`. Add corresponding `.sql` query files. These queries are the most complex pg_catalog queries in the codebase; the PostgreSQL agent reviews this branch. Integration tests verify column type mapping, constraint extraction, and foreign key reporting.

**feat/012 — list-extensions-table-stats**
Implement `tools/list_extensions.rs` and `tools/table_stats.rs`. Add corresponding `.sql` query files. Integration tests verify that stats are non-zero for tables with data.

---

### 8.4 Phase 4: Schema Cache (013)

**feat/013 — schema-cache**
Implement `pg/cache.rs` and `pg/invalidation.rs`. The schema cache is populated at startup. The invalidation background task polls at the configured interval. Discovery tools (from branches 008-012) are updated to use the cache for reads rather than hitting pg_catalog on every call. Integration tests in `tests/integration/schema_cache.rs` verify cache hits, cache misses, and cache refresh behavior. Purpose: the cache is a prerequisite for the intelligence tools (branches 019-023) and for acceptable performance.

---

### 8.5 Phase 5: SQL Analysis Layer (014-016)

**feat/014 — sql-parser**
Implement `sql/parser.rs`: the `StatementKind` enum, the `parse_statement` function using `sqlparser` with Postgres dialect, multi-statement detection and rejection, identifier validation. Unit tests cover every `StatementKind` variant. Purpose: must exist before the guardrails or LIMIT injection can be implemented.

**feat/015 — guardrails**
Implement `sql/guardrails.rs`: the guardrail policy struct, guardrail evaluation against a parsed statement, the full set of guardrail rules for the query tool (no DDL, no COPY TO/FROM PROGRAM, no SET session-level parameters that could affect subsequent callers). Unit tests cover every rule. Integration tests in `tests/integration/guardrails.rs` verify that blocked statements are rejected with the correct error code. Purpose: guardrails must exist before the query tool is implemented.

**feat/016 — limit-injection**
Implement `sql/limit.rs`: LIMIT clause injection into SELECT statements that lack a LIMIT. Handles subqueries (does not inject into subquery SELECT, only top-level). Unit tests cover injection, no-injection (LIMIT already present), and subquery handling. Purpose: LIMIT injection must exist before the query tool is implemented.

---

### 8.6 Phase 6: Query Tool (017-018)

**feat/017 — streaming-serialization**
Implement `streaming/mod.rs` (BatchSizer), `streaming/json.rs`, and `streaming/csv.rs`. Implement `pg/types.rs` (OID to Rust to JSON mapping). Add benchmarks in `benches/serialization.rs` and `benches/streaming.rs`. The benchmarks must demonstrate row encoding throughput. Purpose: streaming must be implemented and benchmarked before the query tool uses it, so that regressions can be detected.

**feat/018 — query-tool**
Implement `tools/query.rs`. Wire together SQL analysis (014-016) and streaming (017). Implement all parameters: `intent`, `transaction`, `dry_run`, `limit`, `timeout_seconds`, `format`, `explain`. Integration tests in `tests/integration/query.rs` and `tests/integration/streaming.rs` cover happy path, dry_run, transaction rollback, timeout enforcement, LIMIT injection, and format selection.

---

### 8.7 Phase 7: Intelligence Tools (019-023)

**feat/019 — explain-tool**
Implement `tools/explain.rs`. Integration tests verify that the plan JSON is returned for valid queries and that `analyze: false` produces an estimate-only plan.

**feat/020 — my-permissions**
Implement `tools/my_permissions.rs`. Integration tests verify that the reported privileges match the actual privileges of the test role.

**feat/021 — suggest-index**
Implement `tools/suggest_index.rs`. Heuristic rules cover: missing index on foreign key columns, missing index on columns used in WHERE clauses of the provided query, redundant indexes. Integration tests verify that suggestions are generated for known-bad schemas.

**feat/022 — propose-migration**
Implement `tools/propose_migration.rs`. Integration tests verify that the output contains valid parseable SQL (run each proposed statement through `sql/parser.rs`). The PostgreSQL agent reviews this branch for correctness of the migration heuristics.

**feat/023 — infer-descriptions**
Implement `pg/infer.rs` with the heuristic pattern set (see section 4.5 for categories). Wire `infer.rs` into `describe_table` and `propose_migration` output. Unit tests cover every pattern category (foreign key conventions, timestamp conventions, boolean conventions, status enum conventions). Integration tests verify that inferred descriptions are marked as inferred in tool output.

---

### 8.8 Phase 8: Enrichment (024-025)

**feat/024 — output-formats**
Validate and polish both JSON and CSV output formats. Ensure column type metadata is complete and accurate. Ensure `truncated: true` is set correctly. Verify that `row_count` is accurate for both formats. Benchmark both formats to verify no regression from feat/017 baseline.

**feat/025 — tool-descriptions**
Finalize all tool and parameter descriptions in `server/tool_defs.rs`. Descriptions are written for LLM consumption: unambiguous, specify valid values explicitly, describe edge cases. This branch is prose-heavy and code-light; the quality bar is the clarity of the descriptions.

---

### 8.9 Phase 9: Hardening and Distribution (026-029)

**feat/026 — integration-test-hardening**
Expand integration tests to cover error paths systematically. Every error code in section 3.5 must be triggered by at least one test. Tests cover: unreachable Postgres (connection error), pool exhaustion (timeout error), guardrail violations (all rules), missing schema/table (not_found errors), invalid SQL (parse error). Tests run against PG 14, 15, 16, and 17 in CI.

**feat/027 — ci-pipeline-hardening**
Finalize CI pipeline: enable integration test matrix (PG 14-17), enable benchmark regression checks (5% threshold), add cargo-deny job, add check-targets advisory job. Convert stub jobs to real gates. Implement `.github/workflows/release.yml` as specified in section 10. Implement the full `deny.toml`. All subsequent merges (028, 029) run under the full CI pipeline.

**feat/028 — docker-packaging**
Write the `docker/Dockerfile` (multi-stage, distroless final image). Write `docker-compose.yml` for local development (pgmcp + Postgres). Verify that the Docker image builds cleanly, the binary starts, and health check passes.

**feat/029 — readme-contributing**
Write `README.md` (installation, configuration reference, tool reference, quick-start examples) and `CONTRIBUTING.md` (development setup, branch model, review process from section 6). This is the final branch before the MVP tag.

---

### 8.10 Branch Ordering Rationale

The ordering follows strict dependency order:
- Foundation before connectivity (you need config before you can connect).
- Connectivity before tools (you need a pool before you can query).
- Schema cache before intelligence tools (intelligence tools read from the cache).
- SQL analysis before query tool (guardrails must exist before the query tool executes SQL).
- Streaming before the query tool orchestration (benchmark the hot path before wiring it up).
- CI is bootstrapped in feat/001 (check/fmt/clippy only) and hardened in feat/027 (full test matrix, benchmarks, deny). This means every branch from feat/002 onward runs under at least basic CI, and branches from feat/027 onward run under the complete pipeline.

---

## 9. Dependency Manifest

### 9.1 Runtime Dependencies (15 crates)

| Crate | Version | Purpose |
|---|---|---|
| `tokio` | 1 | Async runtime. Features: `full` in development, trimmed to `rt-multi-thread`, `net`, `io-std`, `time` in production. |
| `rmcp` | 0.x — pin to latest stable in feat/001. Verify upstream maintenance health before committing. | MCP protocol implementation. The protocol layer pgmcp does not reimplement. |
| `axum` | 0.7 | HTTP server for SSE transport. |
| `tokio-postgres` | 0.7 | Postgres wire protocol driver. Features: `with-uuid-1`, `with-time-0_3`. Zero-copy row streaming via `query_raw`. |
| `deadpool-postgres` | 0.13 | Connection pool for tokio-postgres. |
| `serde` | 1 | Serialization framework. Features: `derive`. |
| `serde_json` | 1 | JSON serialization. Used for tool parameter parsing and response construction. |
| `sqlparser` | 0.50 | SQL parser with Postgres dialect. Used by `sql/` module. |
| `time` | 0.3 | Date/time handling. Replaces chrono (see explicit cuts). |
| `tracing` | 0.1 | Structured logging and span instrumentation. |
| `tracing-subscriber` | 0.3 | Tracing subscriber with JSON and human-readable formatters. |
| `thiserror` | 1 | Derive macro for `McpError`. Generates `Display` and `Error` impls. |
| `uuid` | 1 | UUID type for Postgres UUID columns. |
| `ryu` | 1 | Fast float-to-string conversion. Used in hot path row encoder. |
| `bytes` | 1 | `Bytes` and `BytesMut` for zero-copy buffer management in streaming encoder. |
| `toml` | 0.8 | TOML config file deserialization via serde. |

### 9.2 Deferred Decisions

**`deadpool-postgres` vs. hand-rolled pool.** deadpool-postgres is the default for feat/005. After feat/005 is implemented and benchmarked, evaluate whether the pool adds measurable overhead. If pool acquisition overhead is measurable and a ~150-line hand-rolled pool would reduce it, replace deadpool-postgres in a subsequent branch. This is a performance decision, not an aesthetic one; the benchmark decides.

**`axum` feature gating.** Investigate whether axum can be feature-gated behind a `transport-sse` feature flag so that stdio-only deployments do not pull in the HTTP stack. If axum's compile-time cost is acceptable (measured by `cargo build` time), gating is not required. If it adds meaningful compile time or binary size, gate it.

**`ryu` retention.** Keep `ryu` only if the float encoding path in `streaming/json.rs` uses `ryu::Buffer` directly (stack-allocated, no heap). If the path uses `format!()` or `ToString`, `ryu` provides no benefit and should be removed.

### 9.3 Explicit Cuts

**`simd-json`** — Rejected. simd-json accelerates JSON parsing, not JSON serialization. pgmcp's hot path is serialization (encoding Postgres rows as JSON), not parsing. The tool parameter payloads are small and infrequent; parsing them with serde_json is not measurable overhead. simd-json would add a build dependency on SIMD intrinsics and complicate cross-compilation with no benefit.

**`clap`** — Rejected. pgmcp has three command-line arguments: `--transport`, `--config`, and `--help`. This is 30 lines of hand-written argument parsing. clap adds compile time and binary size for no meaningful ergonomic benefit at this argument count.

**`chrono`** — Rejected. chrono has a known soundness issue (time-of-check/time-of-use in local timezone handling). The `time 0.3` crate provides equivalent functionality without the soundness concern. tokio-postgres's `with-time-0_3` feature integrates directly.

**`anyhow` / `eyre`** — Rejected. The error type is part of pgmcp's product surface. `McpError` carries machine-readable error codes that agents consume. Erasing the error type into a boxed trait object (anyhow's model) would lose the codes. The ergonomic cost of defining `McpError` explicitly is low; the benefit is a structured, auditable error surface.

**`once_cell` / `lazy_static`** — Rejected. pgmcp has no global state. There is no use case for these crates. All shared state is injected via `Arc` through `ToolContext`.

**`diesel` / `sqlx`** — Rejected. Both ORMs buffer all result rows into a Vec before returning them. pgmcp's streaming architecture requires zero-copy streaming via tokio-postgres's `query_raw`. Neither ORM exposes this interface. Additionally, sqlx's compile-time query checking requires a live database at compile time, which is incompatible with the CI cross-compilation targets.

### 9.4 Development Dependencies

| Crate | Purpose |
|---|---|
| `testcontainers` | Spin up real Postgres instances in integration tests without external setup. |
| `criterion` | Benchmarking framework. Used in `benches/`. |

### 9.5 CI Dependencies

| Tool | Purpose |
|---|---|
| `cargo-deny` | License, vulnerability, and duplicate dependency checking. |
| `cross` | Cross-compilation for release targets. |

---

## 10. CI Pipeline

### 10.1 `ci.yml`

Triggered on: push to any branch, pull request to main.

Workflow-level permissions: `{}` (empty — no default permissions granted).

**Toolchain:** Defined in `rust-toolchain.toml` (pinned stable toolchain). All jobs use the pinned toolchain. No `dtolnay/rust-toolchain` action with floating channels.

**Caching strategy:**
- Registry cache: keyed on `Cargo.lock` hash.
- Target cache: keyed on `Cargo.lock` hash + job matrix key.
- Tools cache (cargo-deny, criterion baselines): keyed on tool version.
- Bench baselines: persisted across runs using GitHub Actions cache with restore-keys fallback.

---

#### Job: `check`

Runs on: `ubuntu-24.04`

Steps:
1. Checkout
2. Restore registry + target cache
3. `cargo fmt --check`
4. `cargo clippy --all-targets --all-features -- -D warnings`
5. Save cache

This job runs fast (no Postgres required). All CI jobs run in parallel — no inter-job dependencies.

---

#### Job: `check-targets` (advisory)

Runs on: `ubuntu-24.04`

Steps:
1. Checkout
2. Install `cross`
3. Attempt `cross build --target x86_64-unknown-linux-musl`
4. Attempt `cross build --target aarch64-unknown-linux-musl`

Status: advisory (does not block merge). Promoted to required after the first release when release targets are established. macOS builds are validated in release.yml only.

---

#### Job: `test`

Strategy matrix: `pg_version: [14, 15, 16, 17]`, `fail-fast: false`.

Runs on: `ubuntu-24.04`

Services:
```yaml
postgres:
  image: postgres:${{ matrix.pg_version }}
  env:
    POSTGRES_USER: pgmcp_test
    POSTGRES_PASSWORD: pgmcp_test
    POSTGRES_DB: pgmcp_test
  ports:
    - 5432:5432
  options: >-
    --health-cmd pg_isready
    --health-interval 10s
    --health-timeout 5s
    --health-retries 5
```

Steps:
1. Checkout
2. Restore registry + target cache
3. `cargo test --all-features` with `DATABASE_URL` set to the service container
4. Save cache

All four matrix legs must pass for the job to be considered green. `fail-fast: false` means a failure on PG 14 does not cancel the PG 17 run; all failures are surfaced simultaneously.

---

#### Job: `deny`

Runs on: `ubuntu-24.04`

Steps:
1. Checkout
2. Install `cargo-deny`
3. `cargo deny check`

`deny.toml` configuration:
- `[advisories]`: deny all known-vulnerable crates.
- `[licenses]`: allow `Apache-2.0`, `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-DFS-2016`. Deny all others.
- `[bans]`: deny duplicate versions of `tokio`, `serde`, `serde_json`. Deny `chrono` (replaced by `time`). Deny `anyhow`, `eyre`.
- `[sources]`: allow only crates.io.

---

#### Job: `bench`

Runs on: `ubuntu-24.04`

Triggers: push to main, pull request to main (required for hot-path PRs; see gate rules).

Services: Postgres 17 (latest; benches run against one version for consistency).

Steps:
1. Checkout
2. Restore registry + target + bench baseline cache
3. `cargo bench --bench serialization -- --save-baseline current`
4. `cargo bench --bench streaming -- --save-baseline current`
5. `cargo bench --bench connection -- --save-baseline current`
6. Compare against stored baseline; fail if any benchmark regresses more than 5%
7. Save updated baselines to cache

**Regression threshold:** 5%. A benchmark that regresses by more than 5% blocks merge for branches touching the hot path. Branches not touching the hot path are exempt (bench job passes unconditionally).

---

#### Gate Rules (PR to main requires)

- `check` — required, must pass
- `test` (all 4 PG versions) — required, all matrix legs must pass
- `deny` — required, must pass
- `bench` — required for PRs touching hot path files (`streaming/`, `sql/`, `pg/types.rs`); advisory otherwise
- `check-targets` — advisory (not required for merge)

---

### 10.2 `release.yml`

Triggered on: push of a tag matching `v[0-9]+.[0-9]+.[0-9]+`.

---

#### Job: `validate-tag`

Steps:
1. Checkout
2. Extract version from tag (strip leading `v`)
3. Extract version from `Cargo.toml` using `grep`
4. Fail if they do not match exactly

This prevents a tag from being pushed without bumping `Cargo.toml`, which would produce a release binary with a mismatched version string.

---

#### Job: `build`

Strategy matrix:
- `x86_64-unknown-linux-musl` (ubuntu-24.04, via cross)
- `aarch64-unknown-linux-musl` (ubuntu-24.04, via cross)
- `x86_64-apple-darwin` (macos-13, native Intel runner)
- `aarch64-apple-darwin` (macos-14, native Apple Silicon runner)
- `x86_64-pc-windows-msvc` (windows-2022, native)

Steps:
1. Checkout
2. Install `cross` for Linux targets; use native toolchain for macOS
3. Build with `cross build --release --target ${{ matrix.target }}` (Linux) or `cargo build --release --target ${{ matrix.target }}` (macOS)
4. Run binary smoke test: execute `./pgmcp --help` and verify exit code 0
5. Upload binary as release artifact

**Smoke test** before artifact upload: the built binary must start, print help, and exit cleanly. A binary that fails this check fails the build job and blocks the release.

---

#### Job: `docker`

Runs after: `build` (requires linux binaries)

Steps:
1. Checkout
2. Set up QEMU and Docker Buildx for multi-arch builds
3. Build multi-arch Docker image (`linux/amd64`, `linux/arm64`) using pre-built binaries from `build` job artifacts
4. Sign image with `cosign` using keyless signing (OIDC-based)
5. Run `trivy` vulnerability scan on the built image; fail on CRITICAL or HIGH findings
6. Push to GitHub Container Registry (ghcr.io) with tag matching the git tag and `latest`

---

#### Job: `publish`

Needs: `validate-tag`. Runs in parallel with `build` (both depend on `validate-tag`).

Steps:
1. Checkout
2. `cargo publish --dry-run` (validates publishability without pushing)
3. `cargo publish` with crates.io token from secrets

---

#### Job: `release`

Needs: `validate-tag`, `build`, `docker`, `publish` (all must succeed).

Steps:
1. Download all binary artifacts from `build` job
2. Generate SLSA provenance attestation
3. Create GitHub release with:
   - Binaries for all 5 targets (with SHA256 checksums)
   - Docker pull instructions
   - Auto-generated changelog from commit messages since last tag

---

#### Security Hardening

- All third-party actions are pinned to full SHA (not floating tags). Example: `actions/checkout@<sha>` not `actions/checkout@v4`.
- Permissions are set per-job to the minimum required. `contents: read` is the default; `packages: write` is granted only to the `docker` job; `id-token: write` is granted only to the `publish` and `release` jobs (for OIDC).
- SLSA provenance is generated at level 1 for the initial release; level 2 is the target for the first post-MVP release.
- cosign signing uses keyless OIDC signing tied to the GitHub Actions OIDC token; no long-lived signing keys are stored in secrets.

---

## 11. Agents and Tooling

The following specialized agents are used during implementation. Each agent is invoked for tasks within its domain; no single agent handles all work.

**Rust engineer agent** (`voltagent-lang:rust-engineer`)
Primary implementer. Responsible for all Rust code, architecture decisions, trait design, performance optimization, and benchmark interpretation. Reviews unsafe code. Executes PLAN, IMPLEMENT, SELF-TEST, and RESPOND steps of the lifecycle.

**PostgreSQL agent** (`voltagent-data-ai:postgres-pro`)
Consulted for pg_catalog query optimization, Postgres version-aware behavior, index heuristics, and migration proposal correctness. Reviews branches 011 (describe_table pg_catalog queries), 022 (propose_migration heuristics), and any branch that introduces new pg_catalog queries. The PostgreSQL agent does not write Rust code; it reviews SQL and provides PostgreSQL-domain expertise.

**DevOps agent** (`voltagent-infra:devops-engineer`)
Responsible for the CI pipeline design (ci.yml, release.yml), Docker image construction, cross-compilation target configuration, and SLSA/cosign signing setup. Reviews branch 027 (CI pipeline) and branch 028 (Docker packaging). Does not review Rust application code.

**Code reviewer agent** (`superpowers:code-reviewer` or `pr-review-toolkit:code-reviewer`)
Executes REVIEW and RE-REVIEW steps. Applies the 12-point checklist without exception. May request changes on any checklist point. Signals `APPROVED` explicitly when all checklist points are satisfied. The code reviewer agent is a blocking step; no branch merges without its approval.

**Security agent** (`voltagent-qa-sec:security-engineer`)
Consulted for: release signing design (cosign, SLSA), dependency audit configuration (deny.toml), guardrail rule validation (ensuring that the guardrail ruleset cannot be bypassed), and review of any branch that adds unsafe code. Reviews branch 015 (guardrails) and branch 027 (CI security hardening).

---

## 12. What We Exclude (and Why)

These exclusions are permanent architectural decisions, not deferred roadmap items (except where noted for M3 and cloud-only features in section 4.4).

**No other databases.**
pgmcp exploits Postgres-specific features throughout: OID-based type mapping, pg_catalog introspection, extended query protocol, advisory locks, row-level security awareness. Supporting a second database would require abstracting away every Postgres-specific optimization. The abstraction would make the Postgres path worse and the other-database path shallow. Postgres-only means every line of code makes the Postgres experience better.

**No migration framework.**
pgmcp proposes migrations (via `propose_migration`). It does not manage migration history, track applied versions, or execute migrations on the user's behalf. Migration management (Flyway, Liquibase, golang-migrate, sqitch) is an existing solved problem with mature tooling. pgmcp's role is to help agents understand the schema and propose changes; the decision to apply those changes belongs to the human or the agent's orchestration layer.

**No ORM or query builder.**
pgmcp executes SQL that the agent provides. It does not generate SQL from an object model. An ORM would intermediate between the agent's intent and Postgres's execution, adding a layer of indirection that obscures query behavior and prevents direct exploitation of Postgres features. Agents can write SQL; pgmcp executes it.

**No dashboard, GUI, or web UI.**
pgmcp is a headless server. Its interface is the MCP protocol. Adding a web UI would add dependencies, attack surface, and maintenance burden for a use case (human direct interaction) that is explicitly not the target. Humans interacting with Postgres have pgAdmin, DBeaver, psql, and dozens of other tools.

**No built-in auth for OSS.**
pgmcp inherits the authentication model of Postgres: the connection string carries credentials, and Postgres enforces permissions. Adding a separate auth layer to pgmcp (API keys, OAuth, JWT) would duplicate what Postgres already does and create a second authentication surface to audit and maintain. OSS deployments are expected to be run in trusted environments (behind a firewall, in a private network, with network-level access control). The AgentDB cloud product handles multi-tenant authentication as a cloud concern.

**No plugins, extensions, or middleware API.**
pgmcp is not a platform. It does not have a plugin API, a hook system, or a middleware interface. Adding extension points would commit pgmcp to supporting a public API surface indefinitely, constrain future refactoring, and open the door to untested third-party code running in the pgmcp process. Features are added to pgmcp's core or they are not part of pgmcp. This is what makes the codebase auditable and the behavior predictable.
