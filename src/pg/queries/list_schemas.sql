-- src/pg/queries/list_schemas.sql
--
-- Returns all schemas in the current database visible to the connected role,
-- excluding internal Postgres schemas.
-- Used by the list_schemas tool.
--
-- Columns returned (in order):
--   name         TEXT    — schema name
--   owner        TEXT    — owning role name
--   description  TEXT    — comment on the schema, NULL if none
--
-- Exclusions:
--   pg_catalog        — Postgres system catalog (internal)
--   information_schema — SQL standard information schema (internal)
--   pg_toast          — internal TOAST storage
--   pg_temp_*         — per-session temporary schema
--   pg_toast_temp_*   — temporary TOAST schemas
--
-- has_schema_privilege() filters to schemas the connected role can USAGE.
-- This prevents listing schemas the role has no access to, which mirrors
-- what pg_tables and information_schema.schemata would show.

SELECT
    n.nspname                                               AS name,
    r.rolname                                               AS owner,
    d.description                                           AS description
FROM pg_namespace n
JOIN pg_roles r ON r.oid = n.nspowner
LEFT JOIN pg_description d
    ON d.objoid = n.oid
    AND d.classoid = 'pg_namespace'::regclass
WHERE
    n.nspname NOT IN ('pg_catalog', 'information_schema')
    AND n.nspname NOT LIKE 'pg_toast%'
    AND n.nspname NOT LIKE 'pg_temp_%'
    AND has_schema_privilege(n.nspname, 'USAGE')
ORDER BY n.nspname
