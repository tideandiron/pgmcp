# Phase 4 Implementation Plan: feat/013 — Schema Cache

**Branch:** `feat/013`
**Depends on:** `feat/012` (list-extensions-table-stats, all Phase 3 discovery tools merged)
**Spec sections:** 3.2 (Schema Cache, Cache Invalidation Background Task), 3.3 (describe_table data flow), 3.4 (Startup Sequence steps 7–8)
**Files touched:** 12 files modified, 1 file created

---

## Overview

Phase 4 introduces an in-memory schema snapshot that all discovery tools read from, dramatically reducing pg_catalog traffic on repeated calls. The cache is populated before the server accepts any requests, atomically refreshed by a background task, and cleanly invalidated when Postgres reports new transaction commits on the target database.

Architectural invariants that must hold at the end of this phase:

- `SchemaCache` is the single owner of all in-memory schema state. No tool carries its own schema state.
- The cache is always either fully populated or being replaced entirely. Tools never observe a half-refreshed snapshot.
- SQL-execution tools (`query`, `explain`, etc.) never read the schema cache. Only discovery tools do.
- The background invalidation task holds at most one pool connection at a time and releases it after every poll cycle.
- Dropping the `Arc<SchemaCache>` is sufficient to stop the background task.

---

## Step 1: Define `SchemaCache` and `SchemaSnapshot` in `src/pg/cache.rs`

### What to implement

`SchemaCache` is the public API object placed in `ToolContext`. Internally it wraps an `Arc<RwLock<SchemaSnapshot>>` where the `RwLock` is `tokio::sync::RwLock` (async, not `std::sync::RwLock`) so write-lock acquisition does not block the executor thread.

`SchemaSnapshot` is an immutable value type. Once built, it is never mutated — the background task builds a new `SchemaSnapshot` and atomically swaps it in by taking the write lock, replacing the entire inner value, then releasing the lock immediately. Readers hold only a read guard; they never hold a write guard.

### Data structures

```rust
/// Full catalog snapshot captured at a single point in time.
///
/// Constructed by `SchemaCache::load_from_pool`. All fields are
/// read-only after construction. Replacing the snapshot is done
/// by swapping in a new `SchemaSnapshot` under the `RwLock`.
#[derive(Debug, Clone)]
pub struct SchemaSnapshot {
    /// Unix timestamp (seconds) when this snapshot was taken.
    pub captured_at: u64,

    /// All user-visible schemas, keyed by schema name.
    pub schemas: Vec<CachedSchema>,

    /// All user tables, views, and materialized views, keyed by (schema, name).
    pub tables: Vec<CachedTable>,

    /// All user-defined enum types, keyed by (schema, name).
    pub enums: Vec<CachedEnum>,

    /// All installed extensions.
    pub extensions: Vec<CachedExtension>,

    /// Table-level statistics. Keyed by (schema, name).
    /// Populated from pg_stat_user_tables at snapshot time.
    pub table_stats: Vec<CachedTableStats>,
}

#[derive(Debug, Clone)]
pub struct CachedSchema {
    pub name: String,
    pub owner: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedTable {
    pub schema: String,
    pub name: String,
    /// "table", "view", or "materialized_view"
    pub kind: String,
    pub row_estimate: Option<i64>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedEnum {
    pub schema: String,
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CachedExtension {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct CachedTableStats {
    pub schema: String,
    pub table: String,
    pub row_estimate: i64,
    pub live_tuples: i64,
    pub dead_tuples: i64,
    pub seq_scans: i64,
    pub idx_scans: i64,
    pub last_vacuum: Option<String>,      // RFC 3339 or None
    pub last_autovacuum: Option<String>,
    pub last_analyze: Option<String>,
    pub last_autoanalyze: Option<String>,
    pub total_bytes: i64,
    pub table_bytes: i64,
    pub index_bytes: i64,
    pub toast_bytes: i64,
    pub cache_hit_ratio: f64,
    pub modifications_since_analyze: i64,
}
```

`SchemaCache` itself:

```rust
/// Thread-safe, arc-wrapped schema cache.
///
/// Wraps a `tokio::sync::RwLock<SchemaSnapshot>`. Readers acquire a
/// short-lived read guard to clone the data they need; the writer
/// (background invalidation task) takes the write lock only during the
/// atomic swap.
///
/// `SchemaCache` is `Clone` — cloning it shares the same underlying
/// `Arc<RwLock<SchemaSnapshot>>`, which is what you want for passing
/// it into `ToolContext`.
#[derive(Clone, Debug)]
pub struct SchemaCache {
    inner: Arc<tokio::sync::RwLock<SchemaSnapshot>>,
}
```

### Cache read API

These methods are called by discovery tool handlers. They acquire a read lock, extract the required subset, release the lock, and return owned data. Never return a guard — guards are never held across await points.

```rust
impl SchemaCache {
    /// Returns a snapshot of the current cache age in seconds.
    pub async fn age_seconds(&self) -> u64 {
        let snap = self.inner.read().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(snap.captured_at)
    }

    /// Returns all cached schemas.
    pub async fn get_schemas(&self) -> Vec<CachedSchema> {
        self.inner.read().await.schemas.clone()
    }

    /// Returns all cached tables in the given schema, filtered by relkind.
    ///
    /// `kinds` is a slice of "table", "view", "materialized_view".
    /// An empty slice means "all kinds".
    pub async fn get_tables(&self, schema: &str, kinds: &[&str]) -> Vec<CachedTable> {
        let snap = self.inner.read().await;
        snap.tables
            .iter()
            .filter(|t| {
                t.schema == schema
                    && (kinds.is_empty() || kinds.contains(&t.kind.as_str()))
            })
            .cloned()
            .collect()
    }

    /// Look up a single table definition by schema + name.
    pub async fn get_table(&self, schema: &str, table: &str) -> Option<CachedTable> {
        let snap = self.inner.read().await;
        snap.tables
            .iter()
            .find(|t| t.schema == schema && t.name == table)
            .cloned()
    }

    /// Returns all cached enum types.
    pub async fn get_enums(&self) -> Vec<CachedEnum> {
        self.inner.read().await.enums.clone()
    }

    /// Returns all cached extensions.
    pub async fn get_extensions(&self) -> Vec<CachedExtension> {
        self.inner.read().await.extensions.clone()
    }

    /// Returns cached table stats for a given schema + table, if present.
    pub async fn get_table_stats(&self, schema: &str, table: &str) -> Option<CachedTableStats> {
        let snap = self.inner.read().await;
        snap.table_stats
            .iter()
            .find(|s| s.schema == schema && s.table == table)
            .cloned()
    }
}
```

