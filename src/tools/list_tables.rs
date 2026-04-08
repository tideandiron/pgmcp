// src/tools/list_tables.rs
//
// list_tables tool — returns tables, views, and materialized views in a schema.
//
// Parameters:
//   schema (string, required)  — schema name; missing or empty → param_invalid
//   kind   (string, optional)  — "table" | "view" | "materialized_view" | "all"
//                                defaults to "table" when omitted
//
// Returns a JSON object:
//   {
//     "tables": [
//       {
//         "schema":       string,
//         "name":         string,
//         "kind":         "table" | "view" | "materialized_view",
//         "row_estimate": number | null,
//         "description":  string | null
//       },
//       ...
//     ]
//   }
//
// Unknown schema → empty tables array (not an error).
// SQL matches src/pg/queries/list_tables.sql.

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

use crate::{error::McpError, server::context::ToolContext};

/// Map a user-facing kind string to a SQL `IN (...)` fragment using `"char"` literals.
///
/// `pg_class.relkind` is of type `"char"` (a single-byte internal Postgres type), not
/// `TEXT`.  We cannot pass a `Vec<String>` as a `"char"[]` bind parameter, so we
/// inline the relkind characters as SQL literals.  The values are from our own
/// controlled allowlist ('r', 'v', 'm') — there is no injection risk.
///
/// Returns the SQL fragment, e.g. `"c.relkind IN ('r')"` or
/// `"c.relkind IN ('r','v','m')"`.
///
/// # Errors
///
/// Returns [`McpError::param_invalid`] if `kind` is not a recognised value.
fn kind_to_relkind_sql(kind: &str) -> Result<&'static str, McpError> {
    match kind {
        "table" => Ok("c.relkind IN ('r')"),
        "view" => Ok("c.relkind IN ('v')"),
        "materialized_view" => Ok("c.relkind IN ('m')"),
        "all" => Ok("c.relkind IN ('r','v','m')"),
        other => Err(McpError::param_invalid(
            "kind",
            format!("must be one of 'table', 'view', 'materialized_view', 'all'; got '{other}'"),
        )),
    }
}

/// Handle a `list_tables` tool call.
///
/// Acquires a connection and queries `pg_class` joined with `pg_namespace` and
/// `pg_description`. Returns tables, views, and materialized views in the
/// requested schema, filtered by the connected role's `SELECT` privilege.
///
/// Child partition tables (`relispartition = true`) are excluded — only
/// partition parents and regular tables are returned.
///
/// # Parameters
///
/// - `schema` (required): the schema to list. Missing or empty → `param_invalid`.
/// - `kind` (optional): `"table"` (default), `"view"`, `"materialized_view"`, or `"all"`.
///
/// # Errors
///
/// - [`McpError::param_invalid`] when `schema` is missing or `kind` is unrecognised.
/// - [`McpError::pg_pool_timeout`] when a connection cannot be acquired in time.
/// - [`McpError::pg_query_failed`] when the catalog query fails.
pub async fn handle(
    ctx: ToolContext,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, McpError> {
    // schema is required — None args or missing/non-string schema → param_invalid.
    let schema = args
        .as_ref()
        .and_then(|m| m.get("schema"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            McpError::param_invalid("schema", "required string parameter is missing or empty")
        })?
        .to_string();

    let kind_str = args
        .as_ref()
        .and_then(|m| m.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("table");

    // Inline the relkind filter as SQL literals — pg_class.relkind is type "char"
    // (a single-byte internal type), not TEXT.  Bind parameters cannot be coerced
    // to "char"[] from Vec<String>, so we embed the allowlist values directly.
    // The values are from our own controlled allowlist; there is no injection risk.
    let relkind_sql = kind_to_relkind_sql(kind_str)?;

    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);
    let client = ctx.pool.get(timeout).await?;

    // Build the full query with the relkind filter inlined.
    // Only $1 (schema name) is a bind parameter.
    // SQL logic matches src/pg/queries/list_tables.sql.
    let sql = format!(
        "SELECT \
            n.nspname, \
            c.relname, \
            CASE c.relkind \
                WHEN 'r' THEN 'table' \
                WHEN 'v' THEN 'view' \
                WHEN 'm' THEN 'materialized_view' \
            END, \
            CASE WHEN c.relkind IN ('v') THEN NULL ELSE c.reltuples::int8 END, \
            d.description \
        FROM pg_class c \
        JOIN pg_namespace n ON n.oid = c.relnamespace \
        LEFT JOIN pg_description d \
            ON d.objoid = c.oid \
            AND d.objsubid = 0 \
            AND d.classoid = 'pg_class'::regclass \
        WHERE \
            n.nspname = $1 \
            AND {relkind_sql} \
            AND NOT c.relispartition \
            AND has_table_privilege(c.oid, 'SELECT') \
        ORDER BY c.relname"
    );

    let rows = client
        .query(sql.as_str(), &[&schema])
        .await
        .map_err(McpError::from)?;

    // Release the connection before constructing the response.
    drop(client);

    let tables: Vec<Value> = rows
        .iter()
        .map(|row| {
            let schema_name: String = row.get(0);
            let name: String = row.get(1);
            let kind: String = row.get(2);
            let row_estimate: Option<i64> = row.get(3);
            let description: Option<String> = row.get(4);
            serde_json::json!({
                "schema":       schema_name,
                "name":         name,
                "kind":         kind,
                "row_estimate": row_estimate,
                "description":  description,
            })
        })
        .collect();

    let body = serde_json::json!({ "tables": tables });

    Ok(CallToolResult::success(vec![Content::text(
        serde_json::to_string_pretty(&body).map_err(|e| McpError::internal(e.to_string()))?,
    )]))
}
