# pgmcp: Engineering Scope

**One database. One protocol. Zero compromise.**

pgmcp is a Rust MCP server for PostgreSQL. It is the fastest, most capable way for an AI agent to interact with a Postgres database. Nothing else is in scope.

---

## Design Principles

**1. Postgres is the product.** Not "databases." Not "data infrastructure." PostgreSQL. We exploit every Postgres-specific feature — `pg_catalog`, `information_schema`, `pg_stat_statements`, `EXPLAIN (FORMAT JSON)`, advisory locks, `LISTEN/NOTIFY`, `COPY`, row-level security, generated columns, exclusion constraints. A generic "database MCP server" would abstract these away. We surface them as superpowers.

**2. Zero overhead.** The server adds less than 1ms to any operation. If you benchmark pgmcp vs. a direct `psql` connection, the difference should be within noise. This means: zero-copy result streaming, zero-allocation hot paths, no garbage collector, no runtime. Rust makes this a structural guarantee, not a best-effort aspiration.

**3. Agents are the user, not developers.** Every tool response is designed to be consumed by an LLM. This means: rich descriptions alongside data, plain-language error messages, self-documenting schemas, proactive warnings, and context that helps an agent decide what to do next. A developer can always drop to `psql`. An agent only has what we give it.

**4. Intentional exclusion is a feature.** What we don't do is as important as what we do. No Redis. No MySQL. No multi-database abstraction. No ORM. No migration framework. No dashboard. Every feature we refuse to build makes the features we ship better.

---

## What We Exclude (and Why)

These are permanent, philosophical exclusions — not "maybe later."

**No other databases.** Not Redis, not ClickHouse, not MySQL, not SQLite, not DuckDB. The adapter-pattern abstraction that makes multi-database "easy" is exactly the abstraction that prevents us from being best-in-class at Postgres. Every tool, every error message, every response format is Postgres-native. We use `pg_catalog` views, not `information_schema` (which is a SQL standard compatibility layer that loses Postgres-specific metadata). We surface Postgres-specific index types (GIN, GiST, BRIN, SP-GiST), Postgres-specific column types (tsvector, jsonb, citext, ltree, arrays, ranges, composite types), and Postgres-specific features (advisory locks, row-level security policies, generated columns, table inheritance, partitioning). None of this survives a generic database abstraction.

**No migration framework.** We generate SQL for schema changes. We don't track migration history, manage up/down scripts, or compete with Prisma/Drizzle/Alembic. Those tools have years of edge-case handling that we'd replicate poorly. Our `propose_migration` tool produces a SQL statement and a human-readable diff. What the developer does with that SQL is their business.

**No ORM or query builder.** We execute SQL. We don't generate SQL from a DSL, manage entity relationships, or provide a type-safe query API. The agent writes SQL (or describes intent in natural language that we translate to SQL). SQL is the interface.

**No dashboard, no GUI, no web UI.** The MCP protocol is the only interface. There is no admin panel, no query editor, no data browser. These are human tools. We build for agents.

**No built-in auth for the OSS server.** The OSS server trusts whoever connects to it. Auth is the cloud product's job. In OSS, you secure the server the same way you secure any local service: don't expose it to the internet. Adding auth to the OSS server adds complexity, configuration burden, and bug surface for a use case (local development) that doesn't need it.

**No plugins, no extensions, no middleware API.** The server is a single binary that does one thing. There is no plugin interface, no shared library loading, no webhook system, no event hooks. Extensibility is achieved by forking the repo and modifying the source. This is a deliberate tradeoff: plugins create API surface that constrains future changes. We ship fast by keeping the boundary at the process level.

---

## Performance Architecture

### Memory Model

```
┌──────────────────────────────────────────────────┐
│                   pgmcp process                   │
│                                                    │
│  Stack-allocated:                                  │
│  ┌────────────────────────────────────────────┐   │
│  │ MCP frame parsing (zero-copy from buffer)  │   │
│  │ Tool name dispatch (compile-time match)    │   │
│  │ Parameter validation (schema-driven)       │   │
│  └────────────────────────────────────────────┘   │
│                                                    │
│  Heap-allocated (pooled, reused):                  │
│  ┌────────────────────────────────────────────┐   │
│  │ Read buffers (per-connection, 8KB default)  │   │
│  │ Write buffers (per-connection, 8KB default) │   │
│  │ Row serialization buffers (pooled, 64KB)    │   │
│  └────────────────────────────────────────────┘   │
│                                                    │
│  Heap-allocated (per-query, freed on completion):  │
│  ┌────────────────────────────────────────────┐   │
│  │ Query parameter bindings                    │   │
│  │ Column metadata (cached after first query)  │   │
│  └────────────────────────────────────────────┘   │
│                                                    │
│  NEVER allocated:                                  │
│  ┌────────────────────────────────────────────┐   │
│  │ Full result sets (streamed, never buffered) │   │
│  │ Row data beyond current batch (freed per    │   │
│  │   batch before next batch is read)          │   │
│  └────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────┘
```

