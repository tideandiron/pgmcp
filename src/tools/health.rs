// src/tools/health.rs
//
// health tool — executes SELECT 1 and reports pool + connectivity status.
//
// Returns a JSON object:
//   {
//     "status":         "ok" | "unhealthy",
//     "pg_reachable":   bool,
//     "pool_available": bool,
//     "latency_ms":     f64,
//     "pool_stats": {
//       "size":      usize,
//       "available": usize,
//       "waiting":   usize,
//     }
//   }
//
// Latency is measured end-to-end: start → pool acquire → SELECT 1 → row received.

use std::time::{Duration, Instant};

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

/// Handle a `health` tool call.
///
/// Acquires a connection from the pool, runs `SELECT 1`, and measures the
/// end-to-end round-trip latency. Pool statistics are captured after the
/// connection is released.
///
/// # Errors
///
/// This function is infallible from the MCP perspective — connectivity
/// failures are reported as `status: "unhealthy"` in the JSON response
/// rather than as `Err`. Only internal serialization errors (which cannot
/// occur with a fixed schema) would propagate.
pub async fn handle(
    ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    let start = Instant::now();
    let timeout = Duration::from_secs(ctx.config.pool.acquire_timeout_seconds);

    // Attempt pool acquire + SELECT 1.
    // A failure here is reported as unhealthy rather than propagated as an error.
    let pg_reachable = match ctx.pool.get(timeout).await {
        Ok(client) => client.query_one("SELECT 1::int4", &[]).await.is_ok(),
        Err(_) => false,
    };

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    // Round to one decimal place for stable output.
    let latency_ms = (latency_ms * 10.0).round() / 10.0;

    // Capture pool stats after the connection is released.
    let pool_status = ctx.pool.inner().status();

    let pool_available = pg_reachable;
    let status = if pg_reachable { "ok" } else { "unhealthy" };

    let body = serde_json::json!({
        "status":         status,
        "pg_reachable":   pg_reachable,
        "pool_available": pool_available,
        "latency_ms":     latency_ms,
        "pool_stats": {
            "size":      pool_status.size,
            "available": pool_status.available,
            "waiting":   pool_status.waiting,
        }
    });

    Ok(CallToolResult::success(vec![Content::text(
        body.to_string(),
    )]))
}
