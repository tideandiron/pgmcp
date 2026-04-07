// src/config.rs
//
// Configuration loading for pgmcp.
//
// Loading order (later entries win):
//   1. Built-in defaults (via serde default attributes)
//   2. TOML config file (path from CLI --config, or ./pgmcp.toml, or /etc/pgmcp/pgmcp.toml)
//   3. PGMCP_* environment variable overrides
//   4. CLI positional connection string (overrides database_url only)
//   5. CLI --transport flag (overrides transport.mode only)
//
// All fields are validated after merging all sources. Validation errors are
// returned as a String describing the first failing constraint. The caller
// (main.rs) is responsible for printing the error and exiting with code 2.

#![allow(dead_code)]

use serde::Deserialize;

/// Top-level configuration for pgmcp.
///
/// Deserialized from a TOML file. All fields have defaults so that a config
/// file containing only `database_url` is valid.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// PostgreSQL connection string (URI or libpq key=value format). Required.
    pub database_url: String,

    /// Connection pool settings.
    #[serde(default)]
    pub pool: PoolConfig,

    /// Transport selection and binding.
    #[serde(default)]
    pub transport: TransportConfig,

    /// Telemetry and logging.
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// Schema cache invalidation.
    #[serde(default)]
    pub cache: CacheConfig,

    /// SQL guardrail policies.
    #[serde(default)]
    pub guardrails: GuardrailConfig,
}

/// Connection pool configuration.
#[derive(Debug, Deserialize)]
pub struct PoolConfig {
    /// Minimum connections to maintain. Pool init fails at startup if this
    /// many connections cannot be established.
    #[serde(default = "default_pool_min_size")]
    pub min_size: u32,

    /// Maximum connections the pool will open simultaneously.
    #[serde(default = "default_pool_max_size")]
    pub max_size: u32,

    /// Seconds to wait for a connection before returning `pg_pool_timeout`.
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout_seconds: u64,

    /// Seconds an idle connection is kept before recycling. 0 = never recycle.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: default_pool_min_size(),
            max_size: default_pool_max_size(),
            acquire_timeout_seconds: default_acquire_timeout(),
            idle_timeout_seconds: default_idle_timeout(),
        }
    }
}

fn default_pool_min_size() -> u32 {
    2
}
fn default_pool_max_size() -> u32 {
    10
}
fn default_acquire_timeout() -> u64 {
    5
}
fn default_idle_timeout() -> u64 {
    300
}

/// Which transport the server listens on.
#[derive(Debug, Default, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    /// Read JSON-RPC from stdin, write to stdout.
    #[default]
    Stdio,
    /// HTTP server: SSE for server-to-client, POST for client-to-server.
    Sse,
}

/// Transport binding configuration.
#[derive(Debug, Deserialize)]
pub struct TransportConfig {
    /// Which transport to activate.
    #[serde(default)]
    pub mode: TransportMode,

    /// Bind host for the SSE transport. Ignored for stdio.
    #[serde(default = "default_transport_host")]
    pub host: String,

    /// Bind port for the SSE transport. Ignored for stdio.
    #[serde(default = "default_transport_port")]
    pub port: u16,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            mode: TransportMode::default(),
            host: default_transport_host(),
            port: default_transport_port(),
        }
    }
}

fn default_transport_host() -> String {
    "127.0.0.1".to_string()
}
fn default_transport_port() -> u16 {
    3000
}

/// Log output format.
#[derive(Debug, Default, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Structured JSON logs, suitable for log aggregators.
    Json,
    /// Human-readable logs with ANSI color, suitable for development.
    #[default]
    Text,
}

/// Telemetry and logging configuration.
#[derive(Debug, Deserialize)]
pub struct TelemetryConfig {
    /// Log format: "json" or "text".
    #[serde(default)]
    pub log_format: LogFormat,

