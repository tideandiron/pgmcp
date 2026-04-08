-- src/pg/queries/my_permissions.sql
--
-- Reference SQL for the my_permissions tool.
-- The actual tool (src/tools/my_permissions.rs) executes these inline.
-- This file documents the three queries used.

-- Query 1: Role attributes for the current session role.
SELECT
    current_user AS role_name,
    rolsuper,
    rolcreatedb,
    rolcreaterole,
    rolinherit,
    rolcanlogin,
    rolreplication,
    rolbypassrls,
    rolconnlimit
FROM pg_roles
WHERE rolname = current_user;

-- Query 2: Schema privileges visible to the current role.
SELECT
    schema_name,
    has_schema_privilege(schema_name, 'USAGE')  AS can_usage,
    has_schema_privilege(schema_name, 'CREATE') AS can_create
FROM information_schema.schemata
WHERE schema_name NOT IN ('pg_toast', 'pg_catalog', 'information_schema')
  AND schema_name NOT LIKE 'pg_temp_%'
  AND schema_name NOT LIKE 'pg_toast_temp_%'
ORDER BY schema_name;

-- Query 3 (optional): Table-level privileges for a specific table ($1 = 'schema.table').
SELECT
    has_table_privilege($1, 'SELECT')     AS can_select,
    has_table_privilege($1, 'INSERT')     AS can_insert,
    has_table_privilege($1, 'UPDATE')     AS can_update,
    has_table_privilege($1, 'DELETE')     AS can_delete,
    has_table_privilege($1, 'TRUNCATE')   AS can_truncate,
    has_table_privilege($1, 'REFERENCES') AS can_references;