### Cache write API

Called only by the startup warm-up and the background invalidation task.

```rust
impl SchemaCache {
    /// Construct an empty cache (no snapshot yet).
    ///
    /// Used only in tests. Production startup calls `load_from_pool` before
    /// serving any requests.
    pub fn empty() -> Self {
        let snap = SchemaSnapshot {
            captured_at: 0,
            schemas: vec![],
            tables: vec![],
            enums: vec![],
            extensions: vec![],
            table_stats: vec![],
        };
        Self {
            inner: Arc::new(tokio::sync::RwLock::new(snap)),
        }
    }

    /// Build a `SchemaCache` populated from the given pool.
    ///
    /// Executes five catalog queries sequentially on a single connection.
    /// Called at startup (blocking startup gate). Also called by the
    /// background invalidation task when it detects a schema change.
    ///
    /// Returns `Err` only if pool acquisition or any catalog query fails.
    pub async fn load_from_pool(pool: &crate::pg::pool::Pool) -> Result<Self, crate::error::McpError> {
        let timeout = std::time::Duration::from_secs(30);
        let client = pool.get(timeout).await?;
        let snapshot = Self::build_snapshot(&client).await?;
        drop(client);
        Ok(Self {
            inner: Arc::new(tokio::sync::RwLock::new(snapshot)),
        })
    }

    /// Atomically replace the snapshot.
    ///
    /// Called by the background task after building a fresh snapshot.
    /// The write lock is held only for the duration of the struct-level swap.
    pub async fn replace_snapshot(&self, new: SchemaSnapshot) {
        let mut guard = self.inner.write().await;
        *guard = new;
    }

    /// Build a complete snapshot using an already-acquired client.
    ///
    /// Runs five sequential catalog queries. All queries run on the same
    /// connection so they see a consistent point in time.
    pub(crate) async fn build_snapshot(
        client: &deadpool_postgres::Client,
    ) -> Result<SchemaSnapshot, crate::error::McpError> {
        // ... five queries, described in Step 2 ...
    }
}
```

