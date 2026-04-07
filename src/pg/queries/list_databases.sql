-- src/pg/queries/list_databases.sql
--
-- Returns all databases visible to the connected role on this Postgres instance.
-- Used by the list_databases tool.
--
-- Columns returned (in order):
--   name         TEXT    — database name
--   owner        TEXT    — name of the owning role
--   encoding     TEXT    — character encoding name (e.g. 'UTF8')
--   size_bytes   INT8    — size in bytes, NULL if pg_database_size() would error
--   description  TEXT    — comment on the database, NULL if none
--
-- Notes:
--   pg_database_size() requires CONNECT privilege on the target database.
--   For databases the role cannot connect to (e.g., template0 with datallowconn=false),
--   we use a CASE expression to avoid an error and return NULL instead.
--   template0 is included but its size is NULL because datallowconn = false.
--
--   pg_shdescription holds per-database (shared-catalog) comments.
--   The class OID for pg_database is 1262.

SELECT
    d.datname                                           AS name,
    r.rolname                                           AS owner,
    pg_encoding_to_char(d.encoding)                     AS encoding,
    CASE
        WHEN d.datallowconn THEN pg_database_size(d.oid)
        ELSE NULL
    END                                                 AS size_bytes,
    sd.description                                      AS description
FROM pg_database d
JOIN pg_roles r ON r.oid = d.datdba
LEFT JOIN pg_shdescription sd
    ON sd.objoid = d.oid
    AND sd.classoid = 'pg_database'::regclass
ORDER BY d.datname
