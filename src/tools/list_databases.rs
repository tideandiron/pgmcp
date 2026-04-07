// src/tools/list_databases.rs
// Stub implementation — real implementation lands in feat/009.

use rmcp::model::{CallToolResult, Content};
use serde_json::Map;

use crate::{error::McpError, server::context::ToolContext};

pub(crate) async fn handle(
    _ctx: ToolContext,
    _args: Option<Map<String, serde_json::Value>>,
) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(
        r#"{"status":"not_yet_implemented","tool":"list_databases","phase":3}"#,
    )]))
}
