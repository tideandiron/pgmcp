// src/main.rs
//
// pgmcp entry point.
//
// Startup sequence (spec section 3.4):
//  1. Parse CLI args
//  2. Load and validate config
//  3. Initialize telemetry
//  4. Build connection pool
//  5. Check Postgres version (exit 4 if < 14)
//  6. Health check (exit 5 if pool unhealthy)
//  7a. Initial schema cache load (blocks until complete)
//  7b. Spawn background invalidation task
//  8. Start transport

#![deny(warnings)]

mod config;
mod error;
mod pg;
mod server;
mod sql;
mod streaming;
mod telemetry;
mod tools;
mod transport;

use std::{sync::Arc, time::Duration};

use config::{CliArgs, TransportMode};
use pg::pool::Pool;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();
    let config = match config::Config::load(
        args.config.as_deref(),
        args.connection_string.as_deref(),
        args.transport.as_deref(),
    ) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("pgmcp: configuration error: {e}");
            std::process::exit(2);
        }
    };

    if let Err(e) =
        telemetry::init_telemetry(config.telemetry.log_format, &config.telemetry.log_level)
    {
        eprintln!("pgmcp: telemetry error: {e}");
        std::process::exit(2);
    }

    tracing::info!("pgmcp starting");

    // Step 4: Build connection pool.
    let pool = match Pool::build(&config) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!(error = %e, "failed to build connection pool");
            eprintln!("pgmcp: pool error: {e}");
            std::process::exit(3);
        }
    };

    let acquire_timeout = Duration::from_secs(config.pool.acquire_timeout_seconds);

    // Step 5: Postgres version check.
    match pool.check_pg_version(acquire_timeout).await {
        Ok(major) => tracing::info!(pg_major = major, "Postgres version OK"),
        Err(e) => {
            tracing::error!(error = %e, "Postgres version check failed");
            eprintln!("pgmcp: {e}");
            std::process::exit(4);
        }
    }

    // Step 6: Health check.
    match pool.health_check(acquire_timeout).await {
        Ok(()) => tracing::info!("connection pool healthy"),
        Err(e) => {
            tracing::error!(error = %e, "pool health check failed");
            eprintln!("pgmcp: {e}");
            std::process::exit(5);
        }
    }

    let config = Arc::new(config);

    // Step 7a: Initial schema cache load (blocks until complete).
    let cache = match pg::cache::SchemaCache::load_from_pool(&pool).await {
        Ok(c) => {
            tracing::info!("schema cache populated at startup");
            Arc::new(c)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to populate schema cache at startup");
            eprintln!("pgmcp: schema cache error: {e}");
            std::process::exit(5);
        }
    };

    // Step 7b: Spawn background invalidation task.
    let _invalidation_handle = pg::invalidation::spawn_invalidation_task(
        Arc::clone(&cache),
        Arc::clone(&pool),
        config.cache.invalidation_interval_seconds,
    );
    // _invalidation_handle is intentionally not awaited; it runs until process exit.
    // Dropping main's handle aborts the task when main exits.

    // Step 8: Start transport.
    let transport_mode = config.transport.mode;
    tracing::info!(transport = ?transport_mode, "starting transport");

    let result = match transport_mode {
        TransportMode::Stdio => {
            transport::stdio::run(Arc::clone(&pool), Arc::clone(&cache), Arc::clone(&config)).await
        }
        TransportMode::Sse => {
            let ct = CancellationToken::new();

            // Install Ctrl-C handler to trigger graceful shutdown.
            let ct_signal = ct.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("received Ctrl-C, shutting down");
                ct_signal.cancel();
            });

            transport::sse::run(
                Arc::clone(&pool),
                Arc::clone(&cache),
                Arc::clone(&config),
                ct,
            )
            .await
        }
    };

    if let Err(e) = result {
        tracing::error!(error = %e, "transport error");
        eprintln!("pgmcp: transport error: {e}");
        std::process::exit(6);
    }

    tracing::info!("pgmcp stopped");
}