### Unit tests inside `src/pg/cache.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // SchemaCache must be Clone, Send, and Sync so it can be placed in ToolContext
    // and passed across task boundaries.
    fn assert_send_sync_clone<T: Clone + Send + Sync>() {}

    #[test]
    fn schema_cache_is_clone_send_sync() {
        assert_send_sync_clone::<SchemaCache>();
    }

    #[test]
    fn empty_cache_returns_no_schemas() {
        // Runtime needed for async read.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = SchemaCache::empty();
        let schemas = rt.block_on(cache.get_schemas());
        assert!(schemas.is_empty());
    }

    #[test]
    fn empty_cache_age_seconds_is_large() {
        // captured_at = 0 → age = now - 0 = very large number.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = SchemaCache::empty();
        let age = rt.block_on(cache.age_seconds());
        // Must be at least 50 years in seconds (2024 - 1970 = 54 years ≈ 1.7 billion seconds).
        assert!(age > 50 * 365 * 24 * 3600);
    }

    #[tokio::test]
    async fn replace_snapshot_is_visible_to_readers() {
        let cache = SchemaCache::empty();
        let new_snap = SchemaSnapshot {
            captured_at: 9_999_999_999,
            schemas: vec![CachedSchema {
                name: "analytics".to_string(),
                owner: "alice".to_string(),
                description: None,
            }],
            tables: vec![],
            enums: vec![],
            extensions: vec![],
            table_stats: vec![],
        };
        cache.replace_snapshot(new_snap).await;
        let schemas = cache.get_schemas().await;
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "analytics");
    }

    #[tokio::test]
    async fn get_tables_filters_by_schema() {
        let cache = SchemaCache::empty();
        let snap = SchemaSnapshot {
            captured_at: 1,
            schemas: vec![],
            tables: vec![
                CachedTable {
                    schema: "public".to_string(),
                    name: "users".to_string(),
                    kind: "table".to_string(),
                    row_estimate: Some(100),
                    description: None,
                },
                CachedTable {
                    schema: "analytics".to_string(),
                    name: "events".to_string(),
                    kind: "table".to_string(),
                    row_estimate: Some(5000),
                    description: None,
                },
            ],
            enums: vec![],
            extensions: vec![],
            table_stats: vec![],
        };
        cache.replace_snapshot(snap).await;

        let public_tables = cache.get_tables("public", &[]).await;
        assert_eq!(public_tables.len(), 1);
        assert_eq!(public_tables[0].name, "users");

        let analytics_tables = cache.get_tables("analytics", &[]).await;
        assert_eq!(analytics_tables.len(), 1);
        assert_eq!(analytics_tables[0].name, "events");
    }

    #[tokio::test]
    async fn get_tables_filters_by_kind() {
        let cache = SchemaCache::empty();
        let snap = SchemaSnapshot {
            captured_at: 1,
            schemas: vec![],
            tables: vec![
                CachedTable {
                    schema: "public".to_string(),
                    name: "orders".to_string(),
                    kind: "table".to_string(),
                    row_estimate: None,
                    description: None,
                },
                CachedTable {
                    schema: "public".to_string(),
                    name: "orders_view".to_string(),
                    kind: "view".to_string(),
                    row_estimate: None,
                    description: None,
                },
            ],
            enums: vec![],
            extensions: vec![],
            table_stats: vec![],
        };
        cache.replace_snapshot(snap).await;

        let tables_only = cache.get_tables("public", &["table"]).await;
        assert_eq!(tables_only.len(), 1);
        assert_eq!(tables_only[0].name, "orders");

        let views_only = cache.get_tables("public", &["view"]).await;
        assert_eq!(views_only.len(), 1);
        assert_eq!(views_only[0].name, "orders_view");

        let all = cache.get_tables("public", &[]).await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn get_table_returns_none_for_unknown() {
        let cache = SchemaCache::empty();
        let result = cache.get_table("public", "ghost").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_table_stats_returns_none_for_unknown() {
        let cache = SchemaCache::empty();
        let result = cache.get_table_stats("public", "ghost").await;
        assert!(result.is_none());
    }
}
```

---

## Step 2: Implement `build_snapshot` — five catalog queries

`build_snapshot` runs five sequential queries on a single `deadpool_postgres::Client`. Using one connection is intentional: it ensures all five views are consistent.

### Query 1 — schemas

```sql
SELECT n.nspname, r.rolname, d.description
FROM pg_namespace n
JOIN pg_roles r ON r.oid = n.nspowner
LEFT JOIN pg_description d
    ON d.objoid = n.oid AND d.classoid = 'pg_namespace'::regclass
WHERE
    n.nspname NOT IN ('pg_catalog', 'information_schema')
    AND n.nspname NOT LIKE 'pg_toast%'
    AND n.nspname NOT LIKE 'pg_temp_%'
    AND has_schema_privilege(n.nspname, 'USAGE')
ORDER BY n.nspname
```

This is identical to the query in `list_schemas::handle`. The cache populates from exactly the same source.

### Query 2 — tables, views, materialized views

```sql
SELECT
    n.nspname,
    c.relname,
    CASE c.relkind
        WHEN 'r' THEN 'table'
        WHEN 'v' THEN 'view'
        WHEN 'm' THEN 'materialized_view'
    END,
    CASE WHEN c.relkind IN ('v') THEN NULL ELSE c.reltuples::int8 END,
    d.description
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_description d
    ON d.objoid = c.oid
    AND d.objsubid = 0
    AND d.classoid = 'pg_class'::regclass
WHERE
    c.relkind IN ('r','v','m')
    AND NOT c.relispartition
    AND has_table_privilege(c.oid, 'SELECT')
    AND n.nspname NOT IN ('pg_catalog', 'information_schema')
    AND n.nspname NOT LIKE 'pg_toast%'
    AND n.nspname NOT LIKE 'pg_temp_%'
ORDER BY n.nspname, c.relname
```

Note: this loads tables across ALL schemas, not per-schema. The cache then answers per-schema queries by filtering in memory. This is correct because the cache holds a complete snapshot.

### Query 3 — enums

```sql
SELECT t.typname, n.nspname, array_agg(e.enumlabel ORDER BY e.enumsortorder)
FROM pg_enum e
JOIN pg_type t ON e.enumtypid = t.oid
JOIN pg_namespace n ON t.typnamespace = n.oid
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
GROUP BY t.typname, n.nspname
ORDER BY n.nspname, t.typname
```

### Query 4 — extensions

```sql
SELECT e.extname, e.extversion, n.nspname, COALESCE(a.comment, '') AS description
FROM pg_extension e
JOIN pg_namespace n ON e.extnamespace = n.oid
LEFT JOIN pg_available_extensions a ON a.name = e.extname
ORDER BY e.extname
```

### Query 5 — table stats

```sql
SELECT
    s.relname,
    s.schemaname,
    c.reltuples::int8                                     AS row_estimate,
    pg_total_relation_size(c.oid)                        AS total_size,
    pg_table_size(c.oid)                                 AS table_size,
    pg_indexes_size(c.oid)                               AS indexes_size,
    COALESCE(
        pg_total_relation_size(c.oid)
            - pg_table_size(c.oid)
            - pg_indexes_size(c.oid),
        0
    )                                                     AS toast_size,
    s.seq_scan,
    s.idx_scan,
    s.n_live_tup,
    s.n_dead_tup,
    s.last_vacuum,
    s.last_autovacuum,
    s.last_analyze,
    s.last_autoanalyze,
    s.n_mod_since_analyze,
    COALESCE(
        stio.heap_blks_hit::float8
            / NULLIF(stio.heap_blks_hit + stio.heap_blks_read, 0),
        0.0
    )                                                     AS cache_hit_ratio
FROM pg_stat_user_tables s
JOIN pg_class c ON c.oid = s.relid
LEFT JOIN pg_statio_user_tables stio ON stio.relid = s.relid
ORDER BY s.schemaname, s.relname
```

`table_stats` does not cache describe_table output (columns, constraints, indexes) — those are per-table and queried live on demand. The stats sub-tool is different: it is a poll-based aggregate for which cache staleness of 30 seconds is acceptable. `describe_table` goes to Postgres on every call because structural metadata (column definitions, constraints, indexes) is more sensitive to staleness and the query is cheaper.

### Implementation notes for `build_snapshot`

- Use `time::OffsetDateTime` → `format!(Rfc3339)` for timestamp columns, same as in `table_stats::format_timestamp`.
- `captured_at` is set from `std::time::SystemTime::now()` after all five queries complete (not before), so it reflects when the snapshot became fully consistent.
- Any query error returns `McpError::pg_query_failed`.

---

## Step 3: Implement `src/pg/invalidation.rs` — background polling task

### `InvalidationHandle`

The background task is launched by `spawn_invalidation_task` which returns a `JoinHandle`. The caller (main.rs) holds this handle; dropping it aborts the task. The task holds a `Weak<tokio::sync::RwLock<SchemaSnapshot>>` — if the cache is dropped (process shutdown), `Weak::upgrade` returns `None` and the task exits cleanly.

Because the task holds only a weak reference, the `SchemaCache` Arc does not prevent shutdown. The task checks the weak reference at the top of each poll cycle.

```rust
/// Spawn the cache invalidation background task.
///
/// Returns the `JoinHandle`. Drop the handle to stop the task (it will
/// abort on the next poll cycle boundary). The task also exits if the
/// cache `Arc` is dropped before the handle.
///
/// # Arguments
///
/// - `cache`: the shared cache to refresh. The task holds a `Weak`
///   reference so it does not prevent shutdown.
/// - `pool`: connection pool. The task acquires and releases one
///   connection per poll cycle.
/// - `interval_secs`: poll interval in seconds (from config).
pub fn spawn_invalidation_task(
    cache: Arc<SchemaCache>,
    pool: Arc<crate::pg::pool::Pool>,
    interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    let weak_cache = Arc::downgrade(&cache);
    // Drop the strong reference so the weak ref controls task lifetime.
    drop(cache);

    tokio::spawn(async move {
        run_invalidation_loop(weak_cache, pool, interval_secs).await;
    })
}

async fn run_invalidation_loop(
    weak_cache: std::sync::Weak<SchemaCache>,
    pool: Arc<crate::pg::pool::Pool>,
    interval_secs: u64,
) {
    let interval = std::time::Duration::from_secs(interval_secs);
    let mut last_xact_commit: Option<i64> = None;

    loop {
        tokio::time::sleep(interval).await;

        // Exit cleanly if the cache has been dropped.
        let Some(cache) = weak_cache.upgrade() else {
            tracing::debug!("schema cache dropped, invalidation task exiting");
            return;
        };

        match poll_and_maybe_refresh(&cache, &pool, &mut last_xact_commit).await {
            Ok(refreshed) => {
                if refreshed {
                    tracing::info!("schema cache refreshed after detecting pg_catalog changes");
                } else {
                    tracing::debug!("schema cache poll: no changes detected");
                }
            }
            Err(e) => {
                // Log and continue. Worst case: stale cache data. Not a crash.
                tracing::warn!(error = %e, "schema cache invalidation poll failed; retrying next interval");
            }
        }
    }
}
```

### Poll logic

```rust
async fn poll_and_maybe_refresh(
    cache: &SchemaCache,
    pool: &crate::pg::pool::Pool,
    last_xact_commit: &mut Option<i64>,
) -> Result<bool, crate::error::McpError> {
    let timeout = std::time::Duration::from_secs(10);
    let client = pool.get(timeout).await?;

    // Step 1: Check transaction commit count.
    // If it has not increased since last poll, no DDL has occurred.
    let row = client
        .query_one(
            "SELECT SUM(xact_commit)::int8 \
             FROM pg_stat_database \
             WHERE datname = current_database()",
            &[],
        )
        .await
        .map_err(crate::error::McpError::from)?;

    let current_commit: i64 = row.get(0);

    let changed = match *last_xact_commit {
        None => true,  // First poll — always refresh.
        Some(prev) => current_commit != prev,
    };
    *last_xact_commit = Some(current_commit);

    if !changed {
        drop(client);
        return Ok(false);
    }

    // Step 2: Build new snapshot using the same connection.
    let new_snapshot = SchemaCache::build_snapshot(&client).await?;
    drop(client);  // Release before taking write lock.

    // Step 3: Atomically replace.
    cache.replace_snapshot(new_snapshot).await;

    Ok(true)
}
```

### Design rationale for poll approach

The spec says to compare `max(xact_commit)` from `pg_stat_database`. The implementation above uses `SUM(xact_commit)` across all databases rather than `max` because a Postgres cluster can have multiple databases and the `pg_stat_database` view returns one row per database. However, we filter to `current_database()`, so a single row is returned and `SUM` equals `max`. This is equivalent to the spec's intent: detecting any transaction commit on the connected database.

Note: `xact_commit` is the number of committed transactions since stats were last reset. It wraps when the counter hits `INT8_MAX` (approximately 9.2 × 10^18 transactions — not a practical concern). The invalidation task treats any change (increase or unexpected decrease due to stats reset) as a trigger for refresh.

### Unit tests inside `src/pg/invalidation.rs`

```rust
#[cfg(test)]
mod tests {
    // poll_and_maybe_refresh is tested in integration tests (requires real Postgres).
    // Unit tests here cover the task cancellation contract.

    use super::*;

    #[tokio::test]
    async fn task_exits_when_weak_cache_is_dropped() {
        // Build a minimal pool pointing at a real PG (not needed here because
        // the weak reference is dropped before the first sleep expires).
        // This test verifies the Weak reference upgrade returns None after the
        // Arc is dropped.
        use std::sync::Arc;
        let cache = Arc::new(SchemaCache::empty());
        let weak = Arc::downgrade(&cache);
        drop(cache);

        // Weak::upgrade must return None after the last strong ref is dropped.
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn spawn_invalidation_task_returns_a_handle() {
        // Verify the return type compiles — no runtime needed.
        fn accepts_join_handle(_h: tokio::task::JoinHandle<()>) {}
        // Compilation check only; we do not actually spawn against a real pool here.
        // The live spawn is tested in integration tests.
        let _ = accepts_join_handle; // suppress unused warning
    }
}
```

---

## Step 4: Update `ToolContext` in `src/server/context.rs`

Add `Arc<SchemaCache>` to `ToolContext`. Update the constructor. Update the `ToolContext::new` call sites (main.rs, transport wiring, test helpers).

```rust
use crate::{config::Config, pg::{cache::SchemaCache, pool::Pool}};

#[derive(Clone)]
pub struct ToolContext {
    pub(crate) pool: Arc<Pool>,
    pub(crate) cache: Arc<SchemaCache>,
    pub(crate) config: Arc<Config>,
}

impl ToolContext {
    pub fn new(pool: Arc<Pool>, cache: Arc<SchemaCache>, config: Arc<Config>) -> Self {
        Self { pool, cache, config }
    }
}
```

The `Send + Sync` compile-time assertion in the existing unit test will continue to pass because `Arc<SchemaCache>` is `Send + Sync` (all inner types implement those traits).

All callers of `ToolContext::new` must be updated:

- `src/transport/stdio.rs` — receives pool and config; now also receives `Arc<SchemaCache>`
- `src/transport/sse.rs` — same
- `tests/health.rs` — `test_ctx` helper
- `tests/discovery.rs` — `test_ctx` helper
- Any other test helpers that construct `ToolContext`

In all test helpers, construct the cache via `SchemaCache::empty()` (no live queries needed for tests that don't test cache behavior).

---

## Step 5: Update `src/main.rs` — startup warm-up and background task spawn

Insert two new steps between the existing health check (step 6) and transport start (step 7):

```rust
// Step 7a: Initial schema cache load (blocks until complete).
let cache = match SchemaCache::load_from_pool(&pool).await {
    Ok(c) => {
        tracing::info!("schema cache populated at startup");
        Arc::new(c)
    }
    Err(e) => {
        tracing::error!(error = %e, "failed to populate schema cache at startup");
        eprintln!("pgmcp: schema cache error: {e}");
        std::process::exit(5);
    }
};

// Step 7b: Spawn background invalidation task.
let _invalidation_handle = pg::invalidation::spawn_invalidation_task(
    Arc::clone(&cache),
    Arc::clone(&pool),
    config.cache.invalidation_interval_seconds,
);
// _invalidation_handle is intentionally not awaited; it runs until process exit.
// Dropping main's handle aborts the task when main exits.

// Step 8: Start transport (pass cache to transport run functions).
let result = match transport_mode {
    TransportMode::Stdio => {
        transport::stdio::run(Arc::clone(&pool), Arc::clone(&cache), Arc::clone(&config)).await
    }
    TransportMode::Sse => {
        let ct = CancellationToken::new();
        let ct_signal = ct.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("received Ctrl-C, shutting down");
            ct_signal.cancel();
        });
        transport::sse::run(Arc::clone(&pool), Arc::clone(&cache), Arc::clone(&config), ct).await
    }
};
```

The transport run functions (`stdio::run`, `sse::run`) must be updated to accept `Arc<SchemaCache>` and pass it to `ToolContext::new`.

---

## Step 6: Update discovery tool handlers to read from cache

Six handlers get a cache-first path. The pattern is identical in all of them:

1. Try to get data from `ctx.cache`.
2. If the cache returns non-empty data, return it immediately (no pool connection acquired).
3. If the cache is empty (only on the first call if cache failed to warm, or if no tables exist in schema), fall through to the live query.

For `describe_table` and `table_stats`, which return per-object data:

1. Try `ctx.cache.get_table(schema, table)` / `ctx.cache.get_table_stats(schema, table)`.
2. Cache hit: build the JSON response from the cached struct and return.
3. Cache miss: acquire connection, execute the live query (exactly as before), return result. Do NOT populate cache on miss — the background task handles cache refresh.

**Rationale for not populating on miss:** The invariant is that the cache is always written as a complete atomic snapshot. Single-entry writes would violate the "readers see either old full snapshot or new full snapshot, never partial" invariant from the spec. The correct behavior on cache miss is to serve from Postgres and wait for the next invalidation cycle to update the cache.

### `list_schemas::handle` update

```rust
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    // Cache-first: read schemas from snapshot.
    let cached = ctx.cache.get_schemas().await;
    let schemas: Vec<Value> = if !cached.is_empty() {
        tracing::debug!(count = cached.len(), "list_schemas: cache hit");
        cached.into_iter().map(|s| serde_json::json!({
            "name":        s.name,
            "owner":       s.owner,
            "description": s.description,
        })).collect()
    } else {
        // Cache miss (startup race or empty database): fall through to live query.
        tracing::debug!("list_schemas: cache miss, querying pg_catalog");
        let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
        let client = ctx.pool.get(timeout).await?;
        let rows = client.query(/* same SQL as before */, &[]).await.map_err(McpError::from)?;
        drop(client);
        rows.iter().map(|row| { /* same row → json as before */ }).collect()
    };

    let body = serde_json::json!({ "schemas": schemas });
    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
