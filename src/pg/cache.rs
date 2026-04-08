// src/pg/cache.rs
//
// SchemaCache — in-memory snapshot of pg_catalog data.

#![allow(dead_code)]
//
// Architecture invariants:
// - SchemaCache is the single owner of all in-memory schema state.
// - The snapshot is always either fully populated or being replaced entirely.
//   Tools never observe a half-refreshed snapshot.
// - Read methods acquire a read lock, clone the required data, drop the lock,
//   and return owned data. Guards are never held across await points.
// - The background invalidation task builds a new SchemaSnapshot and atomically
//   swaps it in by taking the write lock, replacing the entire inner value,
//   then releasing the lock immediately.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::McpError;

// ─── Cached data types ────────────────────────────────────────────────────────

/// Full catalog snapshot captured at a single point in time.
///
/// Constructed by [`SchemaCache::build_snapshot`]. All fields are read-only
/// after construction. Replacing the snapshot is done by swapping in a new
/// `SchemaSnapshot` under the `RwLock`.
#[derive(Debug, Clone)]
pub struct SchemaSnapshot {
    /// Unix timestamp (seconds) when this snapshot was taken.
    pub captured_at: u64,

    /// All user-visible schemas.
    pub schemas: Vec<CachedSchema>,

    /// All user tables, views, and materialized views across all schemas.
    pub tables: Vec<CachedTable>,

    /// All user-defined enum types.
    pub enums: Vec<CachedEnum>,

    /// All installed extensions.
    pub extensions: Vec<CachedExtension>,

    /// Table-level statistics. Populated from `pg_stat_user_tables` at snapshot time.
    pub table_stats: Vec<CachedTableStats>,
}

/// A cached schema entry.
#[derive(Debug, Clone)]
pub struct CachedSchema {
    pub name: String,
    pub owner: String,
    pub description: Option<String>,
}

/// A cached table, view, or materialized view entry.
#[derive(Debug, Clone)]
pub struct CachedTable {
    pub schema: String,
    pub name: String,
    /// One of `"table"`, `"view"`, or `"materialized_view"`.
    pub kind: String,
    pub row_estimate: Option<i64>,
    pub description: Option<String>,
}

/// A cached user-defined enum type.
#[derive(Debug, Clone)]
pub struct CachedEnum {
    pub schema: String,
    pub name: String,
    pub values: Vec<String>,
}

/// A cached extension entry.
#[derive(Debug, Clone)]
pub struct CachedExtension {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub description: String,
}

/// Cached table-level statistics from `pg_stat_user_tables`.
#[derive(Debug, Clone)]
pub struct CachedTableStats {
    pub schema: String,
    pub table: String,
    pub row_estimate: i64,
    pub live_tuples: i64,
    pub dead_tuples: i64,
    pub seq_scans: i64,
    pub idx_scans: i64,
    pub last_vacuum: Option<String>,
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

// ─── SchemaCache ──────────────────────────────────────────────────────────────

/// Thread-safe, arc-wrapped schema cache.
///
/// Wraps a `tokio::sync::RwLock<SchemaSnapshot>`. Readers acquire a short-lived
/// read guard to clone the data they need; the writer (background invalidation
/// task) takes the write lock only during the atomic swap.
///
/// `SchemaCache` is `Clone` — cloning it shares the same underlying
/// `Arc<RwLock<SchemaSnapshot>>`, which is what you want for passing it into
/// `ToolContext`.
#[derive(Clone, Debug)]
pub struct SchemaCache {
    inner: Arc<RwLock<SchemaSnapshot>>,
}

// ─── Read API ─────────────────────────────────────────────────────────────────

impl SchemaCache {
    /// Returns the age of the current snapshot in seconds.
    ///
    /// An empty cache (constructed with [`SchemaCache::empty`]) has
    /// `captured_at = 0`, so the age will be very large (current Unix epoch).
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

    /// Returns all cached tables in the given schema, filtered by kind.
    ///
    /// `kinds` is a slice of `"table"`, `"view"`, `"materialized_view"`.
    /// An empty `kinds` slice means "all kinds".
    pub async fn get_tables(&self, schema: &str, kinds: &[&str]) -> Vec<CachedTable> {
        let snap = self.inner.read().await;
        snap.tables
            .iter()
            .filter(|t| {
                t.schema == schema && (kinds.is_empty() || kinds.contains(&t.kind.as_str()))
            })
            .cloned()
            .collect()
    }

    /// Look up a single table entry by schema and name.
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

