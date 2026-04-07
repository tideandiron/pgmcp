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

    // Startup sequence continues in feat/005 (pool).
}
