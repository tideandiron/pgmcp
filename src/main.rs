// src/main.rs
//
// pgmcp entry point.
//
// Startup sequence (per spec section 3.4):
//  1. Parse CLI args
//  2. Load and validate config
//  3. Initialize telemetry
//  4. Build connection pool
//  5. Check Postgres version (exits with code 4 if < 14)
//  6. Health check pool (exits with code 5 if pool cannot serve a connection)
//  7. [feat/006] Initialize server and transport
//  8. Begin serving

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

use config::CliArgs;
use pg::pool::Pool;

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

    // Step 5: Check Postgres version (must be >= 14).
    match pool.check_pg_version(acquire_timeout).await {
        Ok(major) => tracing::info!(pg_major = major, "Postgres version OK"),
        Err(e) => {
            tracing::error!(error = %e, "Postgres version check failed");
            eprintln!("pgmcp: {e}");
            std::process::exit(4);
        }
    }

    // Step 6: Health check — verify pool can serve a live connection.
    match pool.health_check(acquire_timeout).await {
        Ok(()) => tracing::info!("connection pool healthy"),
        Err(e) => {
            tracing::error!(error = %e, "pool health check failed");
            eprintln!("pgmcp: {e}");
            std::process::exit(5);
        }
    }

    tracing::info!(
        transport = ?config.transport.mode,
        "startup complete — transport initialization continues in feat/006"
    );

    // Transport initialization: feat/006.
    // Suppress unused warnings until feat/006 wires these in.
    let _ = (pool, config);
}