    /// Returns cached table statistics for a given schema and table, if present.
    pub async fn get_table_stats(&self, schema: &str, table: &str) -> Option<CachedTableStats> {
        let snap = self.inner.read().await;
        snap.table_stats
            .iter()
            .find(|s| s.schema == schema && s.table == table)
            .cloned()
    }
}

// ─── Write API ────────────────────────────────────────────────────────────────

impl SchemaCache {
    /// Construct an empty cache with no snapshot data.
    ///
    /// Used in tests and as a fallback when warm-up is not yet complete.
    /// Production startup calls [`SchemaCache::load_from_pool`] before serving
    /// any requests.
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
            inner: Arc::new(RwLock::new(snap)),
        }
    }

    /// Build a `SchemaCache` populated from the given pool.
    ///
    /// Executes five catalog queries sequentially on a single connection so
    /// all five views are consistent at the same point in time.
    ///
    /// Called at startup (blocking startup gate). Also called by the background
    /// invalidation task when it detects a schema change.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if pool acquisition or any catalog query fails.
    pub async fn load_from_pool(pool: &crate::pg::pool::Pool) -> Result<Self, McpError> {
        let timeout = std::time::Duration::from_secs(30);
        let client = pool.get(timeout).await?;
        let snapshot = Self::build_snapshot(&client).await?;
        drop(client);
        Ok(Self {
            inner: Arc::new(RwLock::new(snapshot)),
        })
    }

    /// Atomically replace the current snapshot with a new one.
    ///
    /// Called by the background invalidation task after building a fresh
    /// snapshot. The write lock is held only for the duration of the swap.
    pub async fn replace_snapshot(&self, new: SchemaSnapshot) {
        let mut guard = self.inner.write().await;
        *guard = new;
    }

    /// Build a complete [`SchemaSnapshot`] using an already-acquired client.
    ///
    /// Runs five sequential catalog queries on the same connection to ensure
    /// a consistent point-in-time view.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_query_failed`] if any catalog query fails.
    pub(crate) async fn build_snapshot(
        client: &deadpool_postgres::Client,
    ) -> Result<SchemaSnapshot, McpError> {
        // Query 1: schemas
        let schema_rows = client
            .query(
                "SELECT n.nspname, r.rolname, d.description \
                 FROM pg_namespace n \
                 JOIN pg_roles r ON r.oid = n.nspowner \
                 LEFT JOIN pg_description d \
                     ON d.objoid = n.oid AND d.classoid = 'pg_namespace'::regclass \
                 WHERE \
                     n.nspname NOT IN ('pg_catalog', 'information_schema') \
                     AND n.nspname NOT LIKE 'pg_toast%' \
                     AND n.nspname NOT LIKE 'pg_temp_%' \
                     AND has_schema_privilege(n.nspname, 'USAGE') \
                 ORDER BY n.nspname",
                &[],
            )
            .await
            .map_err(McpError::from)?;

        let schemas: Vec<CachedSchema> = schema_rows
            .iter()
            .map(|row| CachedSchema {
                name: row.get(0),
                owner: row.get(1),
                description: row.get(2),
            })
            .collect();

        // Query 2: tables, views, materialized views (all schemas at once)
        let table_rows = client
            .query(
                "SELECT \
                     n.nspname, \
                     c.relname, \
                     CASE c.relkind \
                         WHEN 'r' THEN 'table' \
                         WHEN 'v' THEN 'view' \
                         WHEN 'm' THEN 'materialized_view' \
                     END, \
                     CASE WHEN c.relkind IN ('v') THEN NULL ELSE c.reltuples::int8 END, \
                     d.description \
                 FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 LEFT JOIN pg_description d \
                     ON d.objoid = c.oid \
                     AND d.objsubid = 0 \
                     AND d.classoid = 'pg_class'::regclass \
                 WHERE \
                     c.relkind IN ('r','v','m') \
                     AND NOT c.relispartition \
                     AND has_table_privilege(c.oid, 'SELECT') \
                     AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                     AND n.nspname NOT LIKE 'pg_toast%' \
                     AND n.nspname NOT LIKE 'pg_temp_%' \
                 ORDER BY n.nspname, c.relname",
                &[],
            )
            .await
            .map_err(McpError::from)?;

        let tables: Vec<CachedTable> = table_rows
            .iter()
            .map(|row| CachedTable {
                schema: row.get(0),
                name: row.get(1),
                kind: row.get(2),
                row_estimate: row.get(3),
                description: row.get(4),
            })
            .collect();

        // Query 3: enums
        let enum_rows = client
            .query(
                "SELECT t.typname, n.nspname, \
                     array_agg(e.enumlabel ORDER BY e.enumsortorder) \
                 FROM pg_enum e \
                 JOIN pg_type t ON e.enumtypid = t.oid \
                 JOIN pg_namespace n ON t.typnamespace = n.oid \
                 WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
                 GROUP BY t.typname, n.nspname \
                 ORDER BY n.nspname, t.typname",
                &[],
            )
            .await
            .map_err(McpError::from)?;

        let enums: Vec<CachedEnum> = enum_rows
            .iter()
            .map(|row| CachedEnum {
                name: row.get(0),
                schema: row.get(1),
                values: row.get(2),
            })
            .collect();

        // Query 4: extensions
        let ext_rows = client
            .query(
                "SELECT e.extname, e.extversion, n.nspname, \
                     COALESCE(a.comment, '') AS description \
                 FROM pg_extension e \
                 JOIN pg_namespace n ON e.extnamespace = n.oid \
                 LEFT JOIN pg_available_extensions a ON a.name = e.extname \
                 ORDER BY e.extname",
                &[],
            )
            .await
            .map_err(McpError::from)?;

        let extensions: Vec<CachedExtension> = ext_rows
            .iter()
            .map(|row| CachedExtension {
                name: row.get(0),
                version: row.get(1),
                schema: row.get(2),
                description: row.get(3),
            })
            .collect();

        // Query 5: table statistics
        let stats_rows = client
            .query(
                "SELECT \
                     s.relname, \
                     s.schemaname, \
                     c.reltuples::int8                                         AS row_estimate, \
                     pg_total_relation_size(c.oid)                            AS total_size, \
                     pg_table_size(c.oid)                                     AS table_size, \
                     pg_indexes_size(c.oid)                                   AS indexes_size, \
                     COALESCE( \
                         pg_total_relation_size(c.oid) \
                             - pg_table_size(c.oid) \
                             - pg_indexes_size(c.oid), \
                         0 \
                     )                                                         AS toast_size, \
                     s.seq_scan, \
                     s.idx_scan, \
                     s.n_live_tup, \
                     s.n_dead_tup, \
                     s.last_vacuum, \
                     s.last_autovacuum, \
                     s.last_analyze, \
                     s.last_autoanalyze, \
                     s.n_mod_since_analyze, \
                     COALESCE( \
                         stio.heap_blks_hit::float8 \
                             / NULLIF(stio.heap_blks_hit + stio.heap_blks_read, 0), \
                         0.0 \
                     )                                                         AS cache_hit_ratio \
                 FROM pg_stat_user_tables s \
                 JOIN pg_class c ON c.oid = s.relid \
                 LEFT JOIN pg_statio_user_tables stio ON stio.relid = s.relid \
                 ORDER BY s.schemaname, s.relname",
                &[],
            )
            .await
            .map_err(McpError::from)?;

        let table_stats: Vec<CachedTableStats> = stats_rows
            .iter()
            .map(|row| {
                let last_vacuum: Option<time::OffsetDateTime> = row.get(11);
                let last_autovacuum: Option<time::OffsetDateTime> = row.get(12);
                let last_analyze: Option<time::OffsetDateTime> = row.get(13);
                let last_autoanalyze: Option<time::OffsetDateTime> = row.get(14);

                CachedTableStats {
                    table: row.get(0),
                    schema: row.get(1),
                    row_estimate: row.get(2),
                    total_bytes: row.get(3),
                    table_bytes: row.get(4),
                    index_bytes: row.get(5),
                    toast_bytes: row.get(6),
                    seq_scans: row.get(7),
                    idx_scans: row.get::<_, Option<i64>>(8).unwrap_or(0),
                    live_tuples: row.get(9),
                    dead_tuples: row.get(10),
                    last_vacuum: format_ts(last_vacuum),
                    last_autovacuum: format_ts(last_autovacuum),
                    last_analyze: format_ts(last_analyze),
                    last_autoanalyze: format_ts(last_autoanalyze),
                    modifications_since_analyze: row.get(15),
                    cache_hit_ratio: row.get(16),
                }
            })
            .collect();

        let captured_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(SchemaSnapshot {
            captured_at,
            schemas,
            tables,
            enums,
            extensions,
            table_stats,
        })
    }
}

/// Format an `Option<time::OffsetDateTime>` as an RFC 3339 string, or `None`.
fn format_ts(ts: Option<time::OffsetDateTime>) -> Option<String> {
    ts.and_then(|dt| {
        dt.format(&time::format_description::well_known::Rfc3339)
            .ok()
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync_clone<T: Clone + Send + Sync>() {}

    #[test]
    fn schema_cache_is_clone_send_sync() {
        assert_send_sync_clone::<SchemaCache>();
    }

    #[test]
    fn empty_cache_returns_no_schemas() {
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