    /// Log level filter in RUST_LOG syntax.
    /// The `RUST_LOG` environment variable takes precedence.
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_format: LogFormat::default(),
            log_level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Schema cache invalidation configuration.
#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    /// Seconds between pg_catalog polls for schema changes.
    #[serde(default = "default_invalidation_interval")]
    pub invalidation_interval_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            invalidation_interval_seconds: default_invalidation_interval(),
        }
    }
}

fn default_invalidation_interval() -> u64 {
    30
}

/// SQL guardrail policy configuration.
#[derive(Debug, Deserialize)]
pub struct GuardrailConfig {
    /// Block DDL statements (CREATE, DROP, ALTER, TRUNCATE) in the query tool.
    #[serde(default = "default_true")]
    pub block_ddl: bool,

    /// Block COPY TO/FROM PROGRAM statements.
    #[serde(default = "default_true")]
    pub block_copy_program: bool,

    /// Block SET statements that change session-level parameters.
    #[serde(default = "default_true")]
    pub block_session_set: bool,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            block_ddl: true,
            block_copy_program: true,
            block_session_set: true,
        }
    }
}

fn default_true() -> bool {
    true
}

// ─── Config methods ───────────────────────────────────────────────────────────

impl Config {
    /// Apply overrides from a slice of `(key, value)` pairs, as if they were
    /// environment variables.
    ///
    /// This method exists for testing. In production, use `apply_env_overrides`.
    /// Keys must use the `PGMCP_` prefix and `__` as the nested separator.
    pub fn apply_env_overrides_from(&mut self, overrides: &[(&str, &str)]) {
        for (key, value) in overrides {
            self.apply_single_env_override(key, value);
        }
    }

