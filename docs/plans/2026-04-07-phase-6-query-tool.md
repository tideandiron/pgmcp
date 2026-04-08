# Phase 6 Plan: Streaming Serialization + Query Tool

**Date:** 2026-04-07
**Branches:** feat/017 (streaming), feat/018 (query tool)
**Author:** Rust Engineer Agent

---

## 1. Objective

Phase 6 delivers the final two implementation slices that make pgmcp useful for its primary purpose: executing SQL queries from AI agents. The two features are tightly coupled — the serialization layer feeds directly into the query tool — so they ship in sequence.

- **feat/017**: Streaming serialization layer (BatchSizer, JSON encoder, CSV encoder, OID type helpers)
- **feat/018**: Query tool full implementation (the primary agent-facing data access tool)

---

## 2. Current State

- 469 tests passing across lib, integration, and unit test harnesses.
- SQL parser, guardrails, and LIMIT injection all implemented and tested (phases 14–16).
- Schema cache, connection pool, and all 8 discovery/introspection tools implemented.
- `src/streaming/json.rs` and `src/streaming/csv.rs` are empty stubs.
- `src/streaming/mod.rs` exports both submodules but provides no public API.
- `src/tools/query.rs` returns `{"status":"not_yet_implemented"}`.
- `src/tools/query_events.rs` is an empty comment file.
- `src/pg/types.rs` is empty.
- `benches/serialization.rs` has a `fn main() {}` placeholder.

---

## 3. feat/017 — Streaming Serialization

### 3.1 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/pg/types.rs` | Create | PG OID constants and `pg_type_name` helper |
| `src/streaming/mod.rs` | Modify | Add `BatchSizer` struct and exports |
| `src/streaming/json.rs` | Implement | JSON row encoder with OID fast paths |
| `src/streaming/csv.rs` | Implement | CSV row encoder |
| `benches/serialization.rs` | Implement | Criterion benchmarks |

### 3.2 BatchSizer Design

The `BatchSizer` computes adaptive batch sizes per the spec (section 3.2, Streaming + Serialization):

```
Initial batch size:  100 rows
After first batch:   avg_row_bytes = total_encoded_bytes / row_count
Next batch size:     clamp(TARGET_EVENT_BYTES / avg_row_bytes, 1, 1_000)
TARGET_EVENT_BYTES:  65_536 (64 KB)
Reset:               each new query call
```

The sizer holds state across batches:
- `total_bytes_encoded: usize` — cumulative byte count
- `total_rows_encoded: usize` — cumulative row count
- `next_batch_size: usize` — size to use for next batch

For the MVP, "streaming" means collecting all rows into one buffer and returning the complete payload. The sizer controls memory by partitioning the iteration — in a true SSE future, the sizer would control event boundaries.

### 3.3 JSON Encoder Fast Paths

The encoder writes directly to a `Vec<u8>` buffer without going through `serde_json::Value` allocation. Type dispatch is by PG OID:

| PG OID(s) | Rust path | Output |
|-----------|-----------|--------|
| INT2 (21), INT4 (23), INT8 (20) | `i16`, `i32`, `i64` from row | `itoa`-style digit write |
| FLOAT4 (700), FLOAT8 (701) | `f32`, `f64` from row | `ryu::Buffer` |
| TEXT (25), VARCHAR (1043), NAME (19), BPCHAR (1042) | `&str` from row | JSON-escaped string |
| BOOL (16) | `bool` | `b"true"` or `b"false"` |
| JSONB (3802), JSON (114) | `&str` from row | Raw copy (zero-copy passthrough) |
| UUID (2950) | `uuid::Uuid` from row | Hyphenated hex string |
| TIMESTAMPTZ (1184) | `time::OffsetDateTime` from row | RFC 3339 formatted |
| TIMESTAMP (1114) | `time::PrimitiveDateTime` from row | ISO 8601 without timezone |
| DATE (1082) | `time::Date` from row | ISO 8601 date |
| INT4ARRAY (1007), INT8ARRAY (1016) | Fallback | `serde_json::to_writer` |
| NULL (any type, IS NULL) | — | `b"null"` |
| Unknown OID | Fallback | `serde_json::to_writer` via `row.get::<_, serde_json::Value>()` |

The encoder handles NULL before the OID dispatch — `row.try_get()` failure indicates NULL when the column is nullable.

### 3.4 CSV Encoder

The CSV encoder uses RFC 4180 rules:
- Fields containing comma, newline, or double-quote are quoted.
- Double-quotes inside quoted fields are escaped as `""`.
- NULL values serialize as empty field.
- Booleans serialize as `true`/`false`.
- Numbers serialize as their decimal representation.
- Header row is always emitted first.

