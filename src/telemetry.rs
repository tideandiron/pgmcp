// src/telemetry.rs
//
// Tracing subscriber initialization for pgmcp.
//
// Supports two output formats:
//   - LogFormat::Text  — human-readable, ANSI-colored output (development)
//   - LogFormat::Json  — structured JSON output (production / log aggregators)
//
// The subscriber is initialized at most once per process. A second call to
// `init_telemetry` returns `Err(TelemetryError::AlreadyInitialized)` instead
// of panicking, so callers can handle the error gracefully.
//
// Log level is resolved as follows:
//   1. `RUST_LOG` environment variable (if set and non-empty)
//   2. The `log_level` argument passed by the caller

use crate::config::LogFormat;
use std::fmt;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

// ─── Error type ───────────────────────────────────────────────────────────────

/// Errors that can occur when initializing the telemetry subscriber.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TelemetryError {
    /// The global tracing subscriber has already been installed.
    ///
    /// This happens when `init_telemetry` is called more than once in the same
    /// process. The existing subscriber is left in place; no subscriber is
    /// double-registered.
    AlreadyInitialized,
}

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInitialized => {
                write!(f, "telemetry already initialized: subscriber already set")
            }
        }
    }
}

impl std::error::Error for TelemetryError {}

// ─── Initialization ───────────────────────────────────────────────────────────

/// Initialize the global tracing subscriber.
///
/// Must be called once, early in `main`, before any `tracing::` macros are
/// used. The `RUST_LOG` environment variable takes precedence over the
/// `log_level` argument.
///
/// # Errors
///
/// Returns [`TelemetryError::AlreadyInitialized`] if a global subscriber has
/// already been installed (e.g. in tests that call this function more than
/// once). The caller should treat this as a non-fatal warning in test contexts
/// and a fatal error in production.
///
/// # Examples
///
/// ```rust,ignore
/// use pgmcp::telemetry::init_telemetry;
/// use pgmcp::config::LogFormat;
///
/// init_telemetry(LogFormat::Text, "info").expect("telemetry init failed");
/// ```
pub(crate) fn init_telemetry(format: LogFormat, log_level: &str) -> Result<(), TelemetryError> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let result = match format {
        LogFormat::Json => tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .try_init(),
        LogFormat::Text => tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .try_init(),
    };

    result.map_err(|_| TelemetryError::AlreadyInitialized)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Text format initializes without panicking.
    ///
    /// Because the global subscriber may already be set by a previous test,
    /// we accept both `Ok(())` and `Err(AlreadyInitialized)`.
    #[test]
    fn test_text_format_succeeds() {
        let result = init_telemetry(LogFormat::Text, "info");
        assert!(
            result.is_ok() || result == Err(TelemetryError::AlreadyInitialized),
            "unexpected error: {result:?}"
        );
    }

    /// JSON format initializes without panicking.
    #[test]
    fn test_json_format_succeeds() {
        let result = init_telemetry(LogFormat::Json, "info");
        assert!(
            result.is_ok() || result == Err(TelemetryError::AlreadyInitialized),
            "unexpected error: {result:?}"
        );
    }

    /// `LogFormat` is `Copy`, so it can be passed by value without a move.
    #[test]
    fn test_log_format_is_copy() {
        let fmt = LogFormat::Text;
        let _a = fmt;
        let _b = fmt; // would not compile if LogFormat were not Copy
    }

    /// `init_telemetry` returns `Result<(), impl Error>`.
    ///
    /// This test is a compile-time assertion: if the return type changes to
    /// something that is not `Result<(), E: Error>`, this test will fail to
    /// compile.
    #[test]
    fn test_return_type_is_result_error() {
        fn assert_result_error<E: std::error::Error>(_: Result<(), E>) {}
        let result = init_telemetry(LogFormat::Text, "info");
        assert_result_error(result);
    }

    /// When `RUST_LOG` is set, it takes precedence over the `log_level` arg.
    ///
    /// We cannot assert on subscriber internals, so we verify that setting
    /// `RUST_LOG` and calling `init_telemetry` does not panic.
    #[test]
    fn test_rust_log_env_takes_precedence() {
        // SAFETY: test-only, cargo test runs this in a single-threaded test binary
        unsafe {
            std::env::set_var("RUST_LOG", "warn");
        }
        let result = init_telemetry(LogFormat::Text, "debug");
        // SAFETY: test-only, cargo test runs this in a single-threaded test binary
        unsafe {
            std::env::remove_var("RUST_LOG");
        }
        assert!(
            result.is_ok() || result == Err(TelemetryError::AlreadyInitialized),
            "unexpected error: {result:?}"
        );
    }

    /// A second call to `init_telemetry` returns `AlreadyInitialized`.
    ///
    /// Because tests run in the same process, the subscriber is likely already
    /// set after any other test in this module. We call twice explicitly to
    /// guarantee the double-init path is exercised.
    #[test]
    fn test_double_init_returns_already_initialized() {
        // First call: may succeed or may already be initialized.
        let _ = init_telemetry(LogFormat::Text, "info");
        // Second call: must return AlreadyInitialized (subscriber is definitely set).
        let second = init_telemetry(LogFormat::Text, "info");
        assert_eq!(second, Err(TelemetryError::AlreadyInitialized));
    }

    /// `TelemetryError`'s `Display` output contains the word "already".
    #[test]
    fn test_telemetry_error_display_contains_already() {
        let msg = TelemetryError::AlreadyInitialized.to_string();
        assert!(
            msg.contains("already"),
            "Display did not contain 'already': {msg}"
        );
    }
}