### Result Streaming (The Core Performance Feature)

This is the most important architectural decision in the entire server. Every query result is streamed row-by-row from Postgres to the agent. At no point does the full result set exist in server memory.

```
PostgreSQL wire protocol          pgmcp                    MCP SSE stream
─────────────────────────    ──────────────────    ──────────────────────────
DataRow message (binary) ──> Decode row fields ──> Serialize to JSON ──>
                             using zero-copy       Write to SSE buffer ──>
                             from network buffer   Flush to agent
                             
DataRow message ──────────> Decode ──────────────> Serialize ──> Flush
DataRow message ──────────> Decode ──────────────> Serialize ──> Flush
...
CommandComplete ──────────> Send summary event (row count, timing)
```

Implementation uses `tokio-postgres`'s `RowStream` which yields rows as they arrive on the wire. Each row is decoded, serialized to JSON, and flushed to the SSE connection before the next row is processed. The serialization buffer is pre-allocated and reused across rows.

**Memory usage for a 1M-row query: ~128KB** (buffer pool). Same as a 10-row query.

**Latency to first row: database latency + <0.1ms.** The agent sees the first row as soon as Postgres produces it.

### Batch Streaming

Raw row-by-row SSE events create too much framing overhead for large results. Instead, batch rows:

```
Batch size heuristic:
  - Result set <100 rows: send all in one event (low overhead)
  - Result set 100-10,000 rows: batch of 100 rows per event
  - Result set >10,000 rows: batch of 500 rows per event
  
  First batch is always sent immediately regardless of size
  (agent gets data ASAP, we calibrate batch size from row 2 onward)
```

The batch size auto-tunes based on row size. If rows are wide (many columns, large text fields), reduce batch size to keep each SSE event under 64KB. If rows are narrow (a few integers), increase batch size to reduce framing overhead.

### Connection Pooling

pgmcp implements its own connection pool using `deadpool-postgres`. Not PgBouncer — we run in-process for lower latency and simpler deployment.

```
Pool configuration:
  min_connections: 2       (warm connections ready to go)
  max_connections: 20      (configurable, default matches PG default of 100 / 5)
  max_idle_time: 5min      (reclaim connections not used recently)
  max_lifetime: 30min      (rotate connections to pick up PG config changes)
  connection_timeout: 5s   (fail fast if pool is exhausted)
  statement_cache: 256     (prepared statement cache per connection)
```

Every tool call acquires a connection, executes, and releases. Connections are never held across tool calls. The pool uses a LIFO stack (most-recently-used connection first) so hot connections stay warm and idle connections get reclaimed.

**Prepared statement caching:** Every SQL query the server generates is prepared on first execution and cached by the connection. Subsequent executions skip the parse/plan phase. For `describe_table` and `list_tables` — which agents call frequently — this means the second call is 2-5x faster than the first.

### Benchmark Targets

| Operation | Target | Measurement |
|-----------|--------|-------------|
| `list_tables` (50 tables) | <3ms total | From MCP request to complete SSE response |
| `describe_table` (20 columns) | <2ms total | Including all metadata |
| `query_read` (10 rows, simple) | DB time + <0.5ms | Server overhead only |
| `query_read` (10K rows, streaming) | DB time + <2ms to first batch | Time to first SSE event with data |
| `query_read` (100K rows, streaming) | DB time + constant memory | Peak memory <256KB regardless of result size |
| `explain_query` | DB time + <1ms | EXPLAIN + plain-language parse |
| Concurrent connections (idle) | 50K @ <100MB RSS | Tokio tasks, not threads |
| Concurrent connections (active queries) | 500 @ <200MB RSS | Bounded by PG connection pool |
| MCP handshake (tool discovery) | <5ms | Pre-serialized tool list |
| Cold start | <200ms | Binary startup to accepting connections |

These are measured in CI on every merge. Regression >5% blocks the merge.

---

## Postgres Tool Set (Complete)

### Meta Tools

**`list_databases`**

Returns the list of databases on the PostgreSQL server that the connected user has access to. Includes size, owner, encoding, and connection count. This lets an agent orient itself: "what databases exist on this server?"

```
Returns:
  databases: [{
    name: string,
    owner: string,
    encoding: string,
    size_bytes: i64,
    active_connections: i32,
    description: string (from pg_shdescription)
  }]
```

SQL: Single query against `pg_database` joined with `pg_stat_database` and `pg_shdescription`.

**`server_info`**

Returns PostgreSQL version, server settings relevant to agents (max_connections, work_mem, statement_timeout), installed extensions, and pgmcp version.

```
Returns:
  pg_version: string ("16.2"),
  pg_version_num: i32 (160002),
  extensions: [string] (["pg_stat_statements", "uuid-ossp", "pgcrypto"]),
  settings: {max_connections, work_mem, statement_timeout, ...},
  pgmcp_version: string,
  uptime_seconds: i64
```

### Discovery Tools

**`list_schemas`**