### 3.5 OID Constants (pg/types.rs)

```rust
pub(crate) const OID_BOOL: u32 = 16;
pub(crate) const OID_INT2: u32 = 21;
pub(crate) const OID_INT4: u32 = 23;
pub(crate) const OID_INT8: u32 = 20;
pub(crate) const OID_FLOAT4: u32 = 700;
pub(crate) const OID_FLOAT8: u32 = 701;
pub(crate) const OID_TEXT: u32 = 25;
pub(crate) const OID_VARCHAR: u32 = 1043;
pub(crate) const OID_NAME: u32 = 19;
pub(crate) const OID_BPCHAR: u32 = 1042;
pub(crate) const OID_JSON: u32 = 114;
pub(crate) const OID_JSONB: u32 = 3802;
pub(crate) const OID_UUID: u32 = 2950;
pub(crate) const OID_TIMESTAMPTZ: u32 = 1184;
pub(crate) const OID_TIMESTAMP: u32 = 1114;
pub(crate) const OID_DATE: u32 = 1082;
pub(crate) const OID_NUMERIC: u32 = 1700;
pub(crate) const OID_OID: u32 = 26;
pub(crate) const OID_BYTEA: u32 = 17;
```

### 3.6 Benchmark Plan

`benches/serialization.rs` benchmarks (using `criterion`):
1. `bench_json_encode_integers` — 1000 rows of (i32, i64, f64)
2. `bench_json_encode_text` — 1000 rows of text columns of varying width
3. `bench_json_encode_mixed` — 1000 rows with all fast-path types
4. `bench_csv_encode_mixed` — same mixed dataset
5. `bench_batch_sizer_adapt` — sizer convergence over 10 batches

### 3.7 Tests

Each fast path gets a unit test:
- `test_json_null_column` — NULL produces `null`
- `test_json_bool_true` / `test_json_bool_false`
- `test_json_int2`, `test_json_int4`, `test_json_int8`
- `test_json_float4`, `test_json_float8`
- `test_json_text_plain`, `test_json_text_needs_escaping`
- `test_json_uuid`
- `test_json_timestamptz`
- `test_batch_sizer_initial_size` — first call returns 100
- `test_batch_sizer_adapts_after_first_batch` — after recording 100 rows of 64 bytes each, next batch ≈ 1000
- `test_batch_sizer_clamps_to_max` — never exceeds 1000
- `test_batch_sizer_clamps_to_min` — never falls below 1
- `test_csv_null` — NULL produces empty field
- `test_csv_quoting` — field with comma is quoted
- `test_csv_double_quote_escape` — `"` becomes `""`
- `test_csv_header_row`

### 3.8 Implementation Note: tokio-postgres Row API

The `tokio_postgres::Row` API used for column extraction:
- `row.columns()` → `&[Column]` with name and type OID
- `row.try_get::<_, T>(i)` → `Result<T, _>`; returns `Err` for NULL on non-Option types
- `row.try_get::<_, Option<T>>(i)` → always `Ok`; NULL gives `Ok(None)`

For the fast paths, we use `Option<T>` extraction to handle NULL uniformly before OID dispatch.

---

## 4. feat/018 — Query Tool

### 4.1 Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `src/tools/query.rs` | Replace stub | Full implementation |
| `src/tools/query_events.rs` | Implement | Response construction helpers |

### 4.2 Parameter Extraction

All 8 parameters are extracted from the JSON args map:

| Param | Type | Default | Validation |
|-------|------|---------|------------|
| `sql` | string | (required) | non-empty |
| `params` | array | `[]` | (not used in MVP — reserved) |
| `intent` | string | `""` | logged only |
| `limit` | u32 | 100 | 1..=10_000 |
| `timeout_seconds` | u64 | 30 | 1..=300 |
| `format` | enum | `json` | `"json"` \| `"json_compact"` \| `"csv"` |
| `transaction` | bool | false | — |
| `dry_run` | bool | false | — |
| `explain` | bool | false | — |

### 4.3 Execution Pipeline

```
1.  Extract and validate parameters
2.  Parse SQL via sql/parser.rs → ParsedStatement
3.  Run guardrails via sql/guardrails.rs (using config.guardrails)
4.  Intent validation: if intent=read but SQL is mutating → McpError::guardrail_violation
    (Note: "intent" param is a string for logging; there is no read/mutate enum in the spec —
     this check is omitted for MVP; the param is logged only)
5.  For SELECT: inject LIMIT via sql/limit.rs
6.  If dry_run: return parse+guardrail analysis without executing
7.  If explain: prepend EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) to the final SQL
8.  Acquire connection from pool (timeout from config)
9.  Set statement_timeout via SET LOCAL
10. If transaction: BEGIN
11. Execute query, collect rows
12. Serialize rows through json/csv encoder using BatchSizer
13. If transaction: ROLLBACK
14. Release connection
15. Return CallToolResult with metadata
```