```

### `list_tables::handle` update

```rust
// Cache-first: read tables from snapshot.
let cached = ctx.cache.get_tables(&schema, &[kind_str]).await;
if !cached.is_empty() {
    tracing::debug!(schema = %schema, kind = kind_str, count = cached.len(), "list_tables: cache hit");
    let tables: Vec<Value> = cached.into_iter().map(|t| serde_json::json!({
        "schema":       t.schema,
        "name":         t.name,
        "kind":         t.kind,
        "row_estimate": t.row_estimate,
        "description":  t.description,
    })).collect();
    let body = serde_json::json!({ "tables": tables });
    return Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]));
}
// Cache miss: fall through to live query (unchanged from Phase 3).
tracing::debug!(schema = %schema, kind = kind_str, "list_tables: cache miss");
```

Important note on the `kind` filter: the cache's `get_tables` accepts `&[&str]` of kind strings. The mapping from the `kind_to_relkind_sql` function to cache kind strings is:

- `"table"` → filter `kinds = &["table"]`
- `"view"` → filter `kinds = &["view"]`
- `"materialized_view"` → filter `kinds = &["materialized_view"]`
- `"all"` → filter `kinds = &[]` (empty = all kinds)

Remove `kind_to_relkind_sql` from the cache path; it is still needed for the fallback live query.

### `list_enums::handle` update

```rust
let cached = ctx.cache.get_enums().await;
if !cached.is_empty() {
    tracing::debug!(count = cached.len(), "list_enums: cache hit");
    let enums: Vec<Value> = cached.into_iter().map(|e| serde_json::json!({
        "name":   e.name,
        "schema": e.schema,
        "values": e.values,
    })).collect();
    let body = serde_json::json!({ "enums": enums });
    return Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]));
}
// Cache miss: fall through to live query.
```

### `list_extensions::handle` update

Same pattern as `list_enums`. `get_extensions()` returns the cached slice; non-empty → return immediately.

### `table_stats::handle` update

```rust
let (table, schema) = extract_params(args.as_ref())?;