    /// Apply all `PGMCP_*` environment variables from the current process environment.
    pub fn apply_env_overrides(&mut self) {
        let pairs: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("PGMCP_"))
            .collect();
        for (key, value) in &pairs {
            self.apply_single_env_override(key, value);
        }
    }

    /// Apply a single environment variable override.
    ///
    /// Key format: `PGMCP_` followed by the config path in SCREAMING_SNAKE_CASE,
    /// with `__` (double underscore) as the nested separator.
    ///
    /// Supported keys:
    /// - `PGMCP_DATABASE_URL`
    /// - `PGMCP_POOL__MIN_SIZE`
    /// - `PGMCP_POOL__MAX_SIZE`
    /// - `PGMCP_POOL__ACQUIRE_TIMEOUT_SECONDS`
    /// - `PGMCP_POOL__IDLE_TIMEOUT_SECONDS`
    /// - `PGMCP_TRANSPORT__MODE`
    /// - `PGMCP_TRANSPORT__HOST`
    /// - `PGMCP_TRANSPORT__PORT`
    /// - `PGMCP_TELEMETRY__LOG_FORMAT`
    /// - `PGMCP_TELEMETRY__LOG_LEVEL`
    /// - `PGMCP_CACHE__INVALIDATION_INTERVAL_SECONDS`
    /// - `PGMCP_GUARDRAILS__BLOCK_DDL`
    /// - `PGMCP_GUARDRAILS__BLOCK_COPY_PROGRAM`
    /// - `PGMCP_GUARDRAILS__BLOCK_SESSION_SET`
    ///
    /// Unknown keys are silently ignored.
    fn apply_single_env_override(&mut self, key: &str, value: &str) {
        match key {
            "PGMCP_DATABASE_URL" => {
                self.database_url = value.to_string();
            }
            "PGMCP_POOL__MIN_SIZE" => {
                if let Ok(v) = value.parse() {
                    self.pool.min_size = v;
                }
            }
            "PGMCP_POOL__MAX_SIZE" => {
                if let Ok(v) = value.parse() {
                    self.pool.max_size = v;
                }
            }
            "PGMCP_POOL__ACQUIRE_TIMEOUT_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.pool.acquire_timeout_seconds = v;
                }
            }
            "PGMCP_POOL__IDLE_TIMEOUT_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.pool.idle_timeout_seconds = v;
                }
            }
            "PGMCP_TRANSPORT__MODE" => match value {
                "stdio" => self.transport.mode = TransportMode::Stdio,
                "sse" => self.transport.mode = TransportMode::Sse,
                _ => {}
            },
            "PGMCP_TRANSPORT__HOST" => {
                self.transport.host = value.to_string();
            }
            "PGMCP_TRANSPORT__PORT" => {
                if let Ok(v) = value.parse() {
                    self.transport.port = v;
                }
            }
            "PGMCP_TELEMETRY__LOG_FORMAT" => match value {
                "json" => self.telemetry.log_format = LogFormat::Json,
                "text" => self.telemetry.log_format = LogFormat::Text,
                _ => {}
            },
            "PGMCP_TELEMETRY__LOG_LEVEL" => {
                self.telemetry.log_level = value.to_string();
            }
            "PGMCP_CACHE__INVALIDATION_INTERVAL_SECONDS" => {
                if let Ok(v) = value.parse() {
                    self.cache.invalidation_interval_seconds = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_DDL" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_ddl = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_COPY_PROGRAM" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_copy_program = v;
                }
            }
            "PGMCP_GUARDRAILS__BLOCK_SESSION_SET" => {
                if let Ok(v) = value.parse() {
                    self.guardrails.block_session_set = v;
                }
            }
            _ => {
                // Unknown PGMCP_ key. Silently ignored per spec.
            }
        }
    }

    /// Override `database_url` from the CLI positional connection string argument.
    pub fn apply_cli_connection_string(&mut self, conn_str: &str) {
        self.database_url = conn_str.to_string();
    }

    /// Validate the fully-merged configuration.
    ///
    /// Returns `Ok(())` when all constraints are satisfied.
    /// Returns `Err(String)` with a human-readable description of the first violation.
    pub fn validate(&self) -> Result<(), String> {
        if self.database_url.is_empty() {
            return Err(
                "database_url is required. Set it in the config file or via \
                 PGMCP_DATABASE_URL, or pass a connection string as a positional argument."
                    .to_string(),
            );
        }
        if self.pool.max_size == 0 {
            return Err("pool.max_size must be greater than 0.".to_string());
        }
        if self.pool.min_size > self.pool.max_size {
            return Err(format!(
                "pool.min_size ({}) must not exceed pool.max_size ({}).",
                self.pool.min_size, self.pool.max_size
            ));
        }
        if self.pool.acquire_timeout_seconds == 0 {
            return Err("pool.acquire_timeout_seconds must be greater than 0.".to_string());
        }
        if self.transport.mode == TransportMode::Sse && self.transport.port == 0 {
            return Err(
                "transport.port must be greater than 0 when transport.mode is \"sse\".".to_string(),
            );
        }
        Ok(())
    }

    /// Load configuration using the full merging strategy:
    ///
    /// 1. Read the TOML file from `config_path` (if Some), or search default paths.
    /// 2. Apply PGMCP_* environment variable overrides.
    /// 3. Apply CLI connection string (if Some).
    /// 4. Apply CLI transport mode (if Some).
    /// 5. Validate the merged result.
    ///
    /// Returns the merged, validated Config or an error string.
    pub fn load(
        config_path: Option<&str>,
        cli_connection_string: Option<&str>,
        cli_transport: Option<&str>,
    ) -> Result<Self, String> {
        let toml_str = Self::read_config_file(config_path)?;
        let mut config: Config =
            toml::from_str(&toml_str).map_err(|e| format!("Failed to parse config file: {e}"))?;
        config.apply_env_overrides();
        if let Some(conn_str) = cli_connection_string {
            config.apply_cli_connection_string(conn_str);
        }
        if let Some(transport) = cli_transport {
            config.apply_single_env_override("PGMCP_TRANSPORT__MODE", transport);
        }
        config.validate()?;
        Ok(config)
    }

    /// Read the config file from the given path or search default locations.
    ///
    /// Default search order:
    ///   1. `./pgmcp.toml`
    ///   2. `/etc/pgmcp/pgmcp.toml`
    ///
    /// If neither default exists and no explicit path was given, returns an
    /// empty TOML string (Config will use all defaults, and validation will
    /// fail if database_url was not provided another way).
    fn read_config_file(config_path: Option<&str>) -> Result<String, String> {
        if let Some(path) = config_path {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read config file '{path}': {e}"));
        }
        // Search default locations.
        for candidate in &["./pgmcp.toml", "/etc/pgmcp/pgmcp.toml"] {
            if std::path::Path::new(candidate).exists() {
                return std::fs::read_to_string(candidate)
                    .map_err(|e| format!("Failed to read config file '{candidate}': {e}"));
            }
        }
        // No config file found; proceed with defaults.
        Ok(String::new())
    }
}

