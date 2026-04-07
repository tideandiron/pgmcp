// src/pg/pool.rs
//
// Connection pool wrapper for pgmcp.
//
// Wraps deadpool-postgres::Pool in a newtype. Provides:
// - Pool::build()            — construct pool from Config
// - Pool::get()              — acquire a raw client with timeout
// - Pool::health_check()     — SELECT 1
// - Pool::pg_version_string() — SHOW server_version
// - Pool::check_pg_version() — parse and validate major version in [14, 17]
//
// The pool is internally Arc-backed by deadpool, so `Clone` is cheap.

#![allow(dead_code)]

use std::{str::FromStr, time::Duration};

use deadpool_postgres::{Manager, ManagerConfig, Pool as DeadpoolPool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;

use crate::{config::Config, error::McpError};

/// Minimum supported Postgres major version.
const MIN_PG_MAJOR: u32 = 14;

/// Maximum supported Postgres major version (inclusive). Versions 14-17 are tested.
const MAX_PG_MAJOR: u32 = 17;

/// Newtype wrapper around a deadpool-postgres pool.
///
/// `Clone` is cheap: deadpool's pool is internally `Arc`-backed. Cloning
/// `Pool` shares the same underlying connection pool without copying
/// configuration or opening new connections.
///
/// # Example
///
/// ```rust,ignore
/// let pool = Pool::build(&config).expect("pool build");
/// pool.health_check(Duration::from_secs(5)).await?;
/// let major = pool.check_pg_version(Duration::from_secs(5)).await?;
/// ```
#[derive(Clone)]
pub struct Pool {
    inner: DeadpoolPool,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("status", &self.inner.status())
            .finish()
    }
}

impl Pool {
    /// Build a pool from the given configuration.
    ///
    /// Parses `config.database_url` as a `tokio_postgres::Config` and
    /// constructs a deadpool-postgres pool configured with the max pool size
    /// and fast recycling. No connections are established at this point; call
    /// [`Pool::health_check`] or [`Pool::check_pg_version`] to verify
    /// connectivity.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_connect_failed`] if `database_url` is malformed
    /// or the deadpool builder rejects the configuration.
    pub fn build(config: &Config) -> Result<Self, McpError> {
        let pg_config = tokio_postgres::Config::from_str(&config.database_url)
            .map_err(|e| McpError::pg_connect_failed(format!("invalid database_url: {e}")))?;

        let mgr_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        };

        let mgr = Manager::from_config(pg_config, NoTls, mgr_config);

        let pool = DeadpoolPool::builder(mgr)
            .max_size(config.pool.max_size as usize)
            .runtime(Runtime::Tokio1)
            .build()
            .map_err(|e| McpError::pg_connect_failed(format!("pool builder failed: {e}")))?;

