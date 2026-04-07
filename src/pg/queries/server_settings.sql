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