// ─── CLI argument parsing ─────────────────────────────────────────────────────

/// Parsed command-line arguments for pgmcp.
///
/// pgmcp intentionally has a small argument surface (three flags + one
/// positional). Using a hand-rolled parser keeps compile times short and
/// avoids pulling in clap for ~30 lines of work.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CliArgs {
    /// Path to the TOML config file. Corresponds to `--config <path>`.
    pub config: Option<String>,

    /// Transport mode override. Corresponds to `--transport <stdio|sse>`.
    pub transport: Option<String>,

    /// Positional connection string: `pgmcp postgres://...`
    pub connection_string: Option<String>,
}

impl CliArgs {
    /// Parse CLI arguments from an iterator of strings.
    ///
    /// Accepts:
    /// - `--config <path>`
    /// - `--transport <stdio|sse>`
    /// - `--help` / `-h` (prints usage and exits)
    /// - A positional argument beginning with `postgres://` or containing `host=`
    ///
    /// Unknown flags produce a usage message to stderr and a non-zero exit.
    pub fn parse_from(mut args: impl Iterator<Item = String>) -> Self {
        // Skip argv[0] (the binary name).
        let _ = args.next();
        let mut parsed = Self::default();
        let rest: Vec<String> = args.collect();
        let mut i = 0;
        while i < rest.len() {
            match rest[i].as_str() {
                "--help" | "-h" => {
                    Self::print_usage();
                    std::process::exit(0);
                }
                "--config" => {
                    i += 1;
                    if i < rest.len() {
                        parsed.config = Some(rest[i].clone());
                    } else {
                        eprintln!("error: --config requires a path argument");
                        std::process::exit(2);
                    }
                }
                "--transport" => {
                    i += 1;
                    if i < rest.len() {
                        parsed.transport = Some(rest[i].clone());
                    } else {
                        eprintln!("error: --transport requires an argument (stdio|sse)");
                        std::process::exit(2);
                    }
                }
                arg if arg.starts_with("postgres://")
                    || arg.starts_with("postgresql://")
                    || arg.contains("host=") =>
                {
                    parsed.connection_string = Some(rest[i].clone());
                }
                arg if arg.starts_with('-') => {
                    eprintln!("error: unknown flag '{arg}'");
                    Self::print_usage();
                    std::process::exit(2);
                }
                _ => {}
            }
            i += 1;
        }
        parsed
    }

    /// Parse from the real process argv.
    pub fn parse() -> Self {
        Self::parse_from(std::env::args())
    }