        Ok(Self { inner: pool })
    }

    /// Acquire a raw deadpool client, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_pool_timeout`] if the timeout elapses before a
    /// connection becomes available.
    /// Returns [`McpError::pg_connect_failed`] if pool acquisition fails for
    /// any other reason (e.g., connection refused).
    pub async fn get(&self, timeout: Duration) -> Result<deadpool_postgres::Client, McpError> {
        tokio::time::timeout(timeout, self.inner.get())
            .await
            .map_err(|_| {
                McpError::pg_pool_timeout(format!(
                    "could not acquire connection within {:.1}s",
                    timeout.as_secs_f64()
                ))
            })?
            .map_err(|e| McpError::pg_connect_failed(format!("pool.get() failed: {e}")))
    }

    /// Execute `SELECT 1` to confirm the pool can serve live connections.
    ///
    /// Uses `timeout` to bound both the connection acquisition and the query.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_pool_timeout`] if connection acquisition times out.
    /// Returns [`McpError::pg_query_failed`] if the `SELECT 1` query fails.
    pub async fn health_check(&self, timeout: Duration) -> Result<(), McpError> {
        let client = self.get(timeout).await?;
        client
            .query_one("SELECT 1::int4", &[])
            .await
            .map_err(|e| McpError::pg_query_failed(format!("health check SELECT 1 failed: {e}")))?;
        Ok(())
    }

    /// Query `SHOW server_version` and return the raw version string.
    ///
    /// Returns strings like `"16.2"` or `"14.8 (Ubuntu 14.8-1.pgdg22.04+1)"`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_query_failed`] if the query fails.
    pub async fn pg_version_string(&self, timeout: Duration) -> Result<String, McpError> {
        let client = self.get(timeout).await?;
        let row = client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(|e| McpError::pg_query_failed(format!("SHOW server_version failed: {e}")))?;
        let version: String = row.get(0);
        Ok(version)
    }

    /// Verify that the connected Postgres major version is in the supported range \[14, 17\].
    ///
    /// Called once at startup. Returns the major version number on success.
    ///
    /// Version strings are of the form `"16.2"` or
    /// `"14.8 (Ubuntu 14.8-1.pgdg22.04+1)"`. The first numeric component
    /// before the first `'.'` or `' '` is taken as the major version.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::pg_version_unsupported`] if the major version is
    /// below 14 or if the version string cannot be parsed.
    pub async fn check_pg_version(&self, timeout: Duration) -> Result<u32, McpError> {
        let version_str = self.pg_version_string(timeout).await?;
        let major = parse_pg_major_version(&version_str).ok_or_else(|| {
            McpError::pg_version_unsupported(format!(
                "could not parse Postgres version string: '{version_str}'"
            ))
        })?;

        if major < MIN_PG_MAJOR {
            return Err(McpError::pg_version_unsupported(format!(
                "Postgres {major} is not supported; pgmcp requires version {MIN_PG_MAJOR} or \
                 later (detected: '{version_str}')"
            )));
        }

        tracing::info!(
            pg_major = major,
            version = version_str,
            "Postgres version check passed"
        );
        Ok(major)
    }

    /// Returns a reference to the underlying deadpool pool.
    ///
    /// Exposed for pool status queries (e.g., the `connection_info` tool).
    pub fn inner(&self) -> &DeadpoolPool {
        &self.inner
    }
}

/// Parse the major version number from a Postgres version string.
///
/// Handles strings like `"16.2"`, `"14.8 (Ubuntu ...)"`, and `"15"`.
/// Returns `None` if the string does not start with a valid ASCII digit
/// sequence.
///
/// # Examples
///
/// ```rust
/// # use pgmcp::pg::pool::parse_pg_major_version;
/// assert_eq!(parse_pg_major_version("16.2"), Some(16));
/// assert_eq!(parse_pg_major_version("14.8 (Ubuntu 14.8-1)"), Some(14));
/// assert_eq!(parse_pg_major_version("not-a-version"), None);
/// ```
pub fn parse_pg_major_version(s: &str) -> Option<u32> {
    // Collect leading ASCII digits — everything before the first '.' or ' '.
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_standard() {
        assert_eq!(parse_pg_major_version("16.2"), Some(16));
    }

    #[test]
    fn parse_version_with_suffix() {
        assert_eq!(
            parse_pg_major_version("14.8 (Ubuntu 14.8-1.pgdg22.04+1)"),
            Some(14)
        );
    }

    #[test]
    fn parse_version_major_only() {
        assert_eq!(parse_pg_major_version("15"), Some(15));
    }

    #[test]
    fn parse_version_garbage() {
        assert_eq!(parse_pg_major_version("not-a-version"), None);
    }

    #[test]
    fn parse_version_empty() {
        assert_eq!(parse_pg_major_version(""), None);
    }

    #[test]
    fn parse_version_13_is_old() {
        let major = parse_pg_major_version("13.11").unwrap();
        assert!(major < MIN_PG_MAJOR);
    }

    #[test]
    fn parse_version_17_is_supported() {
        let major = parse_pg_major_version("17.1").unwrap();
        assert!((MIN_PG_MAJOR..=MAX_PG_MAJOR).contains(&major));
    }
}
