// src/tools/health.rs
// Stub implementation — real implementation lands in feat/008.

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    _ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(
        r#"{"status":"ok","pool_available":true,"pg_reachable":true,"schema_cache_age_seconds":0,"latency_ms":0}"#,
    )]))
}
