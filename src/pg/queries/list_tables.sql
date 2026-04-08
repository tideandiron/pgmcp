-- src/pg/queries/list_tables.sql
--
-- Returns tables, views, and materialized views in a given schema.
-- Used by the list_tables tool.
--
-- Parameters (bound positionally):
--   $1  TEXT    — schema name (exact match against nspname)
--   $2  TEXT[]  — relkind filter: 'r' (table), 'v' (view), 'm' (mat. view)
--                 Pass ARRAY['r','v','m'] for "all".
--
-- Columns returned (in order):
--   schema       TEXT    — schema name (echoes $1)
--   name         TEXT    — table/view name
--   kind         TEXT    — 'table', 'view', or 'materialized_view'
--   row_estimate INT8    — row count estimate from pg_class.reltuples; -1 means
--                          stats not yet collected; NULL for views
--   description  TEXT    — COMMENT ON TABLE, NULL if none
--
-- Note: reltuples is -1 for tables that have never been ANALYZEd, and 0
-- immediately after CREATE TABLE. Callers should treat -1 and 0 as "unknown".
--
-- has_table_privilege() filters to tables the connected role can SELECT from.
-- relispartition excludes child partition tables (only parents are returned).

SELECT
    n.nspname                                               AS schema,
    c.relname                                               AS name,
    CASE c.relkind
        WHEN 'r' THEN 'table'
        WHEN 'v' THEN 'view'
        WHEN 'm' THEN 'materialized_view'
    END                                                     AS kind,
    CASE
        WHEN c.relkind IN ('v') THEN NULL
        ELSE c.reltuples::int8
    END                                                     AS row_estimate,
    d.description                                           AS description
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_description d
    ON d.objoid = c.oid
    AND d.objsubid = 0
    AND d.classoid = 'pg_class'::regclass
WHERE
    n.nspname = $1
    AND c.relkind = ANY($2)
    AND NOT c.relispartition              -- exclude child partition tables
    AND has_table_privilege(c.oid, 'SELECT')
ORDER BY c.relname
