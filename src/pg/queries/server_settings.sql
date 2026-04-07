-- src/pg/queries/server_settings.sql
--
-- Returns connection metadata for the current session.
-- Used by the connection_info tool.
--
-- Columns returned (in order):
--   current_user  TEXT    — the role name connected to Postgres
--   current_db    TEXT    — name of the current database
--   server_host   TEXT    — listen_addresses setting (falls back to 'localhost')
--   server_port   INT4    — port from current_setting, or 5432 as fallback
--   server_version TEXT   — full version string from version()
--   ssl_active    BOOL    — whether the current connection uses SSL (always false for local socket)
--
-- Notes:
--   pg_stat_ssl.ssl is only populated when pg_stat_ssl is available (requires
--   pg_stat_ssl to be enabled, which is the default since PG 9.2). If the view
--   is not populated for this backend, ssl_active returns false.

SELECT
    current_user                                           AS current_user,
    current_database()                                     AS current_db,
    COALESCE(
        current_setting('listen_addresses', true),
        'localhost'
    )                                                      AS server_host,
    COALESCE(
        current_setting('port', true)::int4,
        5432
    )                                                      AS server_port,
    version()                                              AS server_version,
    COALESCE(
        (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
        false
    )                                                      AS ssl_active

-- ── server_info settings query ────────────────────────────────────────────────
--
-- Returns key Postgres server settings and version information.
-- Used by the server_info tool.
--
-- Columns returned (in order):
--   version_string  TEXT   — output of version(), e.g. "PostgreSQL 16.2 on x86_64-..."
--   version_num     INT4   — server_version_num as integer, e.g. 160002
--   current_role    TEXT   — current_user
--   statement_timeout TEXT — GUC value for statement_timeout (ms as string)
--   max_connections TEXT   — GUC value for max_connections
--   work_mem        TEXT   — GUC value for work_mem
--   shared_buffers  TEXT   — GUC value for shared_buffers
--
-- All settings are returned as TEXT strings exactly as Postgres stores them
-- (e.g., "5000" for 5000ms, "128MB" for 128 megabytes). The tool does not
-- reformat these values.

-- SELECT
--     version()                                               AS version_string,
--     current_setting('server_version_num')::int4             AS version_num,
--     current_user                                            AS current_role,
--     current_setting('statement_timeout')                    AS statement_timeout,
--     current_setting('max_connections')                      AS max_connections,
--     current_setting('work_mem')                             AS work_mem,
--     current_setting('shared_buffers')                       AS shared_buffers
