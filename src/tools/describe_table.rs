// src/tools/describe_table.rs
//
// describe_table tool — returns the full structural definition of a table.
//
// Parameters:
//   table  (string, required)  — table name
//   schema (string, optional)  — schema name; defaults to "public"
//
// Executes three sequential pg_catalog queries on a single connection:
//   A — columns (attname, format_type, not_null, default, comment)
//   B — constraints (name, contype, columns array, definition)
//   C — indexes (name, am type, unique, primary, definition, size_bytes)
//
// Returns a JSON object:
//   {
//     "table":       { "name", "schema", "description" },
//     "columns":     [{ "name", "type", "nullable", "default", "description" }],
//     "constraints": [{ "name", "type", "columns", "definition" }],
//     "indexes":     [{ "name", "type", "is_unique", "is_primary",
//                       "definition", "size_bytes" }]
//   }
//
// If the table does not exist Query A returns 0 rows → table_not_found error.
// SQL matches src/pg/queries/describe_table.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};
use tokio_postgres::Row;

use crate::{error::McpError, pg::infer::infer_column_description, server::context::ToolContext};

/// Map a `contype` byte (Postgres internal `"char"` type, received as `i8`)
/// to a human-readable constraint type string.
///
/// Postgres stores the constraint kind as a single ASCII character in a
/// `"char"` column. tokio-postgres decodes `"char"` columns as `i8`.
/// `unsigned_abs()` recovers the byte value without a sign-change lint.
///
/// The mapping is:
/// - `'p'` → `"primary_key"`
/// - `'u'` → `"unique"`
/// - `'f'` → `"foreign_key"`
/// - `'c'` → `"check"`
/// - `'x'` → `"exclusion"`
/// - anything else → `"other"`
fn contype_to_str(raw: i8) -> &'static str {
    let ch = char::from(raw.unsigned_abs());
    match ch {
        'p' => "primary_key",
        'u' => "unique",
        'f' => "foreign_key",
        'c' => "check",
        'x' => "exclusion",
        _ => "other",
    }
}

/// Build a column JSON object from a single `pg_attribute` row.
///
/// Expected column order:
/// 0 — `attname` (`String`)
/// 1 — `format_type` (`String`)
/// 2 — `attnotnull` (`bool`)
/// 3 — `pg_get_expr` default (`Option<String>`)
/// 4 — `col_description` (`Option<String>`)
fn build_column(row: &Row) -> Value {
    let name: String = row.get(0);
    let col_type: String = row.get(1);
    let not_null: bool = row.get(2);
    let default_value: Option<String> = row.get(3);
    let explicit_description: Option<String> = row.get(4);

    // Use explicit COMMENT when available; fall back to heuristic inference.
    let description = explicit_description.or_else(|| infer_column_description(&name, &col_type));

    serde_json::json!({
        "name":        name,
        "type":        col_type,
        "nullable":    !not_null,
        "default":     default_value,
        "description": description,
    })
}

/// Build a constraint JSON object from a single `pg_constraint` row.
///
/// Expected column order:
/// 0 — `conname` (`String`)
/// 1 — `contype` (`i8`, Postgres `"char"` internal type)
/// 2 — `array_agg` attnames (`Option<Vec<String>>`) — `None` when the
///     constraint has no column references (e.g. table-level `CHECK`)
/// 3 — `pg_get_constraintdef` (`String`)
fn build_constraint(row: &Row) -> Value {
    let name: String = row.get(0);
    let contype_raw: i8 = row.get(1);
    let col_names: Vec<String> = row.get::<_, Option<Vec<String>>>(2).unwrap_or_default();
    let definition: String = row.get(3);
    serde_json::json!({
        "name":       name,
        "type":       contype_to_str(contype_raw),
        "columns":    col_names,
        "definition": definition,
    })
}

/// Build an index JSON object from a single `pg_index` row.
///
/// Expected column order:
/// 0 — `indexrelid::regclass::text` (`String`)
/// 1 — `am.amname` (`String`)
/// 2 — `indisunique` (`bool`)
/// 3 — `indisprimary` (`bool`)
/// 4 — `pg_get_indexdef` (`String`)
/// 5 — `pg_relation_size` (`i64`)
fn build_index(row: &Row) -> Value {
    let name: String = row.get(0);
    let index_type: String = row.get(1);
    let is_unique: bool = row.get(2);
    let is_primary: bool = row.get(3);
    let definition: String = row.get(4);
    let size_bytes: i64 = row.get(5);
    serde_json::json!({
        "name":       name,
        "type":       index_type,
        "is_unique":  is_unique,
        "is_primary": is_primary,
        "definition": definition,
        "size_bytes": size_bytes,
    })
}

