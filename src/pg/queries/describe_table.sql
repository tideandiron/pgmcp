-- src/pg/queries/describe_table.sql
--
-- Returns full table definition in three sequential queries executed on a
-- single connection to avoid pool pressure. Used by the describe_table tool.
--
-- All three queries accept $1 as an OID (the table's pg_class.oid), obtained
-- by resolving the qualified name via a preliminary regclass cast.

-- ── Query A — Columns ─────────────────────────────────────────────────────────
--
-- Returns one row per non-dropped user column, ordered by attnum.
--
-- Parameters:
--   $1  OID   — the table OID (from $schema.$table::regclass)
--
-- Columns returned (in order):
--   name          TEXT    — column name (attname)
--   type          TEXT    — human-readable type (format_type)
--   not_null      BOOL    — true when NOT NULL is declared
--   default_value TEXT    — expression from pg_get_expr, NULL when no default
--   description   TEXT    — COMMENT ON COLUMN ..., NULL when none

SELECT
    a.attname                                               AS name,
    format_type(a.atttypid, a.atttypmod)                   AS type,
    a.attnotnull                                            AS not_null,
    pg_get_expr(d.adbin, d.adrelid)                        AS default_value,
    col_description(a.attrelid, a.attnum)                  AS description,
    a.attnum
FROM pg_attribute a
LEFT JOIN pg_attrdef d
    ON a.attrelid = d.adrelid AND a.attnum = d.adnum
WHERE
    a.attrelid = $1
    AND a.attnum > 0
    AND NOT a.attisdropped
ORDER BY a.attnum;

-- ── Query B — Constraints ─────────────────────────────────────────────────────
--
-- Returns one row per constraint that involves at least one column, grouped
-- so each constraint appears once with its involved column names as an array.
--
-- Parameters:
--   $1  OID   — the table OID
--
-- Columns returned (in order):
--   name        TEXT    — constraint name (conname)
--   contype     "char"  — 'p' primary_key, 'u' unique, 'f' foreign_key,
--                         'c' check (raw pg internal char type — cast to i8 in Rust)
--   columns     TEXT[]  — column names in key order (array_position preserves
--                         the declared column order from conkey)
--   definition  TEXT    — full constraint definition from pg_get_constraintdef

SELECT
    c.conname                                               AS name,
    c.contype                                               AS contype,
    array_agg(
        a.attname
        ORDER BY array_position(c.conkey, a.attnum)
    )                                                       AS columns,
    pg_get_constraintdef(c.oid)                             AS definition
FROM pg_constraint c
JOIN pg_attribute a
    ON a.attrelid = c.conrelid
    AND a.attnum = ANY(c.conkey)
WHERE c.conrelid = $1
GROUP BY c.oid, c.conname, c.contype
ORDER BY c.contype, c.conname;

-- ── Query C — Indexes ─────────────────────────────────────────────────────────
--
-- Returns one row per index defined on the table.
--
-- Parameters:
--   $1  OID   — the table OID
--
-- Columns returned (in order):
--   name        TEXT    — qualified index name (indexrelid::regclass::text)
--   type        TEXT    — access method name (btree, hash, gin, gist, …)
--   is_unique   BOOL    — true if the index is declared UNIQUE
--   is_primary  BOOL    — true if the index backs a PRIMARY KEY constraint
--   definition  TEXT    — full CREATE INDEX statement from pg_get_indexdef
--   size_bytes  INT8    — on-disk size in bytes from pg_relation_size

SELECT
    ix.indexrelid::regclass::text                          AS name,
    am.amname                                              AS type,
    ix.indisunique                                         AS is_unique,
    ix.indisprimary                                        AS is_primary,
    pg_get_indexdef(ix.indexrelid)                         AS definition,
    pg_relation_size(ix.indexrelid)                        AS size_bytes
FROM pg_index ix
JOIN pg_class i ON i.oid = ix.indexrelid
JOIN pg_am am ON i.relam = am.oid
WHERE ix.indrelid = $1
ORDER BY ix.indisprimary DESC, i.relname;
