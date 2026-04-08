-- src/pg/queries/list_enums.sql
--
-- Returns all user-defined enum types with their ordered label values.
-- Used by the list_enums tool.
--
-- Excludes types in pg_catalog and information_schema — those are system
-- types, not user-defined enums.
--
-- No parameters; returns all visible enums across all user schemas.
--
-- Columns returned (in order):
--   name    TEXT    — enum type name (pg_type.typname)
--   schema  TEXT    — schema that contains the type (pg_namespace.nspname)
--   values  TEXT[]  — enum labels in declaration order (sorted by enumsortorder,
--                     which is float4 — ORDER BY guarantees correct ordering even
--                     when labels have been added with ALTER TYPE … ADD VALUE)

SELECT
    t.typname                                              AS name,
    n.nspname                                              AS schema,
    array_agg(e.enumlabel ORDER BY e.enumsortorder)        AS values
FROM pg_enum e
JOIN pg_type t ON e.enumtypid = t.oid
JOIN pg_namespace n ON t.typnamespace = n.oid
WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
GROUP BY t.typname, n.nspname
ORDER BY n.nspname, t.typname;