```
Returns:
  schemas: [{
    name: string,
    owner: string,
    table_count: i32,
    description: string
  }]
```

**`list_tables`**

```
Parameters:
  schema: string (default "public")

Returns:
  tables: [{
    name: string,
    schema: string,
    type: "table" | "view" | "materialized_view" | "partitioned_table" | "foreign_table",
    row_estimate: i64 (from pg_stat_user_tables — fast, no COUNT(*)),
    size_bytes: i64 (pg_total_relation_size — includes indexes and TOAST),
    size_pretty: string ("145 MB"),
    column_count: i32,
    has_primary_key: bool,
    index_count: i32,
    description: string (from pg_description COMMENT),
    last_vacuum: timestamp (nullable),
    last_analyze: timestamp (nullable),
    live_tuples: i64,
    dead_tuples: i64
  }]
```

Single query. Joins `pg_class`, `pg_namespace`, `pg_stat_user_tables`, `pg_description`, `pg_index`. No per-table queries.

**`describe_table`**

The most important tool in the server. This is how an agent understands a table.

```
Parameters:
  table: string (required)
  schema: string (default "public")

Returns:
  table: {
    name, schema, type, description,
    row_estimate: i64,
    size: {total_bytes, table_bytes, index_bytes, toast_bytes, total_pretty},
    
    columns: [{
      name: string,
      type: string ("integer", "text", "uuid", "timestamp with time zone", "jsonb", "text[]"),
      type_detail: string (for complex types: "numeric(10,2)", "varchar(255)"),
      nullable: bool,
      default: string (nullable — "nextval('id_seq')", "now()", "'{}'::jsonb"),
      is_primary_key: bool,
      is_unique: bool,
      is_generated: "never" | "always" | "by_default" (identity columns),
      generation_expression: string (nullable, for generated columns),
      description: string (from COMMENT),
      
      # Foreign key info (if this column references another table)
      references: {
        table: string,
        column: string,
        on_delete: string ("CASCADE", "SET NULL", "RESTRICT"),
        on_update: string
      } (nullable),
      
      # For enum types, list the values
      enum_values: [string] (nullable),
      
      # Inferred from column name + type when no COMMENT exists
      inferred_description: string
    }],
    
    indexes: [{
      name: string,
      columns: [string],
      unique: bool,
      primary: bool,
      type: "btree" | "hash" | "gin" | "gist" | "spgist" | "brin",
      size_bytes: i64,
      condition: string (nullable — for partial indexes),
      definition: string (full CREATE INDEX statement)
    }],
    
    constraints: [{
      name: string,
      type: "primary_key" | "foreign_key" | "unique" | "check" | "exclusion",
      columns: [string],
      definition: string,
      referenced_table: string (nullable, for FK)
    }],
    
    # Tables that reference THIS table (reverse FK lookup)
    referenced_by: [{
      table: string,
      column: string,
      constraint_name: string,
      on_delete: string
    }],
    
    # Partitioning info (if partitioned)
    partitioning: {
      strategy: "range" | "list" | "hash",
      key: string,
      partitions: [{name, expression, row_estimate}]
    } (nullable),
    
    # RLS policies (if any)
    rls_policies: [{
      name: string,
      command: "ALL" | "SELECT" | "INSERT" | "UPDATE" | "DELETE",
      roles: [string],
      using_expression: string,
      check_expression: string (nullable)
    }],
    
    # Triggers
    triggers: [{
      name: string,
      timing: "BEFORE" | "AFTER" | "INSTEAD OF",
      events: ["INSERT", "UPDATE", "DELETE"],
      function: string,
      enabled: bool
    }]
  }
```

This response is dense by design. An agent calling `describe_table` gets everything it needs to understand the table in a single round trip. No follow-up calls to get indexes, constraints, or foreign keys separately.

Implementation: 3 queries batched in a single round trip via `tokio-postgres` pipeline:
1. Columns: `pg_attribute` + `pg_attrdef` + `pg_description` + `pg_constraint` (FK)
2. Indexes: `pg_index` + `pg_class` + `pg_am`
3. Constraints + RLS + Triggers: `pg_constraint` + `pg_policy` + `pg_trigger`

**`inferred_description` field:** When a column has no `COMMENT`, pgmcp generates a best-effort description from the column name and type:
- `created_at timestamp` → "Timestamp when the record was created"
- `user_id uuid` → "Reference to a user record (likely foreign key)"
- `email text` → "Email address"
- `is_active boolean` → "Whether this record is currently active"
- `amount_cents integer` → "Monetary amount in cents"
- `metadata jsonb` → "Flexible JSON metadata"
- `tags text[]` → "Array of tag values"

This is a hardcoded heuristic table (~200 common patterns), not LLM-based. It's wrong sometimes, but it's better than nothing, and it's instant. The cloud version replaces this with LLM-inferred descriptions.

**`list_enums`**

```
Returns:
  enums: [{
    name: string,
    schema: string,
    values: [string],
    used_by: [{table, column}]
  }]
```

