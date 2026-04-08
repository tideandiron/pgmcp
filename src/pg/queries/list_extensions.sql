-- src/pg/queries/list_extensions.sql
--
-- Returns all extensions currently installed in this database.
-- Used by the list_extensions tool.
--
-- Columns returned (in order):
--   extname     TEXT    — extension name (e.g. "plpgsql", "pg_stat_statements")
--   extversion  TEXT    — installed version string
--   nspname     TEXT    — schema the extension objects live in
--   description TEXT    — human-readable comment from pg_available_extensions,
--                         or empty string when not listed there
--
-- Notes:
--   The LEFT JOIN on pg_available_extensions handles extensions that were
--   installed by other means and may not appear in that view (e.g. locally-
--   built extensions). COALESCE ensures description is always a non-null string.
--   Results are ordered alphabetically by extension name.

SELECT
    e.extname,
    e.extversion,
    n.nspname,
    COALESCE(a.comment, '') AS description
FROM pg_extension e
JOIN pg_namespace n ON e.extnamespace = n.oid
LEFT JOIN pg_available_extensions a ON a.name = e.extname
ORDER BY e.extname
