# Phase 7 — Intelligence Tools

**Date:** 2026-04-07  
**Branches:** feat/019 through feat/023  
**Status:** Implementation plan

---

## Overview

Phase 7 adds the five "intelligence" tools that transform pgmcp from a data-access
layer into an analytical assistant. These tools reason about queries, schemas, and
permissions rather than just executing SQL or listing catalog rows.

All five tools follow the same structural pattern established in Phase 6:
- Parameter extraction helpers return `McpError::param_invalid` on bad input
- SQL-accepting tools pass through `sql::parser` + `sql::guardrails` first
- DB-touching tools acquire a pool connection with `acquire_timeout` from config
- Results are serialized as pretty-printed JSON via `serde_json::to_string_pretty`
- Every tool has pure-unit tests (no DB) and integration tests (container)

---

## Branch Sequence

### feat/019 — explain tool

**File:** `src/tools/explain.rs`

**Design decisions:**

The `query` tool already has an `explain: true` flag that prepends
`EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`. The standalone `explain` tool is
different: it owns the full EXPLAIN lifecycle, offers a `verbose` flag, provides
a plain-language analysis of the plan, and is the canonical path for LLM
plan inspection (no result rows returned).

**Parameter model:**

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `sql` | string | required | Passed through guardrails |
| `analyze` | bool | false | ANALYZE adds real execution stats |
| `verbose` | bool | false | VERBOSE adds per-column output |
| `buffers` | bool | true | BUFFERS requires analyze=true |