Agents need to know what enum types exist and what values are valid. This prevents agents from trying to insert invalid enum values.

**`list_extensions`**

```
Returns:
  installed: [{name, version, schema, description}],
  available: [{name, version, description, comment}]
```

**`table_stats`**

```
Parameters:
  table: string

Returns:
  row_estimate: i64,
  disk_size: {total, table, indexes, toast},
  cache_hit_ratio: f64 (from pg_stat_user_tables),
  sequential_scans: i64,
  index_scans: i64,
  live_tuples: i64,
  dead_tuples: i64,
  last_vacuum: timestamp,
  last_autovacuum: timestamp,
  last_analyze: timestamp,
  last_autoanalyze: timestamp,
  modifications_since_analyze: i64
```

This helps agents (and the cloud product's guardrails) understand table health. A table with 50% dead tuples needs a VACUUM. A table with 0 index scans and 10K sequential scans needs an index.

### Read Tools

**`query`**

The primary query execution tool. Handles SELECT, INSERT...RETURNING, UPDATE...RETURNING, DELETE...RETURNING, and CTEs.

```
Parameters:
  sql: string (required)
  params: [any] (positional $1, $2, ... — default [])
  limit: i32 (default 100, max 10000 — only applied to SELECT without existing LIMIT)
  timeout_seconds: i32 (default 30, max 300)
  explain: bool (default false — if true, prepend EXPLAIN ANALYZE)
  format: "json" | "json_compact" | "csv" (default "json")

Returns: (streamed)
  Event 1 — metadata:
    columns: [{name, type, description}]
    sql_executed: string (the actual SQL, with LIMIT injection if applied)
    estimated_rows: i64 (from planner, before execution)
    
  Events 2..N — data batches:
    rows: [[value, value, ...]]  (array-of-arrays in json_compact, array-of-objects in json)
    batch_number: i32
    
  Final event — summary:
    total_rows: i64
    execution_time_ms: f64
    rows_scanned: i64 (if pg_stat_statements available)
    shared_blocks_hit: i64 (cache hits)
    shared_blocks_read: i64 (disk reads)
    plan_summary: string (if query took >500ms, auto-explain)
```

Design decisions:

**Single `query` tool, not `query_read` and `query_write`.** Splitting reads and writes creates an awkward boundary for `INSERT...RETURNING`, CTEs with side effects, and `SELECT...FOR UPDATE`. A single tool handles everything. The agent declares intent through the SQL itself. The server detects whether the query modifies data by parsing the first keyword (SELECT vs INSERT/UPDATE/DELETE/CREATE/DROP) and applies appropriate guardrails.

**`json_compact` format.** Default `json` returns `[{"id": 1, "name": "Alice"}, ...]` — readable but verbose. `json_compact` returns column names once in metadata, then rows as arrays: `[[1, "Alice"], ...]`. For large result sets, this is 40-60% smaller on the wire. Agents that need performance should use `json_compact`.

**Auto-explain for slow queries.** If a query takes >500ms, the summary event includes a plain-language plan analysis. The agent didn't ask for it, but it needs it — a slow query is a problem the agent should know about and potentially fix (by calling `suggest_index`).

**`csv` format.** For agents that need to process large data exports or pass results to other tools. CSV streaming is trivially efficient — rows are serialized as text lines with no structural overhead.

**LIMIT injection logic:**
1. Parse the SQL with a lightweight SQL parser (we use `sqlparser-rs`).
2. If the statement is a SELECT and has no LIMIT clause, append `LIMIT {limit}`.
3. If the statement is a SELECT with a LIMIT clause, use the smaller of the existing LIMIT and the configured max.
4. Non-SELECT statements are never modified.
5. The `sql_executed` field in the response shows exactly what was run, so the agent knows if LIMIT was injected.

**Parameter handling:** Parameters use Postgres positional syntax (`$1`, `$2`). The server validates that the number of params matches the number of placeholders before sending to Postgres. This catches a common agent mistake ("I provided 3 parameters but my SQL has 2 placeholders") with a clear error message instead of a cryptic Postgres error.

**`explain`**

```
Parameters:
  sql: string
  params: [any]
  analyze: bool (default false)
  verbose: bool (default false)

Returns:
  plan_json: object (raw EXPLAIN JSON output)
  plan_text: string (EXPLAIN text output)
  summary: {
    total_cost: f64,
    estimated_rows: i64,
    actual_time_ms: f64 (if analyze=true),
    actual_rows: i64 (if analyze=true),
    node_summary: string (plain-language description of the plan),
    warnings: [string],
    suggestions: [string]
  }
```

**Plain-language plan parsing** is a deterministic rule engine, not LLM-based:

```rust
// Simplified example of plan analysis rules
match node.node_type {
    "Seq Scan" if node.plan_rows > 10_000 => {
        warnings.push(format!(
            "Sequential scan on {} ({} estimated rows). This reads the entire table.",
            node.relation_name, node.plan_rows
        ));
        if let Some(filter) = &node.filter {
            suggestions.push(format!(
                "Consider creating an index: CREATE INDEX ON {} ({})",
                node.relation_name, extract_filter_columns(filter)
            ));
        }
    },
    "Nested Loop" if node.plan_rows > 1_000 => {
        warnings.push("Nested loop join with large outer set — may be slow".into());
    },
    "Sort" if node.sort_method == Some("external merge") => {
        warnings.push("Sort spilled to disk — work_mem may be too low".into());
    },
    _ => {}
}
```

~50 rules covering the most common plan node types. Not exhaustive, but covers 90% of what agents encounter.

**`suggest_index`**

```
Parameters:
  sql: string
  params: [any]

Returns:
  current_plan: string,
  suggestions: [{
    create_sql: string ("CREATE INDEX CONCURRENTLY idx_orders_customer_id ON orders (customer_id)"),
    impact: string ("Eliminates sequential scan on orders. Estimated 50x improvement for this query."),
    tradeoffs: string ("Adds ~12MB index. Slightly slower INSERT/UPDATE on orders."),
    index_size_estimate: string ("~12 MB based on 500K rows")
  }]
```

Implementation:
1. Run `EXPLAIN (FORMAT JSON)` on the query.
2. Walk the plan tree and collect all Seq Scan nodes with filter conditions.
3. For each, extract the filtered columns and generate a CREATE INDEX statement.
4. If `hypopg` extension is installed, create hypothetical indexes and re-run EXPLAIN to get actual cost comparison. If not installed, use heuristic estimates.
5. `CONCURRENTLY` is always included in the suggestion (non-blocking index creation).

### Schema Modification Tools

**`propose_migration`**

```
Parameters:
  intent: string ("add a notes column to the orders table, text, nullable")

Returns:
  sql: string ("ALTER TABLE orders ADD COLUMN notes text"),
  reverse_sql: string ("ALTER TABLE orders DROP COLUMN notes"),
  diff: string (human-readable before/after schema comparison),
  warnings: [{
    severity: "info" | "warning" | "danger",
    message: string
  }],
  estimated_impact: {
    locks: string ("ACCESS EXCLUSIVE lock on orders for <1 second"),
    downtime: bool,
    data_loss_risk: bool
  }
```

**Intent parsing** is template-based (not LLM). Supported patterns:

```
"add column {name} to {table}, {type}"
"add column {name} to {table}, {type}, not null, default {value}"
"drop column {name} from {table}"
"rename column {old} to {new} on {table}"
"rename table {old} to {new}"
"create index on {table}({columns})"
"create unique index on {table}({columns})"
"drop index {name}"
"add foreign key on {table}({col}) references {other_table}({other_col})"
"add check constraint on {table}: {expression}"
"create table {name} ({col1} {type1}, {col2} {type2}, ...)"
"drop table {name}"
"add enum value {value} to {type}"
```

If the intent doesn't match any pattern, return an error: "I couldn't parse that intent. Try a more specific phrasing like 'add column X to table Y, type Z', or provide the SQL directly using the query tool."

**Warning generation** is Postgres-version-aware:

```rust
// On PG <11, adding a column with a non-null default rewrites the table
if pg_version < 110000 && has_default && !nullable {
    warnings.push(Warning {
        severity: Danger,
        message: format!(
            "On PostgreSQL {}, adding a NOT NULL column with a default rewrites the entire table. \
             This locks {} for the duration. Consider adding the column as nullable first, \
             then backfilling, then adding the NOT NULL constraint.",
            pg_version_string, table_name
        ),
    });
}

// Adding an index without CONCURRENTLY blocks writes
if intent.is_create_index && !intent.sql.contains("CONCURRENTLY") {
    warnings.push(Warning {
        severity: Warning,
        message: "CREATE INDEX locks the table for writes during index build. \
                  Use CREATE INDEX CONCURRENTLY to avoid blocking.".into(),
    });
    // Auto-fix: inject CONCURRENTLY
    intent.sql = intent.sql.replace("CREATE INDEX", "CREATE INDEX CONCURRENTLY");
}
```

**`execute_sql`**

For when the agent has SQL and just needs to run it. No intent parsing, no wrapping.

```
Parameters:
  sql: string
  params: [any]
  transaction: bool (default true — wrap in explicit transaction)
  dry_run: bool (default false — execute in transaction then ROLLBACK)

Returns:
  success: bool,
  rows_affected: i64,
  execution_time_ms: f64,
  notices: [string] (Postgres NOTICE messages)
```

Guardrails:
- `DROP DATABASE`, `DROP SCHEMA ... CASCADE`, `TRUNCATE` without `--i-mean-it` parameter: blocked with a clear error. These are nearly always mistakes when an agent runs them.
- `DELETE FROM {table}` or `UPDATE {table} SET` without WHERE: blocked. "This would affect all rows. Add a WHERE clause."
- Any DDL on system tables (`pg_*`, `information_schema`): blocked unconditionally.

### Introspection Tools

**`my_permissions`**

In OSS, returns: "Full access. You are connected as {user} to {database} on {host}. You can read and write all tables, modify schema, and manage extensions."

In cloud (via the gateway), returns the scoped permission set.

**`connection_info`**

```
Returns:
  user: string,
  database: string,
  host: string,
  port: i32,
  ssl: bool,
  server_version: string,
  connection_pool: {
    active: i32,
    idle: i32,
    max: i32,
    waiting: i32
  }
```

### Utility Tools

**`health`**

Quick health check. Runs `SELECT 1`, returns latency and pool status. Agents use this to verify the database is alive before starting work.

**`cancel`**

Cancel a running query by query ID (returned in streaming metadata). This lets agents abort expensive queries mid-stream if they realize they don't need the results.

**`listen` / `notify`**

```
listen:
  Parameters: channel: string
  Returns: starts streaming events from the channel as MCP notifications

notify:
  Parameters: channel: string, payload: string
  Returns: success: bool
```

This exposes Postgres LISTEN/NOTIFY for inter-agent communication. Agent A can notify a channel when it writes data, and Agent B (listening on that channel) can react immediately. This is a Postgres-native pub/sub primitive that no generic database MCP server would surface.

---

## Wire Protocol & Serialization

### Why Serialization Is the Bottleneck

In a database proxy server, the CPU-bound work is:
1. Deserializing Postgres wire protocol (binary row data → typed values)
2. Serializing typed values → JSON for MCP response
3. Framing JSON as SSE events

Steps 2 and 3 dominate. For a 10K-row result with 10 columns, the server serializes 100K values to JSON. At 50 bytes per value average, that's 5MB of JSON generation. This is where Rust vs. Go is a 3-5x difference.

### Serialization Strategy

**Use `simd-json` for JSON generation.** SIMD-accelerated JSON serialization. On x86_64 with AVX2, this is 2-4x faster than `serde_json`.

**Type-specific fast paths:**

```rust
// Instead of generic serde serialization, use type-aware formatting
match column_type {
    Type::INT4 => write_i32_fast(buf, row.get::<i32>(i)),
    Type::INT8 => write_i64_fast(buf, row.get::<i64>(i)),
    Type::FLOAT8 => write_f64_fast(buf, row.get::<f64>(i)),  // ryu for fast float formatting
    Type::TEXT | Type::VARCHAR => write_string_escaped(buf, row.get::<&str>(i)),
    Type::BOOL => buf.extend_from_slice(if row.get::<bool>(i) { b"true" } else { b"false" }),
    Type::TIMESTAMPTZ => write_timestamp_iso8601(buf, row.get::<DateTime>(i)),
    Type::UUID => write_uuid_hyphenated(buf, row.get::<Uuid>(i)),
    Type::JSONB => buf.extend_from_slice(row.get::<&[u8]>(i)),  // JSONB is already JSON — zero-copy passthrough
    _ => serde_json::to_writer(buf, &row.get::<serde_json::Value>(i)),  // fallback
}
```

**JSONB passthrough** is the biggest single optimization. When a column is `jsonb`, Postgres sends it as a JSON string on the wire. Instead of parsing it into a Rust value and re-serializing it, we copy the bytes directly into the output buffer. Zero-copy, zero-parse. For tables with JSONB columns (increasingly common), this eliminates the most expensive serialization work entirely.

**`ryu` for float formatting.** Rust's `ryu` crate formats f64 to string 3-5x faster than `format!("{}", f)`. For analytics queries with many numeric columns, this matters.

**Pre-allocated write buffers.** Each connection has a reusable 64KB write buffer. JSON is written into this buffer, then flushed to the SSE stream. Buffer is cleared (not deallocated) between batches. No per-row allocations.

---

## SQL Parser

pgmcp includes a lightweight SQL parser (`sqlparser-rs`) for:

1. **LIMIT injection:** Detect SELECT statements without LIMIT and append one.
2. **Statement classification:** Determine if a statement is read-only (SELECT) or mutating (INSERT/UPDATE/DELETE/DDL).
3. **Guardrails:** Detect dangerous patterns (UPDATE without WHERE, DROP TABLE, TRUNCATE).
4. **Parameter validation:** Count `$1`, `$2`, ... placeholders and verify against provided params.

The parser is used for analysis only — we never modify the SQL AST and regenerate SQL (too error-prone). We either append to the SQL string (LIMIT injection) or reject the query entirely (guardrails).

**What the parser does NOT do:**
- Rewrite queries for optimization (that's Postgres's job)
- Validate SQL syntax (send it to Postgres and let the database return the error)
- Generate SQL from natural language (that's the agent's job, or the cloud product's LLM integration)

---

## Error Handling

Every error returned to the agent is designed for LLM consumption. This means:

**Structured error with context:**

```json
{
  "error": {
    "code": "PERMISSION_DENIED",
    "message": "Cannot UPDATE the 'users' table: your credential scope only allows READ access to this table.",
    "detail": "Your current permissions: READ on [users, orders, products]. WRITE on [order_notes].",
    "hint": "To write to the users table, request a credential with write access from your administrator.",
    "pg_code": "42501",
    "pg_message": "permission denied for table users"
  }
}
```

**Agent-friendly error categories:**

| Code | Meaning | Agent should... |
|------|---------|----------------|
| `TABLE_NOT_FOUND` | Table doesn't exist | Call `list_tables` to find correct name |
| `COLUMN_NOT_FOUND` | Column doesn't exist | Call `describe_table` to see available columns |
| `TYPE_MISMATCH` | Wrong type in parameter | Check column type and adjust |
| `PERMISSION_DENIED` | Not allowed by scope | Call `my_permissions` to understand constraints |
| `QUERY_TIMEOUT` | Query exceeded timeout | Simplify query or increase timeout param |
| `QUERY_BLOCKED` | Guardrail prevented execution | Read the hint for what to change |
| `RATE_LIMITED` | Too many requests | Wait and retry (retry_after_ms provided) |
| `CONNECTION_ERROR` | Database unreachable | Call `health` to check status |
| `SYNTAX_ERROR` | Invalid SQL | Read pg_message for details |
| `CONSTRAINT_VIOLATION` | Unique/FK/check constraint | Read constraint details, adjust data |

Every error includes the `hint` field with a concrete next action. This is the most important field — it turns a failure into a recovery path.

---

## Configuration

```yaml
# pgmcp.yaml — minimal by default, exhaustive when needed

# Required: at least one of connection_string or host+database
connection_string: postgres://user:pass@localhost:5432/mydb

# OR individual fields:
# host: localhost
# port: 5432
# database: mydb
# user: user
# password: pass
# ssl_mode: prefer

# Server (all optional, sensible defaults)
server:
  port: 8765                # MCP listen port
  host: 127.0.0.1           # Bind address (localhost only by default — safe)
  
# Query defaults (all optional)
query:
  default_limit: 100        # LIMIT injected when none present
  max_limit: 10000          # Maximum allowed LIMIT
  timeout_seconds: 30       # Default query timeout
  max_timeout_seconds: 300   # Maximum allowed timeout
  
# Connection pool (all optional)
pool:
  max_connections: 20
  min_connections: 2

# Logging (all optional)
log:
  level: info               # debug, info, warn, error
  format: json              # json or text

# Guardrails (all optional, all default to true)
guardrails:
  block_full_table_update: true
  block_full_table_delete: true
  block_drop_table: true
  block_truncate: true
  auto_limit: true
  warn_sequential_scan: true
```

### Zero-Config Mode

For the fastest possible start, pgmcp accepts a connection string as the only argument:

```bash
pgmcp "postgres://localhost:5432/mydb"
```

No config file. All defaults. Server starts on port 8765, binds to localhost. This is the path from `cargo install pgmcp` to working MCP server in under 10 seconds.

### Environment Variables

Every config field has an env var override:

```bash
PGMCP_CONNECTION_STRING=postgres://...
PGMCP_SERVER_PORT=9999
PGMCP_QUERY_DEFAULT_LIMIT=500
PGMCP_POOL_MAX_CONNECTIONS=50
PGMCP_LOG_LEVEL=debug
```

Env vars override config file, which overrides defaults. No surprises.

---

## Distribution

### Binaries

Cross-compiled for every platform via `cross`:

```
pgmcp-linux-x86_64        (primary — servers)
pgmcp-linux-aarch64        (ARM servers, Graviton)
pgmcp-darwin-x86_64        (Intel Macs)
pgmcp-darwin-aarch64       (Apple Silicon — most developers)
pgmcp-windows-x86_64.exe   (Windows developers)
```

Static linking via `musl` on Linux — truly zero dependencies. The binary runs on any Linux kernel 3.2+.

### Install Methods

```bash
# 1. Cargo (Rust developers)
cargo install pgmcp

# 2. Homebrew (macOS/Linux)
brew install pgmcp/tap/pgmcp

# 3. curl one-liner (any platform)
curl -fsSL https://install.pgmcp.dev | sh

# 4. npm wrapper (Node developers — downloads the right binary)
npx pgmcp "postgres://localhost:5432/mydb"

# 5. Docker
docker run -p 8765:8765 ghcr.io/pgmcp/pgmcp \
  "postgres://host.docker.internal:5432/mydb"

# 6. Nix
nix run github:pgmcp/pgmcp -- "postgres://..."
```

### Docker Image

```dockerfile
FROM scratch
COPY pgmcp /pgmcp
ENTRYPOINT ["/pgmcp"]
```

Image size: ~8MB (Rust static binary, no OS, no shell). Compare: a typical Go binary in a scratch image is 15-20MB. Node-based MCP servers are 200MB+.

---

## Project Structure

```
pgmcp/
├── src/
│   ├── main.rs                    # entry point, config loading, server startup
│   ├── config.rs                  # config struct, YAML + env parsing, validation
│   │
│   ├── server/
│   │   ├── mod.rs
│   │   ├── mcp.rs                 # MCP protocol message types
│   │   ├── sse.rs                 # SSE transport (axum-based)
│   │   ├── handler.rs             # tool call dispatch
│   │   └── streaming.rs           # result streaming, batch sizing
│   │
│   ├── postgres/
│   │   ├── mod.rs
│   │   ├── pool.rs                # connection pool (deadpool-postgres)
│   │   ├── tools.rs               # tool definitions (metadata, schemas)
│   │   ├── discovery.rs           # list_tables, describe_table, list_schemas, etc.
│   │   ├── query.rs               # query execution, LIMIT injection, streaming
│   │   ├── explain.rs             # EXPLAIN parser, plain-language rules
│   │   ├── schema.rs              # propose_migration, execute_sql
│   │   ├── introspection.rs       # server_info, connection_info, my_permissions
│   │   ├── notify.rs              # LISTEN/NOTIFY
│   │   └── infer.rs               # column name → inferred description heuristics
│   │
│   ├── serialization/
│   │   ├── mod.rs
│   │   ├── json.rs                # simd-json fast paths, type-specific formatters
│   │   ├── csv.rs                 # CSV streaming serializer
│   │   └── types.rs               # Postgres type → JSON type mapping
│   │
│   ├── sql/
│   │   ├── mod.rs
│   │   ├── parser.rs              # sqlparser-rs wrapper, statement classification
│   │   ├── limit.rs               # LIMIT injection logic
│   │   └── guardrails.rs          # dangerous pattern detection
│   │
│   └── error.rs                   # error types, agent-friendly formatting
│
├── tests/
│   ├── integration/
│   │   ├── discovery_test.rs      # test list_tables, describe_table against real PG
│   │   ├── query_test.rs          # test query execution, streaming, LIMIT injection
│   │   ├── explain_test.rs        # test EXPLAIN parsing
│   │   ├── schema_test.rs         # test propose_migration, execute_sql
│   │   ├── guardrails_test.rs     # test dangerous query blocking
│   │   └── mcp_test.rs            # end-to-end MCP protocol tests
│   └── bench/
│       ├── serialization_bench.rs # JSON serialization benchmarks
│       ├── streaming_bench.rs     # row streaming throughput
│       └── connection_bench.rs    # concurrent connection overhead
│
├── docker-compose.test.yml        # PostgreSQL 14, 15, 16 for integration tests
├── pgmcp.example.yaml
├── Cargo.toml
├── Dockerfile
├── Cross.toml                     # cross-compilation config
├── .github/workflows/
│   ├── ci.yml                     # test + lint + bench on every PR
│   └── release.yml                # build + publish on tag
├── README.md
├── LICENSE                        # Apache 2.0
└── CONTRIBUTING.md
```

---

## Milestones

### M1: Core (Week 1-2)

Ship: `list_tables`, `describe_table`, `list_schemas`, `list_enums`, `server_info`, `query` (with LIMIT injection and streaming), `explain`, `health`, `connection_info`.

SSE transport via axum. Connection pooling via deadpool-postgres. Config file + env vars + CLI flags. JSON serialization with simd-json fast paths. Guardrails (block full-table mutations). Structured error handling.

Docker image. README with quickstart. `cargo install pgmcp` works.

**Done when:** Claude Desktop connects via MCP, discovers tables, queries data, and gets streaming results from a local Postgres instance. The whole experience takes <60 seconds from install to first query.

### M2: Schema + Intelligence (Week 3-4)

Ship: `propose_migration`, `execute_sql`, `suggest_index`, `table_stats`, `list_extensions`. EXPLAIN plain-language rule engine (50 rules). Inferred column descriptions (200 patterns). `json_compact` and `csv` output formats. Parameter validation with clear error messages.

Performance benchmarks in CI with regression detection.

**Done when:** An agent can understand a table's structure, query it, identify performance problems, suggest fixes, and propose schema changes — all through MCP tools with no human intervention.

### M3: Distribution + Launch (Week 5-6)

Ship: Homebrew tap, npm wrapper, curl installer, Nix flake. Cross-compiled binaries for 5 platforms. Integration tests against PG 14, 15, 16. Integration guides for Claude Desktop, LangChain, LlamaIndex, CrewAI. LISTEN/NOTIFY tools. `cancel` tool.

Launch: GitHub, Hacker News, Reddit, Twitter/X. Submit PRs to framework docs.

**Done when:** 500 GitHub stars. 100 active installations. Referenced in at least 2 framework documentation sites.

### M4: Hardening (Week 7-8)

Fix everything that broke in the first 2 weeks of real usage. Performance tune based on real-world query patterns. Add tests for edge cases users reported. Improve inferred descriptions based on feedback. Write a blog post on the architecture and performance characteristics.

**Done when:** Zero known bugs. Benchmark results published. Post-mortem on what surprised us.