    fn print_usage() {
        eprintln!(
            "Usage: pgmcp [OPTIONS] [CONNECTION_STRING]\n\
             \n\
             Options:\n\
             --config <path>          Path to TOML config file\n\
             --transport <stdio|sse>  Transport mode (overrides config)\n\
             --help, -h               Show this help message\n\
             \n\
             Arguments:\n\
             CONNECTION_STRING        PostgreSQL connection string \
             (e.g. postgres://user:pass@host:5432/db)\n\
             Overrides database_url in config and PGMCP_DATABASE_URL.\n\
             \n\
             Examples:\n\
             pgmcp postgres://myuser:mypass@localhost/mydb\n\
             pgmcp --config /etc/pgmcp.toml --transport sse\n\
             pgmcp --transport stdio 'host=localhost dbname=mydb user=me'\n"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TOML deserialization ────────────────────────────────────────────

    #[test]
    fn test_config_from_minimal_toml() {
        let toml = r#"
            database_url = "postgres://user:pass@localhost:5432/db"
        "#;
        let cfg: Config = toml::from_str(toml).expect("minimal config must parse");
        assert_eq!(cfg.database_url, "postgres://user:pass@localhost:5432/db");
        // All other fields must have their defaults.
        assert_eq!(cfg.pool.min_size, 2);
        assert_eq!(cfg.pool.max_size, 10);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 5);
        assert_eq!(cfg.pool.idle_timeout_seconds, 300);
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
        assert_eq!(cfg.transport.host, "127.0.0.1");
        assert_eq!(cfg.transport.port, 3000);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
        assert_eq!(cfg.telemetry.log_level, "info");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 30);
        assert!(cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_config_from_full_toml() {
        let toml = r#"
            database_url = "postgres://myuser:secret@db.example.com:5433/prod"

            [pool]
            min_size = 5
            max_size = 50
            acquire_timeout_seconds = 10
            idle_timeout_seconds = 600

            [transport]
            mode = "sse"
            host = "0.0.0.0"
            port = 8080

            [telemetry]
            log_format = "json"
            log_level = "debug"

            [cache]
            invalidation_interval_seconds = 60

            [guardrails]
            block_ddl = false
            block_copy_program = true
            block_session_set = false
        "#;
        let cfg: Config = toml::from_str(toml).expect("full config must parse");
        assert_eq!(
            cfg.database_url,
            "postgres://myuser:secret@db.example.com:5433/prod"
        );
        assert_eq!(cfg.pool.min_size, 5);
        assert_eq!(cfg.pool.max_size, 50);
        assert_eq!(cfg.pool.acquire_timeout_seconds, 10);
        assert_eq!(cfg.pool.idle_timeout_seconds, 600);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
        assert_eq!(cfg.transport.host, "0.0.0.0");
        assert_eq!(cfg.transport.port, 8080);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
        assert_eq!(cfg.telemetry.log_level, "debug");
        assert_eq!(cfg.cache.invalidation_interval_seconds, 60);
        assert!(!cfg.guardrails.block_ddl);
        assert!(cfg.guardrails.block_copy_program);
        assert!(!cfg.guardrails.block_session_set);
    }

    #[test]
    fn test_transport_mode_deserializes_stdio() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "stdio"
        "#;
        let cfg: Config = toml::from_str(toml).expect("stdio mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Stdio);
    }

    #[test]
    fn test_transport_mode_deserializes_sse() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "sse"
        "#;
        let cfg: Config = toml::from_str(toml).expect("sse mode must parse");
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_log_format_deserializes_json() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [telemetry]
            log_format = "json"
        "#;
        let cfg: Config = toml::from_str(toml).expect("json log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_log_format_deserializes_text() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [telemetry]
            log_format = "text"
        "#;
        let cfg: Config = toml::from_str(toml).expect("text log format must parse");
        assert_eq!(cfg.telemetry.log_format, LogFormat::Text);
    }

    // ── Environment variable overrides ─────────────────────────────────

    #[test]
    fn test_env_override_database_url() {
        // Use a scoped environment variable so parallel tests do not interfere.
        // Serial test: set env var, call apply_env_overrides, unset.
        let mut cfg: Config =
            toml::from_str(r#"database_url = "postgres://original@localhost/db""#).unwrap();
        // Simulate PGMCP_DATABASE_URL override.
        cfg.apply_env_overrides_from(&[(
            "PGMCP_DATABASE_URL",
            "postgres://override@remotehost/newdb",
        )]);
        assert_eq!(cfg.database_url, "postgres://override@remotehost/newdb");
    }

    #[test]
    fn test_env_override_pool_max_size() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_POOL__MAX_SIZE", "99")]);
        assert_eq!(cfg.pool.max_size, 99);
    }

    #[test]
    fn test_env_override_transport_mode_sse() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TRANSPORT__MODE", "sse")]);
        assert_eq!(cfg.transport.mode, TransportMode::Sse);
    }

    #[test]
    fn test_env_override_log_format_json() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_TELEMETRY__LOG_FORMAT", "json")]);
        assert_eq!(cfg.telemetry.log_format, LogFormat::Json);
    }