### 4.4 dry_run Response

```json
{
  "dry_run": true,
  "statement_kind": "Select",
  "sql_analyzed": "SELECT * FROM users LIMIT 100",
  "guardrails_passed": true,
  "limit_injected": true,
  "row_count": null
}
```

### 4.5 Normal Response (JSON format)

```json
{
  "columns": [{"name": "id", "type": "int4"}, {"name": "name", "type": "text"}],
  "rows": [{"id": 1, "name": "alice"}, ...],
  "row_count": 42,
  "format": "json",
  "sql_executed": "SELECT id, name FROM users LIMIT 100",
  "execution_time_ms": 12.4,
  "limit_injected": true
}
```

### 4.6 Normal Response (CSV format)

```json
{
  "columns": [{"name": "id", "type": "int4"}, {"name": "name", "type": "text"}],
  "rows": "id,name\n1,alice\n2,bob\n",
  "row_count": 2,
  "format": "csv",
  "sql_executed": "SELECT id, name FROM users LIMIT 100",
  "execution_time_ms": 8.1,
  "limit_injected": false
}
```

### 4.7 explain Response (when explain: true)

The explain plan is prepended to the normal response:
```json
{
  "columns": [...],
  "rows": [...],
  "row_count": 42,
  "format": "json",
  "sql_executed": "SELECT ...",
  "execution_time_ms": 18.2,
  "plan": { ... }
}
```

### 4.8 Error Cases

| Condition | Error Code |
|-----------|------------|
| Missing `sql` parameter | `param_invalid` |
| Invalid `format` value | `param_invalid` |
| `limit` out of range | `param_invalid` |
| SQL parse failure | `sql_parse_error` |
| Guardrail violation | `guardrail_violation` |
| Pool timeout | `pg_pool_timeout` |
| Query execution failure | `pg_query_failed` |
| Statement timeout | `pg_query_failed` |

### 4.9 Tests

Unit tests (no DB required):
- `test_extract_params_sql_required` — missing sql → param_invalid
- `test_extract_params_defaults` — all optional params use their defaults
- `test_extract_params_limit_max` — limit=10001 → param_invalid
- `test_extract_params_limit_min` — limit=0 → param_invalid
- `test_extract_params_format_csv` — format="csv" accepted
- `test_extract_params_format_json_compact` — format="json_compact" accepted
- `test_extract_params_format_invalid` — format="xml" → param_invalid
- `test_extract_params_timeout_default`
- `test_extract_params_dry_run_true`
- `test_extract_params_explain_true`
- `test_extract_params_transaction_true`

query_events.rs tests:
- `test_dry_run_response_structure`
- `test_success_response_json_fields`
- `test_success_response_csv_fields`
- `test_error_response_wraps_mcp_error`

---

## 5. Deviations from Spec

### 5.1 Simplified "Streaming" (MVP)

The design notes specify that for MVP, streaming is simplified: rows are collected into a single `Vec<u8>` buffer and returned as one `CallToolResult`. True SSE streaming (emitting progress events mid-result) requires MCP client support that is not yet stable. The `BatchSizer` is still implemented and drives the buffer-growth strategy.

### 5.2 params[] Array (Reserved)

The `params[]` parameter (for `$1`, `$2`... bindings) is accepted but not executed in MVP. The SQL is always run as a simple query (`client.query(sql, &[])`) without parameter binding. Parameter binding requires the extended query protocol and type inference, which adds complexity out of scope for this phase.

### 5.3 Intent Read/Mutate Enum

The spec mentions `intent` as `read` vs `mutate`. The tool definition uses `intent` as a free-form string for logging. There is no programmatic read/mutate enforcement in the MVP — guardrails enforce safety through SQL analysis regardless of intent.

### 5.4 TIMESTAMPTZ Serialization

`time::OffsetDateTime` requires the `formatting` feature, which is in `Cargo.toml`. RFC 3339 is produced via `time::format_description::well_known::Rfc3339`.

---

## 6. Commit Strategy

```
feat(017): implement streaming JSON/CSV serialization with type-specific fast paths
feat(018): implement query tool with full execution pipeline
```

Both are squash-merged to main after tests pass.

---

## 7. Test Count Target

- feat/017 adds approximately 25 unit tests (fast paths + BatchSizer logic)
- feat/018 adds approximately 15 unit tests (parameter extraction + response construction)
- Total target: ~510 tests passing after both branches merge