/// Handle a `describe_table` tool call.
///
/// Acquires a single connection, resolves the table OID via a direct
/// `pg_class`/`pg_namespace` lookup, then runs three sequential catalog
/// queries (columns, constraints, indexes) and assembles the JSON response.
///
/// # Parameters
///
/// - `table` (required): table name. Missing or empty → `param_invalid`.
/// - `schema` (optional): schema name, defaults to `"public"`.
///
/// # Errors
///
/// - [`McpError::param_invalid`] when `table` is missing or empty.
/// - [`McpError::table_not_found`] when the table does not exist in the schema.
/// - [`McpError::pg_pool_timeout`] when a connection cannot be acquired.
/// - [`McpError::pg_query_failed`] when any catalog query fails.
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    let (table, schema) = extract_params(args.as_ref())?;

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // ── Resolve table OID ─────────────────────────────────────────────────────
    //
    // Look up the table OID and table-level comment directly from pg_class +
    // pg_namespace. This avoids the regclass text-cast ambiguity in the
    // extended query protocol where `$1::regclass` is rejected when the
    // parameter is typed as TEXT.
    let oid_rows = client
        .query(
            "SELECT c.oid, obj_description(c.oid, 'pg_class') \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await
        .map_err(McpError::from)?;

    if oid_rows.is_empty() {
        return Err(McpError::table_not_found(&schema, &table));
    }

    let table_oid: u32 = oid_rows[0].get::<_, u32>(0);
    let table_description: Option<String> = oid_rows[0].get(1);

    // ── Query A — Columns ─────────────────────────────────────────────────────
    let attribute_rows = client
        .query(
            "SELECT \
                a.attname, \
                format_type(a.atttypid, a.atttypmod), \
                a.attnotnull, \
                pg_get_expr(d.adbin, d.adrelid), \
                col_description(a.attrelid, a.attnum) \
             FROM pg_attribute a \
             LEFT JOIN pg_attrdef d \
                 ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
             WHERE a.attrelid = $1 \
               AND a.attnum > 0 \
               AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[&table_oid],
        )
        .await
        .map_err(McpError::from)?;

    // 0 attribute rows → table was dropped after OID resolution (edge case).
    if attribute_rows.is_empty() {
        return Err(McpError::table_not_found(&schema, &table));
    }

    let columns: Vec<Value> = attribute_rows.iter().map(build_column).collect();

    // ── Query B — Constraints ─────────────────────────────────────────────────
    //
    // LEFT JOIN instead of INNER JOIN so that table-level CHECK constraints
    // (where `conkey` IS NULL) are not silently dropped. When no attribute
    // rows join, `array_agg … FILTER (WHERE a.attname IS NOT NULL)` returns
    // NULL rather than an array containing NULLs; `build_constraint` converts
    // that NULL to an empty `Vec<String>` via `Option::unwrap_or_default`.
    let constraint_rows = client
        .query(
            "SELECT \
                c.conname, \
                c.contype, \
                array_agg(a.attname ORDER BY array_position(c.conkey, a.attnum)) \
                    FILTER (WHERE a.attname IS NOT NULL), \
                pg_get_constraintdef(c.oid) \
             FROM pg_constraint c \
             LEFT JOIN pg_attribute a \
                 ON a.attrelid = c.conrelid \
                 AND c.conkey IS NOT NULL \
                 AND a.attnum = ANY(c.conkey) \
             WHERE c.conrelid = $1 \
             GROUP BY c.oid, c.conname, c.contype \
             ORDER BY c.contype, c.conname",
            &[&table_oid],
        )
        .await
        .map_err(McpError::from)?;

    let constraints: Vec<Value> = constraint_rows.iter().map(build_constraint).collect();

    // ── Query C — Indexes ─────────────────────────────────────────────────────
    let index_rows = client
        .query(
            "SELECT \
                ix.indexrelid::regclass::text, \
                am.amname, \
                ix.indisunique, \
                ix.indisprimary, \
                pg_get_indexdef(ix.indexrelid), \
                pg_relation_size(ix.indexrelid) \
             FROM pg_index ix \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             JOIN pg_am am ON i.relam = am.oid \
             WHERE ix.indrelid = $1 \
             ORDER BY ix.indisprimary DESC, i.relname",
            &[&table_oid],
        )
        .await
        .map_err(McpError::from)?;

    // Release the connection — all three queries done.
    drop(client);

    let indexes: Vec<Value> = index_rows.iter().map(build_index).collect();

    let body = serde_json::json!({
        "table": {
            "name":        table,
            "schema":      schema,
            "description": table_description,
        },
        "columns":     columns,
        "constraints": constraints,
        "indexes":     indexes,
    });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}

/// Extract and validate the `table` and `schema` parameters from `args`.
///
/// # Errors
///
/// Returns [`McpError::param_invalid`] when `table` is missing or empty.
fn extract_params(args: Option<&Map<String, Value>>) -> Result<(String, String), McpError> {
    let table = args
        .and_then(|m| m.get("table"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            McpError::param_invalid("table", "required string parameter is missing or empty")
        })?
        .to_string();

    let schema = args
        .and_then(|m| m.get("schema"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("public")
        .to_string();

    Ok((table, schema))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contype_primary_key() {
        assert_eq!(contype_to_str(b'p' as i8), "primary_key");
    }

    #[test]
    fn contype_unique() {
        assert_eq!(contype_to_str(b'u' as i8), "unique");
    }

    #[test]
    fn contype_foreign_key() {
        assert_eq!(contype_to_str(b'f' as i8), "foreign_key");
    }

    #[test]
    fn contype_check() {
        assert_eq!(contype_to_str(b'c' as i8), "check");
    }

    #[test]
    fn contype_exclusion() {
        assert_eq!(contype_to_str(b'x' as i8), "exclusion");
    }

    #[test]
    fn contype_unknown() {
        assert_eq!(contype_to_str(b'z' as i8), "other");
    }

    #[test]
    fn extract_params_missing_table_returns_error() {
        let result = extract_params(None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "param_invalid");
    }

    #[test]
    fn extract_params_schema_defaults_to_public() {
        let args: Map<String, Value> = serde_json::from_str(r#"{"table":"users"}"#).unwrap();
        let (table, schema) = extract_params(Some(&args)).unwrap();
        assert_eq!(table, "users");
        assert_eq!(schema, "public");
    }

    #[test]
    fn extract_params_explicit_schema() {
        let args: Map<String, Value> =
            serde_json::from_str(r#"{"table":"events","schema":"analytics"}"#).unwrap();
        let (table, schema) = extract_params(Some(&args)).unwrap();
        assert_eq!(table, "events");
        assert_eq!(schema, "analytics");
    }
}
