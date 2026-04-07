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

use config::CliArgs;

fn main() {
    let args = CliArgs::parse();
    let _config = match config::Config::load(
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
    // Startup sequence continues in feat/003 (telemetry) and feat/005 (pool).
}