Note: `analyze` defaults to **false** here (unlike `query`'s explain mode).
The explain tool is designed for safe pre-flight analysis where the user may
not want to actually run the statement.

**EXPLAIN SQL construction:**

```
EXPLAIN (FORMAT JSON [, ANALYZE] [, VERBOSE] [, BUFFERS when analyze=true])
```

BUFFERS is only valid when ANALYZE is true (Postgres rejects BUFFERS without ANALYZE).

**Plain-language rule engine (~25 rules):**

Implemented as a pure function `analyze_plan(plan_json) -> PlanAnalysis` that
walks the JSON tree recursively.

Rules (deterministic, order-independent):

1. Seq Scan on table with `rows` estimate > 10,000 — suggest index
2. Seq Scan with filter condition present — "filter applied after full scan"
3. Nested Loop with outer rows > 1,000 — potential N+1 warning
4. Hash Join with `Batches > 1` — join spilled to disk (work_mem too small)
5. Sort with `Sort Method: external` — sort spilled to disk
6. Sort with `Sort Method: external merge` — ditto
7. `Rows Removed by Filter` > 50% of rows scanned — poor selectivity
8. Node total cost > 10,000 — high-cost node flagged
9. `Startup Cost` > 1,000 on inner Nested Loop node — expensive setup repeated per outer row
10. Index Scan on partial index with condition not matching — warning
11. `Parallel workers: 0` when cost > 100,000 — parallelism not used
12. `Loops` > 1 on expensive node (cost-per-loop * loops > 50,000) — loop amplification
13. `Width` > 4096 — very wide rows (potential TOAST overhead)
14. `Shared Hit Blocks` very low relative to rows — cold cache
15. `Shared Read Blocks` dominant — most data read from disk
16. `Temp Written Blocks` > 0 — temp file usage (sort or hash spill)
17. Aggregate with `Strategy: Sorted` and large input — presort before aggregation
18. Unique node (deduplication) with large input — `DISTINCT` on unsorted data
19. WindowAgg with no suitable index — window function doing full scan
20. Seq Scan on system catalog (pg_*) — usually fine but flagged for awareness
21. BitmapHeap Scan with `Recheck Cond: true` and many lossy pages — heap recheck overhead
22. Plan total cost = 0.0 — plan not yet optimized (shouldn't happen; indicates stale stats)
23. Plan rows estimate = 1 on multi-row node — stale statistics / no ANALYZE
24. Function Scan — result not cacheable, executed per outer row in loops
25. CTE Scan with `CTE fenced` — CTE optimization fence (PG < 12 or WITH MATERIALIZED)

**Output structure:**

```json
{
  "sql": "...",
  "plan_json": [...],
  "plan_text": "...",
  "summary": {
    "total_cost": 1234.56,
    "estimated_rows": 500,
    "node_count": 7,
    "warnings": ["Seq Scan on orders (50,000 rows): consider an index on status"],
    "suggestions": ["Add index: CREATE INDEX CONCURRENTLY ON orders(status)"]
  }
}
```

`plan_text` is obtained by running a second `EXPLAIN (FORMAT TEXT)` query on the
same SQL (when `analyze=false`, this is zero-cost — just estimation). When
`analyze=true`, only the JSON plan is fetched to avoid double-execution; `plan_text`
is synthesized by pretty-printing the JSON node tree in a text-like format.

**Guardrails:** SQL passes through the same `check()` function as `query`. DDL
is blocked by default. Dry-run is not supported (explain itself is the analysis).

**Tests:**
- Unit: parameter extraction, rule engine with crafted JSON plans
- Integration: real PG with test tables, verify plan shape

---

### feat/020 — my_permissions

**File:** `src/tools/my_permissions.rs`  
**SQL:** `src/pg/queries/my_permissions.sql`

**Design decisions:**

Introspects the **current session role** (i.e., the role pgmcp connected as).
Does not accept a role name parameter — agents cannot impersonate other roles
through this tool.

Uses `current_user` / `current_role` to identify the connected role, then:
1. Queries `pg_roles` for role attributes (superuser, createdb, createrole,
   inherit, login, replication, bypassrls, connection limit)
2. Queries `has_schema_privilege(schema, 'USAGE')` and `has_schema_privilege(schema, 'CREATE')`
   for each schema the role can see
3. Optionally queries `has_table_privilege(table, privilege)` for a specific
   table when `table` parameter is provided

**Parameters:**

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `schema` | string | `"public"` | Schema to check schema-level privileges for |
| `table` | string | optional | If set, check table-level privileges too |

**SQL queries (inline, not file-based — the SQL is too dynamic for static files):**

Query 1 — role attributes:
```sql
SELECT
  current_user AS role_name,
  rolsuper, rolcreatedb, rolcreaterole, rolinherit,
  rolcanlogin, rolreplication, rolbypassrls,
  rolconnlimit
FROM pg_roles
WHERE rolname = current_user
```

Query 2 — schema privileges (one row per schema):
```sql
SELECT
  schema_name,
  has_schema_privilege(schema_name, 'USAGE') AS can_usage,
  has_schema_privilege(schema_name, 'CREATE') AS can_create
FROM information_schema.schemata
WHERE schema_name NOT IN ('pg_toast', 'pg_catalog', 'information_schema')
  AND schema_name NOT LIKE 'pg_temp_%'
  AND schema_name NOT LIKE 'pg_toast_temp_%'
ORDER BY schema_name
```

Query 3 — table privileges (only when `table` param provided):
```sql
SELECT
  has_table_privilege($1, 'SELECT') AS can_select,
  has_table_privilege($1, 'INSERT') AS can_insert,
  has_table_privilege($1, 'UPDATE') AS can_update,
  has_table_privilege($1, 'DELETE') AS can_delete,
  has_table_privilege($1, 'TRUNCATE') AS can_truncate,
  has_table_privilege($1, 'REFERENCES') AS can_references
```

**Output structure:**

```json
{
  "role": {
    "name": "pgmcp_user",
    "is_superuser": false,
    "can_create_db": false,
    "can_create_role": false,
    "inherits": true,
    "can_login": true,
    "is_replication": false,
    "bypass_rls": false,
    "connection_limit": -1
  },
  "schema_privileges": [
    { "schema": "public", "usage": true, "create": false }
  ],
  "table_privileges": {
    "table": "public.orders",
    "select": true,
    "insert": false,
    "update": false,
    "delete": false,
    "truncate": false,
    "references": false
  }
}
```

`table_privileges` is omitted when `table` parameter is not provided.

**Tests:**
- Unit: parameter extraction
- Integration: verify role name matches, schema privileges returned

---

### feat/021 — suggest_index

**File:** `src/tools/suggest_index.rs`

**Design decisions:**

Runs `EXPLAIN (FORMAT JSON)` on the provided SQL (no ANALYZE — we don't want
side effects, and plan-shape analysis suffices for index suggestions). Walks
the plan tree to find Seq Scan nodes. For each Seq Scan on a table with
estimated rows > threshold (default 1,000), generates a `CREATE INDEX` suggestion.

**Parameters:**

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `sql` | string | required | SELECT only; DDL/DML blocked by guardrails |
| `schema` | string | `"public"` | Default schema for unqualified table refs |

**Algorithm:**

1. Parse + guardrail the SQL (must be SELECT)
2. Run `EXPLAIN (FORMAT JSON)` — estimation only, no ANALYZE
3. Walk the JSON plan tree depth-first
4. Collect `SeqScan` nodes where:
   - `Plan Rows` > 1,000 (configurable internally, not exposed as param)
   - `Filter` condition is present (if no filter, full-scan may be intentional)
5. For each candidate Seq Scan:
   a. Extract table name and filter columns from the `Filter` expression
      (best-effort text parse — extract identifiers before `=`, `<`, `>`, `LIKE`)
   b. Check existing indexes on the table via pg_catalog
   c. If no suitable index exists, generate suggestion
6. Estimate index size: `estimated_rows * avg_column_width * 1.3` (btree overhead)

**Index generation heuristic:**

For a filter `col1 = $1 AND col2 > $2`:
- Suggest composite index on `(col1, col2)` — equality first, range last
- For single equality: `(col)` index
- For ORDER BY: include ORDER BY columns in the index

**Output structure:**

```json
{
  "sql_analyzed": "SELECT ...",
  "current_plan_cost": 12345.67,
  "suggestions": [
    {
      "table": "orders",
      "reason": "Sequential scan on orders (estimated 50,000 rows) with filter on status",
      "create_sql": "CREATE INDEX CONCURRENTLY ON orders (status)",
      "estimated_index_size_bytes": 4096000,
      "impact": "high",
      "tradeoffs": "Adds write overhead; speeds up queries filtering on status"
    }
  ]
}
```

**Tests:**
- Unit: plan tree walker with crafted JSON, filter column extractor
- Integration: unindexed test table, verify suggestion generated

---

### feat/022 — propose_migration

**File:** `src/tools/propose_migration.rs`

**Design decisions:**

Accepts a DDL SQL string, parses it with sqlparser, and returns a safety
assessment. Does NOT execute the SQL. This is pure static analysis.

The tool is version-aware: it reads the server version from the pool (or a
cached value) to emit version-specific warnings.

**Parameters:**

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `sql` | string | required | Must be DDL (CREATE/ALTER/DROP) |

**DDL classification:**

The input must be DDL. Non-DDL (SELECT, INSERT, etc.) returns a
`param_invalid` error: "propose_migration only accepts DDL statements".

**Analysis pipeline:**

1. Parse SQL with sqlparser
2. Classify statement kind — must be DDL
3. Generate reverse SQL (the undo statement)
4. Assess lock type:
   - `CREATE TABLE` — `AccessShareLock` (not blocking)
   - `ALTER TABLE ADD COLUMN nullable` — `AccessExclusiveLock` (table rewrite? no for nullable)
   - `ALTER TABLE ADD COLUMN NOT NULL` — `AccessExclusiveLock` (table rewrite on PG < 11)
   - `ALTER TABLE ADD COLUMN NOT NULL DEFAULT` — safe on PG >= 11 (avoids rewrite)
   - `ALTER TABLE DROP COLUMN` — `AccessExclusiveLock`
   - `ALTER TABLE ALTER COLUMN TYPE` — `AccessExclusiveLock` + possible table rewrite
   - `DROP TABLE` — `AccessExclusiveLock`
   - `CREATE INDEX` (without CONCURRENTLY) — `ShareLock` (blocks writes)
   - `CREATE INDEX CONCURRENTLY` — no blocking lock
   - `DROP INDEX` — `AccessExclusiveLock`
   - `TRUNCATE` — `AccessExclusiveLock`
5. Assess data loss risk: DROP TABLE, DROP COLUMN, TRUNCATE → high risk
6. Generate warnings list

**Reverse SQL generation:**

| DDL | Reverse SQL |
|-----|-------------|
| `CREATE TABLE t (...)` | `DROP TABLE IF EXISTS t` |
| `ALTER TABLE t ADD COLUMN c type` | `ALTER TABLE t DROP COLUMN c` |
| `ALTER TABLE t DROP COLUMN c` | *(cannot reverse — data is gone)* |
| `CREATE INDEX idx ON t (c)` | `DROP INDEX IF EXISTS idx` |
| `DROP INDEX idx` | *(cannot reverse — must recreate)* |
| `DROP TABLE t` | *(cannot reverse — data is gone)* |
| `TRUNCATE t` | *(cannot reverse — data is gone)* |

When reverse is not possible, `reverse_sql` is `null` with an explanation.

**Version-aware warnings (PG version from `SHOW server_version_num`):**

- PG < 11: `ALTER TABLE ADD COLUMN ... NOT NULL DEFAULT` triggers full table rewrite
- PG < 12: `CREATE INDEX CONCURRENTLY` inside a transaction block fails
- PG < 14: `EXPLAIN ANALYZE` cannot abort early on canceled queries
- Any version: `DROP TABLE` without `IF EXISTS` fails if table absent
- Any version: `ALTER TABLE ... TYPE` may require `USING` clause for non-trivial casts

**Output structure:**

```json
{
  "sql": "ALTER TABLE orders ADD COLUMN archived_at TIMESTAMPTZ",
  "statement_type": "alter_table",
  "is_destructive": false,
  "reverse_sql": "ALTER TABLE orders DROP COLUMN archived_at",
  "lock_type": "AccessExclusiveLock",
  "lock_risk": "medium",
  "downtime_risk": "low",
  "data_loss_risk": "none",
  "warnings": [
    "ALTER TABLE acquires AccessExclusiveLock, blocking all reads and writes during migration."
  ],
  "suggestions": [
    "Consider adding the column as nullable first, backfilling data, then adding the NOT NULL constraint."
  ],
  "pg_version": "16.1"
}
```

**Tests:**
- Unit: all statement type classifications, reverse SQL generation, warning generation
- Integration: version check from real PG

---

### feat/023 — inferred column descriptions

**File:** `src/pg/infer.rs`

**Design decisions:**

A pure function `infer_column_description(col_name: &str, col_type: &str) -> Option<String>`
that applies ~200 heuristic patterns. Called from `describe_table` when
`col_description` is `NULL` (no COMMENT set on the column).

The function is intentionally simple: it uses a match-then-check approach,
not a regex engine. Pattern matching is done with string operations (ends_with,
starts_with, contains) for zero dependencies and maximum speed.

**Pattern categories (with example rules):**

1. **FK conventions** — columns ending in `_id`
   - `user_id` → "Foreign key reference to the users table"
   - `account_id` → "Foreign key reference to the accounts table"
   - `*_uuid` → "UUID foreign key reference"

2. **Primary key** — column named `id`, `uuid`, `oid`, `pk`
   - `id` + integer type → "Primary key (auto-incrementing integer)"
   - `id` + uuid type → "Primary key (UUID)"
   - `uuid` → "Primary key UUID"

3. **Timestamps** — `*_at`, `*_on`, `created`, `updated`, `deleted`
   - `created_at` → "Timestamp when the record was created"
   - `updated_at` → "Timestamp when the record was last updated"
   - `deleted_at` → "Soft-delete timestamp; null means the record is active"
   - `published_at` → "Timestamp when the record was published"
   - `*_at` generic → "Timestamp for the event indicated by the column name"

4. **Booleans** — `is_*`, `has_*`, `can_*`, `allow_*`, `enable_*`, `active`, `enabled`
   - `is_active` → "Whether this record is currently active"
   - `is_deleted` → "Soft-delete flag; true means the record has been removed"
   - `has_*` → "Whether this record has the property indicated by the column name"

5. **Monetary** — `*_cents`, `*_amount`, `*_price`, `*_cost`, `*_fee`, `*_total`
   - `price_cents` → "Price in cents (divide by 100 for dollars)"
   - `*_amount` + numeric type → "Monetary amount"
   - `*_price` → "Price value"

6. **Contact/identity** — `email`, `*_email`, `phone`, `*_phone`, `url`, `*_url`,
   `username`, `slug`, `handle`
   - `email` → "Email address"
   - `phone_number` → "Phone number"
   - `website_url` → "URL"
   - `slug` → "URL-friendly identifier (slug)"

7. **Counters/aggregates** — `*_count`, `*_total`, `*_sum`, `*_num`, `*_qty`
   - `view_count` → "Number of times viewed"
   - `order_total` → "Total amount of the order"
   - `retry_count` → "Number of retry attempts"

8. **JSON/metadata** — `metadata`, `extra`, `properties`, `attributes`, `settings`,
   `config`, `options`, `data`, `payload`, `context`
   - `metadata` → "Arbitrary metadata stored as JSON"
   - `settings` → "Configuration settings stored as JSON"

9. **Arrays** — columns with `[]` type suffix, or `*_ids`, `*_tags`, `*_list`
   - `tag_ids` → "Array of foreign key references"
   - `tags` + array type → "Array of tag values"

10. **Size/measurement** — `*_size`, `*_length`, `*_width`, `*_height`, `*_weight`,
    `*_bytes`, `*_kb`, `*_mb`
    - `file_size` → "File size in bytes"
    - `content_length` → "Content length in bytes"

11. **Version/rank** — `version`, `*_version`, `rank`, `position`, `order`, `priority`
    - `version` → "Record version number"
    - `rank` → "Sort rank or priority order"

12. **Geographic** — `latitude`, `longitude`, `lat`, `lng`, `lon`, `location`,
    `address`, `city`, `country`, `zip`, `postal_code`
    - `latitude` → "Geographic latitude in decimal degrees"
    - `longitude` → "Geographic longitude in decimal degrees"
    - `country_code` → "ISO country code"

13. **Auth/security** — `password`, `*_hash`, `*_token`, `*_secret`, `api_key`,
    `*_salt`, `session_id`
    - `password_hash` → "Hashed password (bcrypt or similar)"
    - `api_key` → "API authentication key"
    - `session_id` → "Session identifier"

14. **Status/state** — `status`, `state`, `stage`, `phase`, `step`
    - `status` → "Current status of the record"
    - `state` → "Current state in the workflow"

15. **Name/description** — `name`, `title`, `label`, `description`, `summary`,
    `body`, `content`, `text`, `note`, `comment`, `message`
    - `name` → "Display name"
    - `title` → "Title or heading"
    - `description` → "Human-readable description"

**Wiring into describe_table:**

`build_column` in `src/tools/describe_table.rs` currently returns
`description: col_description` where `col_description` is the Postgres column
comment or `None`. After this branch, when `col_description` is `None`, we call
`infer_column_description(&name, &col_type)` and use its result as the description.

The returned JSON shape does not change — `description` remains a
`Option<String>`. Agents cannot distinguish inferred from explicit descriptions
(by design — the heuristic is a best-effort convenience).

**Tests:**
- Unit: one test per pattern category verifying key examples
- Unit: verify no false matches (e.g., `address` column typed as integer gets
  no geographic description)
- Integration: describe_table with a column lacking COMMENT returns inferred description

---

## Implementation Notes

### Shared utilities

The `explain` and `suggest_index` tools both need to walk EXPLAIN JSON plan trees.
A shared private function `walk_plan_nodes` is placed in `src/tools/explain.rs`
and re-used by `suggest_index.rs` via a module-private import path (or inlined
in each file to keep coupling minimal — chosen approach: inline, since the walking
logic differs in what is collected).

### Parameter binding for my_permissions table check

The `has_table_privilege($1, privilege)` function accepts a table name as `regclass`
text. We pass `schema.table` as the argument. This works without quoting issues
because the parameter is typed as `text` in the extended protocol and Postgres
coerces it to regclass.

### propose_migration PG version detection

The tool queries `SHOW server_version_num` on an acquired connection. The integer
value (e.g., `160001` for PG 16.1) is parsed and passed to `assess_warnings`.
This is cached per tool call — not across calls — to avoid stale state after
major upgrades.

### No new dependencies

All five branches use only the dependencies already in Cargo.toml. The plan
analysis is pure Rust string/integer logic; no regex crate is needed.

---

## Test Strategy

| Branch | Unit tests | Integration tests |
|--------|-----------|------------------|
| feat/019 | rule engine (~25 tests), param extraction | EXPLAIN on real tables |
| feat/020 | param extraction | role attrs, schema privileges, table privileges |
| feat/021 | plan walker, filter extractor | unindexed table → suggestion generated |
| feat/022 | all DDL types, reverse SQL, warnings | version string from PG |
| feat/023 | all pattern categories (~30 tests) | describe_table uses inferred desc |

---

## Acceptance Criteria

- [ ] All 5 tools respond with non-stub output
- [ ] `cargo test` passes (all existing + new tests)
- [ ] `cargo clippy -- -D warnings` produces zero warnings
- [ ] `cargo fmt --check` passes
- [ ] Each tool's integration test exercises the real code path against a PG container
- [ ] `describe_table` returns inferred descriptions for columns without COMMENTs
- [ ] `propose_migration` does not execute any SQL
- [ ] `suggest_index` does not use ANALYZE in its EXPLAIN call