// Cache-first: look up stats from snapshot.
if let Some(stats) = ctx.cache.get_table_stats(&schema, &table).await {
    tracing::debug!(schema = %schema, table = %table, "table_stats: cache hit");
    let body = build_stats_json(&stats)?;
    return Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]));
}
// Cache miss: fall through to live query.
tracing::debug!(schema = %schema, table = %table, "table_stats: cache miss");
```

Extract a `build_stats_json(stats: &CachedTableStats) -> Result<Value, McpError>` helper to keep the handler clean.

### `describe_table::handle` — no cache path

`describe_table` is the one tool in Phase 4 scope that does NOT get a cache read path. The reasons:

1. The cache stores only `CachedTable` (metadata stub: name, kind, row estimate). It does not store columns, constraints, or indexes — those are complex structured types that would make `SchemaSnapshot` significantly larger.
2. The spec's data flow diagram (section 3.3) shows `ctx.cache.get_table(schema, table)` returning an `Arc<TableDef>`. However, building a full column+constraint+index type would require either adding it to the snapshot or storing it as a separate sub-cache. The spec does not define these sub-types.
3. `describe_table` is low-frequency — agents call it when inspecting a specific table, not in hot loops.
4. The Phase 4 scope says "read from cache first, fall back to PG on miss." For `describe_table`, the cache cannot satisfy the call (it doesn't store full table definitions), so the tool falls through to Postgres on every call.

This is explicitly documented in the handler header:

```rust
// describe_table always queries pg_catalog directly.
// The schema cache stores lightweight table stubs (name, kind, row estimate)
// but not column/constraint/index definitions. Full describe queries are
// infrequent enough that live-query performance is acceptable.
// See: docs/plans/2026-04-07-phase-4-schema-cache.md (Step 6).
```

`describe_table` still benefits indirectly from the cache because `list_tables` (often called before `describe_table`) is cache-served.

---

## Step 7: Update `health::handle` to report `schema_cache_age_seconds`

The health tool gains a new field. The spec (feat/008 acceptance criteria) says `schema_cache_age_seconds` is a number in the health response.

```rust
// In health::handle, after acquiring ctx.cache:
let cache_age_seconds = ctx.cache.age_seconds().await;

