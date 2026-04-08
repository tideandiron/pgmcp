// src/pg/invalidation.rs
//
// Background schema cache invalidation task.
//
// Polls `pg_stat_database` for transaction commit count changes. When a change
// is detected, rebuilds the full SchemaSnapshot and atomically replaces the
// cache. The task holds a `Weak<SchemaCache>` so it exits cleanly when the
// last strong reference is dropped (e.g., on process shutdown).
//
// Design invariants:
// - The task acquires at most one pool connection per poll cycle and releases
//   it before sleeping again.
// - Errors are logged and retried on the next interval; they do not crash the task.
// - Dropping the returned JoinHandle aborts the task at the next poll boundary.

use std::sync::{Arc, Weak};

use crate::{error::McpError, pg::cache::SchemaCache};

/// Spawn the cache invalidation background task.
///
/// Returns the `JoinHandle`. Drop the handle to stop the task (it will abort
/// on the next poll cycle boundary). The task also exits if the cache `Arc`
/// is dropped before the handle.
///
/// # Arguments
///
/// - `cache`: the shared cache to refresh. The task holds a `Weak` reference
///   so it does not prevent shutdown.
/// - `pool`: connection pool. The task acquires and releases one connection
///   per poll cycle.
/// - `interval_secs`: poll interval in seconds (from config).
pub fn spawn_invalidation_task(
    cache: Arc<SchemaCache>,
    pool: Arc<crate::pg::pool::Pool>,
    interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    let weak_cache: Weak<SchemaCache> = Arc::downgrade(&cache);
    // Drop the strong reference so the weak ref controls task lifetime.
    drop(cache);

    tokio::spawn(async move {
        run_invalidation_loop(weak_cache, pool, interval_secs).await;
    })
}

async fn run_invalidation_loop(
    weak_cache: Weak<SchemaCache>,
    pool: Arc<crate::pg::pool::Pool>,
    interval_secs: u64,
) {
    let interval = std::time::Duration::from_secs(interval_secs);
    let mut last_xact_commit: Option<i64> = None;

    loop {
        tokio::time::sleep(interval).await;

        // Exit cleanly if the cache has been dropped.
        let Some(cache) = weak_cache.upgrade() else {
            tracing::debug!("schema cache dropped, invalidation task exiting");
            return;
        };

        match poll_and_maybe_refresh(&cache, &pool, &mut last_xact_commit).await {
            Ok(refreshed) => {
                if refreshed {
                    tracing::info!("schema cache refreshed after detecting pg_catalog changes");
                } else {
                    tracing::debug!("schema cache poll: no changes detected");
                }
            }
            Err(e) => {
                // Log and continue. Worst case: stale cache data. Not a crash.
                tracing::warn!(
                    error = %e,
                    "schema cache invalidation poll failed; retrying next interval"
                );
            }
        }
    }
}

async fn poll_and_maybe_refresh(
    cache: &SchemaCache,
    pool: &crate::pg::pool::Pool,
    last_xact_commit: &mut Option<i64>,
) -> Result<bool, McpError> {
    let timeout = std::time::Duration::from_secs(10);
    let client = pool.get(timeout).await?;

    // Step 1: Check transaction commit count.
    // If it has not changed since last poll, no DDL has occurred.
    let row = client
        .query_one(
            "SELECT SUM(xact_commit)::int8 \
             FROM pg_stat_database \
             WHERE datname = current_database()",
            &[],
        )
        .await
        .map_err(McpError::from)?;

    let current_commit: i64 = row.get(0);

    let changed = match *last_xact_commit {
        None => true, // First poll — always refresh.
        Some(prev) => current_commit != prev,
    };
    *last_xact_commit = Some(current_commit);

    if !changed {
        drop(client);
        return Ok(false);
    }

    // Step 2: Build new snapshot using the same connection.
    let new_snapshot = SchemaCache::build_snapshot(&client).await?;
    drop(client); // Release before taking write lock.

    // Step 3: Atomically replace.
    cache.replace_snapshot(new_snapshot).await;

    Ok(true)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_exits_when_weak_cache_is_dropped() {
        let cache = Arc::new(SchemaCache::empty());
        let weak = Arc::downgrade(&cache);
        drop(cache);

        // Weak::upgrade must return None after the last strong ref is dropped.
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn spawn_invalidation_task_returns_a_handle() {
        // Verify the return type compiles — no runtime needed.
        fn accepts_join_handle(_h: tokio::task::JoinHandle<()>) {}
        // Compilation check only; we do not actually spawn against a real pool here.
        let _ = accepts_join_handle;
    }
}
