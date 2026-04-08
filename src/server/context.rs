// src/server/context.rs
//
// ToolContext — per-call execution context injected into every tool handler.
//
// Constructed by the dispatcher once per `tools/call` request. Contains
// Arc clones of the shared resources (pool, cache, config). Passing ToolContext
// by value to handlers allows them to take ownership without Clone bounds
// on the resources themselves.

use std::sync::Arc;

use crate::{
    config::Config,
    pg::{cache::SchemaCache, pool::Pool},
};

/// Execution context for a single tool call.
///
/// Created by the dispatcher, passed by value to each tool handler.
/// All fields are `Arc`-wrapped so cloning is cheap.
#[derive(Clone)]
pub struct ToolContext {
    /// Connection pool for acquiring Postgres connections.
    pub(crate) pool: Arc<Pool>,

    /// In-memory schema cache populated at startup and refreshed by the
    /// background invalidation task.
    pub(crate) cache: Arc<SchemaCache>,

    /// Application configuration.
    pub(crate) config: Arc<Config>,
}

impl ToolContext {
    /// Create a new `ToolContext`.
    pub fn new(pool: Arc<Pool>, cache: Arc<SchemaCache>, config: Arc<Config>) -> Self {
        Self {
            pool,
            cache,
            config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ToolContext must be Clone and Send + Sync so it can cross task boundaries.
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}

    #[test]
    fn tool_context_is_send_sync_clone() {
        assert_send_sync::<ToolContext>();
        assert_clone::<ToolContext>();
    }
}