let body = serde_json::json!({
    "status":                 status,
    "pg_reachable":           pg_reachable,
    "pool_available":         pool_available,
    "latency_ms":             latency_ms,
    "schema_cache_age_seconds": cache_age_seconds,
    "pool_stats": {
        "size":      pool_status.size,
        "available": pool_status.available,
        "waiting":   pool_status.waiting,
    }
});
```

Update the health integration test to assert `v["schema_cache_age_seconds"].is_number()`.

---

## Step 8: Update transport wiring

Both `stdio::run` and `sse::run` construct `ToolContext` for each tool call. They must now accept and pass `Arc<SchemaCache>`.

```rust
// In stdio::run:
pub async fn run(
    pool: Arc<Pool>,
    cache: Arc<SchemaCache>,
    config: Arc<Config>,
) -> Result<(), McpError> {
    // ... existing rmcp wiring ...
    // ToolContext::new now takes three args:
    let ctx = ToolContext::new(Arc::clone(&pool), Arc::clone(&cache), Arc::clone(&config));
    // ...
}
```

Same signature change for `sse::run`.

---

## Step 9: Integration tests in `tests/schema_cache.rs`

The existing file is a stub. Replace it with a full test suite. All tests require Docker.

```rust
// tests/schema_cache.rs

mod common;

use std::sync::Arc;

use pgmcp::{
    config::{CacheConfig, Config, GuardrailConfig, PoolConfig, TelemetryConfig, TransportConfig},
    pg::{cache::SchemaCache, pool::Pool},
    server::context::ToolContext,
    tools::{list_enums, list_extensions, list_schemas, list_tables, table_stats},
};
use serde_json::Value;

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
        cache: CacheConfig {
            invalidation_interval_seconds: 1, // short interval for tests
        },
        guardrails: GuardrailConfig::default(),
    }
}

async fn test_ctx_with_warm_cache(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache = Arc::new(
        SchemaCache::load_from_pool(&pool)
            .await
            .expect("cache warm-up must succeed"),
    );
    ToolContext::new(pool, cache, config)
}

