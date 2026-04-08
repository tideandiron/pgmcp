-- src/pg/queries/table_stats.sql
--
-- Returns runtime statistics for a single user table.
-- Used by the table_stats tool.
--
-- Parameters:
--   $1  TEXT  — schema name
--   $2  TEXT  — table name
--
-- Columns returned (in order):
--   relname                TEXT      — table name
--   schemaname             TEXT      — schema name
--   row_estimate           INT8      — approximate row count from pg_class.reltuples
--   total_size             INT8      — total relation size (table + indexes + toast)
--   table_size             INT8      — heap (data) size only
--   indexes_size           INT8      — sum of all index sizes
--   toast_size             INT8      — toast table size (0 when no toast)
--   seq_scan               INT8      — number of sequential scans
--   idx_scan               INT8      — number of index scans (NULL if none ever)
--   n_live_tup             INT8      — estimated live rows
--   n_dead_tup             INT8      — estimated dead rows (bloat indicator)
--   last_vacuum            TIMESTAMPTZ — last manual VACUUM, or NULL
--   last_autovacuum        TIMESTAMPTZ — last autovacuum run, or NULL
--   last_analyze           TIMESTAMPTZ — last manual ANALYZE, or NULL
--   last_autoanalyze       TIMESTAMPTZ — last autoanalyze run, or NULL
--   n_mod_since_analyze    INT8      — rows modified since last ANALYZE
--   cache_hit_ratio        FLOAT8    — fraction of heap reads served from cache
--
-- Notes:
--   Zero rows returned means the table does not exist in pg_stat_user_tables
--   (either the table doesn't exist or the connected role cannot see it).
--   pg_stat_user_tables only tracks user tables (not system/catalog tables).
--   cache_hit_ratio is 0.0 when no reads have occurred (NULLIF avoids /0).
--   toast_size is clamped to 0 via COALESCE in case sizes are inconsistent.

SELECT
    s.relname,
    s.schemaname,
    c.reltuples::int8                                        AS row_estimate,
    pg_total_relation_size(c.oid)                           AS total_size,
    pg_table_size(c.oid)                                    AS table_size,
    pg_indexes_size(c.oid)                                  AS indexes_size,
    COALESCE(
        pg_total_relation_size(c.oid)
            - pg_table_size(c.oid)
            - pg_indexes_size(c.oid),
        0
    )                                                        AS toast_size,
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
    )                                                        AS cache_hit_ratio
FROM pg_stat_user_tables s
JOIN pg_class c ON c.oid = s.relid
LEFT JOIN pg_statio_user_tables stio ON stio.relid = s.relid
WHERE s.schemaname = $1
  AND s.relname    = $2