    #[test]
    fn test_env_override_unknown_key_is_ignored() {
        // Unknown PGMCP_ keys must not panic or error; they are silently ignored.
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.apply_env_overrides_from(&[("PGMCP_DOES_NOT_EXIST", "value")]);
        // Unchanged.
        assert_eq!(cfg.database_url, "postgres://u:p@h/d");
    }

    // ── CLI connection string shorthand ────────────────────────────────

    #[test]
    fn test_cli_connection_string_sets_database_url() {
        let mut cfg: Config =
            toml::from_str(r#"database_url = "postgres://original@localhost/db""#).unwrap();
        cfg.apply_cli_connection_string("postgres://cli_user:cli_pass@cli_host/cli_db");
        assert_eq!(
            cfg.database_url,
            "postgres://cli_user:cli_pass@cli_host/cli_db"
        );
    }

    // ── CLI argument parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_cli_args_config_flag() {
        let args = ["pgmcp", "--config", "/etc/pgmcp.toml"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/etc/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, None);
        assert_eq!(parsed.connection_string, None);
    }

    #[test]
    fn test_parse_cli_args_transport_flag() {
        let args = ["pgmcp", "--transport", "sse"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.transport, Some("sse".to_string()));
    }

    #[test]
    fn test_parse_cli_args_positional_connection_string() {
        let args = ["pgmcp", "postgres://u:p@h/d"];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    #[test]
    fn test_parse_cli_args_all_flags() {
        let args = [
            "pgmcp",
            "--config",
            "/tmp/pgmcp.toml",
            "--transport",
            "stdio",
            "postgres://u:p@h/d",
        ];
        let parsed = CliArgs::parse_from(args.iter().map(|s| s.to_string()));
        assert_eq!(parsed.config, Some("/tmp/pgmcp.toml".to_string()));
        assert_eq!(parsed.transport, Some("stdio".to_string()));
        assert_eq!(
            parsed.connection_string,
            Some("postgres://u:p@h/d".to_string())
        );
    }

    // ── Validation ─────────────────────────────────────────────────────

    #[test]
    fn test_validate_rejects_empty_database_url() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.database_url = String::new();
        assert!(cfg.validate().is_err());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("database_url"));
    }

    #[test]
    fn test_validate_rejects_zero_max_pool_size() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.pool.max_size = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_min_size_greater_than_max_size() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.pool.min_size = 10;
        cfg.pool.max_size = 5;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_zero_acquire_timeout() {
        let mut cfg: Config = toml::from_str(r#"database_url = "postgres://u:p@h/d""#).unwrap();
        cfg.pool.acquire_timeout_seconds = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_sse_port() {
        let toml = r#"
            database_url = "postgres://u:p@h/d"
            [transport]
            mode = "sse"
            port = 0
        "#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.transport.mode = TransportMode::Sse;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_accepts_valid_config() {
        let toml = r#"database_url = "postgres://u:p@h/d""#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.validate().is_ok());
    }
}