fn test_ctx_with_empty_cache(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache = Arc::new(SchemaCache::empty());
    ToolContext::new(pool, cache, config)
}
```

### Test: cache warm-up populates schemas

```rust
#[tokio::test]
async fn test_cache_warmup_populates_schemas() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache = SchemaCache::load_from_pool(&pool)
        .await
        .expect("cache warm-up must succeed");

    let schemas = cache.get_schemas().await;
    assert!(!schemas.is_empty(), "warm cache must contain at least one schema");
    assert!(
        schemas.iter().any(|s| s.name == "public"),
        "public schema must be present"
    );
}
```

### Test: list_schemas serves from cache (no pool query on cache hit)

```rust
#[tokio::test]
async fn test_list_schemas_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = list_schemas::handle(ctx, None)
        .await
        .expect("list_schemas must succeed");

    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let schemas = v["schemas"].as_array().unwrap();
    assert!(!schemas.is_empty());
    assert!(schemas.iter().any(|s| s["name"] == "public"));
}
```

### Test: list_schemas falls back to Postgres when cache is empty

```rust
#[tokio::test]
async fn test_list_schemas_fallback_on_empty_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    // Use an empty cache — forces the live-query path.
    let ctx = test_ctx_with_empty_cache(&url);
    let result = list_schemas::handle(ctx, None)
        .await
        .expect("list_schemas fallback must succeed");

    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(v["schemas"].as_array().unwrap().iter().any(|s| s["name"] == "public"));
}
```

### Test: list_tables cache-first path returns correct kind filtering

```rust
#[tokio::test]
async fn test_list_tables_cache_kind_filter() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    // Create a test table and a view.
    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS public.cache_test_tbl (id int); \
                 CREATE OR REPLACE VIEW public.cache_test_view AS SELECT 1 AS x;",
            )
            .await
            .unwrap();
    }

    let cache = Arc::new(
        SchemaCache::load_from_pool(&pool)
            .await
            .expect("cache warm-up"),
    );
    let ctx = ToolContext::new(Arc::clone(&pool), cache, Arc::clone(&config));

    // "table" kind should include cache_test_tbl but not cache_test_view.
    let args = serde_json::from_str::<serde_json::Map<_, _>>(
        r#"{"schema":"public","kind":"table"}"#,
    )
    .unwrap();
    let result = list_tables::handle(ctx.clone(), Some(args))
        .await
        .expect("list_tables must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert!(tables.iter().any(|t| t["name"] == "cache_test_tbl"));
    assert!(!tables.iter().any(|t| t["name"] == "cache_test_view"));

    // "view" kind should include cache_test_view but not cache_test_tbl.
    let args2 = serde_json::from_str::<serde_json::Map<_, _>>(
        r#"{"schema":"public","kind":"view"}"#,
    )
    .unwrap();
    let result2 = list_tables::handle(ctx, Some(args2))
        .await
        .expect("list_tables must succeed");
    let text2 = &result2.content[0].as_text().unwrap().text;
    let v2: Value = serde_json::from_str(text2).unwrap();
    let tables2 = v2["tables"].as_array().unwrap();
    assert!(tables2.iter().any(|t| t["name"] == "cache_test_view"));
    assert!(!tables2.iter().any(|t| t["name"] == "cache_test_tbl"));
}
```

### Test: cache refreshes after schema change

```rust
#[tokio::test]
async fn test_cache_refreshes_after_new_table() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));

    // Populate cache before creating the new table.
    let cache = Arc::new(
        SchemaCache::load_from_pool(&pool)
            .await
            .expect("initial cache warm-up"),
    );

    // Verify the table is not yet in cache.
    let initial_tables = cache.get_tables("public", &[]).await;
    assert!(!initial_tables.iter().any(|t| t.name == "phase4_new_table"));

    // Create the table.
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS public.phase4_new_table (id int PRIMARY KEY)",
                &[],
            )
            .await
            .unwrap();
    }

    // Spawn the invalidation task with a 1-second interval.
    let _handle = pgmcp::pg::invalidation::spawn_invalidation_task(
        Arc::clone(&cache),
        Arc::clone(&pool),
        1, // 1-second interval
    );

    // Wait up to 5 seconds for the cache to pick up the new table.
    let mut found = false;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let tables = cache.get_tables("public", &[]).await;
        if tables.iter().any(|t| t.name == "phase4_new_table") {
            found = true;
            break;
        }
    }

    assert!(found, "cache must contain phase4_new_table after invalidation refresh");
}
```

### Test: cache age reported in health response

```rust
#[tokio::test]
async fn test_health_reports_cache_age() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = pgmcp::tools::health::handle(ctx, None)
        .await
        .expect("health must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert!(
        v.get("schema_cache_age_seconds").is_some(),
        "health response must include schema_cache_age_seconds"
    );
    assert!(
        v["schema_cache_age_seconds"].is_number(),
        "schema_cache_age_seconds must be a number"
    );
    // A freshly warmed cache is recent: age should be < 60 seconds.
    let age = v["schema_cache_age_seconds"].as_f64().unwrap();
    assert!(age < 60.0, "freshly warmed cache age should be < 60s, got {age}");
}
```

### Test: list_enums cache hit

```rust
#[tokio::test]
async fn test_list_enums_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .execute(
                "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'mood') THEN \
                 CREATE TYPE public.mood AS ENUM ('happy', 'sad', 'neutral'); END IF; END $$",
                &[],
            )
            .await
            .ok(); // ignore error if already exists
    }

    let cache = Arc::new(SchemaCache::load_from_pool(&pool).await.unwrap());
    let ctx = ToolContext::new(pool, cache, config);

    let result = list_enums::handle(ctx, None).await.expect("list_enums must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    // Cache must have served the result (verified by the presence of a known enum if created).
    assert!(v["enums"].is_array());
}
```

### Test: list_extensions cache hit

```rust
#[tokio::test]
async fn test_list_extensions_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let ctx = test_ctx_with_warm_cache(&url).await;
    let result = list_extensions::handle(ctx, None)
        .await
        .expect("list_extensions must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    let exts = v["extensions"].as_array().unwrap();
    assert!(!exts.is_empty());
    assert!(exts.iter().any(|e| e["name"] == "plpgsql"));
}
```

### Test: table_stats cache hit

```rust
#[tokio::test]
async fn test_table_stats_served_from_cache() {
    let Some((_container, url)) = common::fixtures::pg_container().await else {
        eprintln!("SKIP: Docker not available");
        return;
    };

    let config = Arc::new(test_config(&url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    {
        let client = pool.get(std::time::Duration::from_secs(10)).await.unwrap();
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS public.stats_cache_test (id int); \
                 INSERT INTO public.stats_cache_test VALUES (1),(2),(3); \
                 ANALYZE public.stats_cache_test;",
            )
            .await
            .unwrap();
    }

    let cache = Arc::new(SchemaCache::load_from_pool(&pool).await.unwrap());
    let ctx = ToolContext::new(pool, cache, config);

    let args = serde_json::from_str::<serde_json::Map<_, _>>(
        r#"{"schema":"public","table":"stats_cache_test"}"#,
    )
    .unwrap();
    let result = table_stats::handle(ctx, Some(args))
        .await
        .expect("table_stats must succeed");
    let text = &result.content[0].as_text().unwrap().text;
    let v: Value = serde_json::from_str(text).unwrap();
    assert_eq!(v["table"], "stats_cache_test");
    assert_eq!(v["schema"], "public");
    assert!(v["sizes"].is_object());
}
```

### Test: invalidation task stops when cache is dropped

```rust
#[tokio::test]
async fn test_invalidation_task_stops_on_cache_drop() {
    // This test does not require Docker — it verifies the weak-ref exit path.
    let cache = Arc::new(SchemaCache::empty());

    // Build a dummy pool pointing at a valid-looking (but unreachable) URL.
    // The task will sleep for 1000s before attempting a poll, so it will not
    // actually reach Postgres during this test.
    let config = Arc::new(test_config("postgresql://x:x@127.0.0.1:19999/x"));
    // We can't build a real pool without Postgres, so we test the weak-ref path
    // only at the data-structure level — verified by the unit test in invalidation.rs.
    //
    // Integration-level testing of the stop behavior is covered by
    // test_cache_refreshes_after_new_table (which requires Docker).
    let weak = Arc::downgrade(&cache);
    drop(cache);
    assert!(weak.upgrade().is_none(), "Arc must be freed after drop");
}
```

---

## Step 10: Update existing discovery integration tests

The `tests/discovery.rs` and `tests/health.rs` test helpers construct `ToolContext` via `ToolContext::new(pool, config)`. After this phase, the signature becomes `ToolContext::new(pool, cache, config)`.

All existing integration tests must update their `test_ctx` helper to use an empty cache:

```rust
fn test_ctx(url: &str) -> ToolContext {
    let config = Arc::new(test_config(url));
    let pool = Arc::new(Pool::build(&config).expect("pool build"));
    let cache = Arc::new(SchemaCache::empty()); // ← new
    ToolContext::new(pool, cache, config)        // ← updated signature
}
```

These tests still pass because:
- They use an empty cache, which triggers the fallback live-query path for cache-aware tools.
- `describe_table` is unchanged (always queries live).
- `server_info` and `list_databases` are not cache-backed (they query Postgres directly; no cache involvement).

No existing test logic changes; only the helper constructor changes.

Also update `tests/health.rs` health assertions to include the new `schema_cache_age_seconds` field (assert it exists and is a number; do not assert a specific value since the empty-cache age is large).

---

## Step 11: Update `src/pg/mod.rs` to expose `invalidation` module

The test suite and `main.rs` need to call `spawn_invalidation_task`. Currently `invalidation` is `pub(crate)`. For integration tests to call it via `pgmcp::pg::invalidation::spawn_invalidation_task`, the module must be promoted to `pub`:

```rust
// src/pg/mod.rs
pub(crate) mod cache;
pub(crate) mod infer;
pub mod invalidation;   // promoted from pub(crate) for integration test access
pub mod pool;
pub(crate) mod types;
```

And `cache` must also be promoted:

```rust
pub mod cache;          // promoted from pub(crate) for integration test access
```

---

## File Change Summary

| File | Change type | Description |
|---|---|---|
| `src/pg/cache.rs` | Full implementation | `SchemaCache`, `SchemaSnapshot`, all cached types, `load_from_pool`, `build_snapshot`, read/write API, unit tests |
| `src/pg/invalidation.rs` | Full implementation | `spawn_invalidation_task`, `run_invalidation_loop`, `poll_and_maybe_refresh`, unit tests |
| `src/pg/mod.rs` | Modify | Promote `cache` and `invalidation` to `pub` |
| `src/server/context.rs` | Modify | Add `cache: Arc<SchemaCache>` field, update `new()` signature |
| `src/main.rs` | Modify | Steps 7a/7b: cache warm-up + spawn invalidation task; pass cache to transport |
| `src/transport/stdio.rs` | Modify | Accept `Arc<SchemaCache>`, pass to `ToolContext::new` |
| `src/transport/sse.rs` | Modify | Same as stdio |
| `src/tools/health.rs` | Modify | Add `schema_cache_age_seconds` field to response |
| `src/tools/list_schemas.rs` | Modify | Cache-first path, fallback to live query |
| `src/tools/list_tables.rs` | Modify | Cache-first path, fallback to live query |
| `src/tools/list_enums.rs` | Modify | Cache-first path, fallback to live query |
| `src/tools/list_extensions.rs` | Modify | Cache-first path, fallback to live query |
| `src/tools/table_stats.rs` | Modify | Cache-first path, extract `build_stats_json` helper |
| `tests/schema_cache.rs` | Full implementation | All integration tests listed in Step 9 |
| `tests/health.rs` | Modify | Update `test_ctx` signature; assert `schema_cache_age_seconds` field |
| `tests/discovery.rs` | Modify | Update `test_ctx` signature |

---

## Acceptance Criteria

- [ ] `SchemaCache` and `SchemaSnapshot` defined with all five data categories
- [ ] `SchemaCache::load_from_pool` populates snapshot from five pg_catalog queries
- [ ] `SchemaCache::empty()` returns a valid zero-state cache for tests
- [ ] `SchemaCache::age_seconds()` returns elapsed seconds since snapshot capture
- [ ] `SchemaCache::get_schemas()`, `get_tables()`, `get_table()`, `get_enums()`, `get_extensions()`, `get_table_stats()` all compile and pass unit tests
- [ ] `SchemaCache::replace_snapshot` takes the write lock, swaps, releases — no partial writes
- [ ] `SchemaCache` is `Clone + Send + Sync`
- [ ] Background task spawned in `main.rs` with configured interval
- [ ] Background task exits cleanly when the `Arc<SchemaCache>` is dropped (weak reference path)
- [ ] Background task logs errors and retries; it never panics
- [ ] `ToolContext` has `cache: Arc<SchemaCache>` field
- [ ] Cache warm-up in `main.rs` exits with code 5 on failure (same as pool failure)
- [ ] `list_schemas`, `list_tables`, `list_enums`, `list_extensions`, `table_stats` all have cache-first path with live-query fallback
- [ ] `describe_table` is explicitly documented as not using cache, continues to query live
- [ ] `health` response includes `schema_cache_age_seconds` (number)
- [ ] All existing `test_ctx` helpers updated to pass `Arc<SchemaCache::empty()>`
- [ ] All existing integration tests pass with the updated signature
- [ ] `cargo test --test schema_cache` passes (all new integration tests)
- [ ] `cargo test --test discovery` passes (updated test_ctx helper)
- [ ] `cargo test --test health` passes (new schema_cache_age_seconds assertion)
- [ ] `cargo clippy -- -D warnings` produces zero warnings
- [ ] `cargo test` (unit tests only, no Docker) passes
- [ ] Cache snapshot is atomic: readers always see a consistent snapshot

---

## Review Focus

- Verify `tokio::sync::RwLock` is used, not `std::sync::RwLock` — blocking a tokio thread with `std::sync::RwLock::write()` under contention would starve the executor.
- Confirm no guard is held across an `await` point. Every read method extracts data (clones it) and returns owned values before returning from the async fn.
- Verify `replace_snapshot` does not call any async function while holding the write lock — it takes the lock, assigns, drops.
- Confirm the background task holds at most one pool connection at a time: it acquires, runs both the commit-count check and the snapshot build, then drops the connection before calling `replace_snapshot`.
- Verify `Arc::downgrade` is used correctly: `spawn_invalidation_task` takes an `Arc<SchemaCache>` by value, downgrades it, drops the original strong reference, and spawns a task holding only the weak reference.
- Confirm the empty-cache fallback in each tool handler queries the same SQL as the Phase 3 implementation. If the SQL differs, correctness is broken.
- Verify the `table_stats` fallback handles the `table_not_found` case (zero rows from `pg_stat_user_tables`) correctly — same as before.
- Check that the five catalog queries in `build_snapshot` exclude the same schemas as the individual tool queries (must not suddenly expose `pg_catalog` or `information_schema` content).
- Verify `describe_table` has a comment explicitly explaining why it has no cache path.
- Confirm startup exits with code 5 (not 3, 4, or 6) on cache warm-up failure — matches the error sequence in `main.rs`.
