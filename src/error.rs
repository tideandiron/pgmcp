// src/error.rs
//
// McpError: the single error type for all fallible operations in pgmcp.
//
// Design invariants (from spec section 3.5):
// - Every error has a machine-readable code (string, lowercase_snake_case).
// - Every error has a human-readable message suitable for returning to an agent.
// - Every error has a hint — an actionable suggestion the agent can act on.
// - Internal source errors (e.g., raw tokio-postgres errors) are stored for
//   logging but are NEVER included in to_json() output sent to agents.
// - McpError is Send + Sync so it can cross async task boundaries.
//
// Constructor convention: one constructor per error code, named after the code.
// This makes callsites readable:
//   return Err(McpError::table_not_found("public", "users"));
//   return Err(McpError::param_invalid("sql", "must not be empty"));

#![allow(dead_code)]

/// The single error type for all fallible operations in pgmcp.
///
/// Every public API boundary returns `Result<T, McpError>`. Raw errors from
/// dependencies (tokio-postgres, toml, etc.) are converted to `McpError` at
/// the module boundary and the original error is stored as `source` for logging.
#[derive(Debug)]
pub struct McpError {
    /// Machine-readable error code. Lowercase snake_case. Stable across versions.
    code: &'static str,

    /// Human-readable message suitable for returning to an agent.
    /// Written to be interpretable by a model, not a human reading a stack trace.
    message: String,

    /// Actionable hint for the agent. Non-empty for all variants.
    hint: String,

    /// Original source error, stored for logging. Never forwarded to agents.
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl McpError {
    // ── Constructors ─────────────────────────────────────────────────────

    /// Configuration is malformed or missing required fields.
    ///
    /// Typical causes: bad env var value, missing required field in TOML,
    /// invalid combination of options.
    pub fn config_invalid(message: impl Into<String>) -> Self {
        Self {
            code: "config_invalid",
            message: message.into(),
            hint: "Check the configuration file and PGMCP_* environment variables. \
                   See config/pgmcp.example.toml for the full schema with defaults."
                .to_string(),
            source: None,
        }
    }

    /// Could not connect to Postgres.
    ///
    /// Typical causes: wrong host, firewall blocking the port, bad credentials,
    /// Postgres not running.
    pub fn pg_connect_failed(message: impl Into<String>) -> Self {
        Self {
            code: "pg_connect_failed",
            message: message.into(),
            hint: "Verify that the PostgreSQL server is running and reachable at the \
                   configured host and port. Check database_url credentials. If using \
                   SSL, verify that the server's certificate is trusted."
                .to_string(),
            source: None,
        }
    }

    /// Postgres version is below the minimum supported version (14).
    pub fn pg_version_unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "pg_version_unsupported",
            message: message.into(),
            hint: "pgmcp requires PostgreSQL 14 or later. Upgrade the server or \
                   connect to a compatible instance."
                .to_string(),
            source: None,
        }
    }

    /// SQL execution error returned by Postgres.
    ///
    /// Typical causes: syntax error, permission denied, constraint violation,
    /// function does not exist.
    pub fn pg_query_failed(message: impl Into<String>) -> Self {
        Self {
            code: "pg_query_failed",
            message: message.into(),
            hint: "Review the SQL statement for syntax errors. Check that the \
                   connected role has the required permissions. Use the explain tool \
                   to analyze the query plan before executing."
                .to_string(),
            source: None,
        }
    }

    /// Could not acquire a connection from the pool within the configured timeout.
    pub fn pg_pool_timeout(message: impl Into<String>) -> Self {
        Self {
            code: "pg_pool_timeout",
            message: message.into(),
            hint: "The connection pool is exhausted. Reduce concurrency, increase \
                   pool.max_size in config, or increase pool.acquire_timeout_seconds. \
                   Check for long-running queries holding connections."
                .to_string(),
            source: None,
        }
    }

    /// Unknown tool name in tool call.
    ///
    /// Typical causes: agent typo, using a tool name from a different version
    /// of pgmcp, or a cloud-only tool in the OSS server.
    pub fn tool_not_found(tool_name: impl Into<String>) -> Self {
        let name = tool_name.into();
        let hint = format!(
            "The tool '{name}' does not exist. Call tools/list to see the available tools \
             and their exact names."
        );
        Self {
            code: "tool_not_found",
            message: format!("unknown tool: '{name}'"),
            hint,
            source: None,
        }
    }

    /// Tool parameter missing, wrong type, or failed validation.
    pub fn param_invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        let field = field.into();
        let reason = reason.into();
        let hint = format!(
            "Check the parameter '{field}': {reason}. Refer to the tool's parameter \
             schema in the tools/list response for valid values and types."
        );
        Self {
            code: "param_invalid",
            message: format!("invalid parameter '{field}': {reason}"),
            hint,
            source: None,
        }
    }

    /// SQL statement blocked by the analysis layer.
    ///
    /// Typical causes: DDL in the query tool, COPY TO/FROM PROGRAM,
    /// SET statements that affect session state.
    pub fn guardrail_violation(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            code: "guardrail_violation",
            message: format!("SQL statement blocked by guardrails: {reason}"),
            hint: "Review the guardrail policies in config.guardrails. DDL statements \
                   should be proposed via the propose_migration tool, not executed \
                   directly. Use dry_run: true to inspect the guardrail analysis \
                   without attempting execution."
                .to_string(),
            source: None,
        }
    }

    /// SQL statement did not parse with the Postgres dialect parser.
    pub fn sql_parse_error(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            code: "sql_parse_error",
            message: format!("SQL parse error: {reason}"),
            hint: "Verify that the SQL is valid PostgreSQL syntax. pgmcp uses the \
                   sqlparser crate with the Postgres dialect. Use dry_run: true to \
                   inspect the parse result before execution."
                .to_string(),
            source: None,
        }
    }

    /// The specified schema does not exist in the database.
    pub fn schema_not_found(schema: impl Into<String>) -> Self {
        let schema = schema.into();
        let hint = format!(
            "Schema '{schema}' does not exist or is not visible to the connected role. \
             Use list_schemas to see available schemas."
        );
        Self {
            code: "schema_not_found",
            message: format!("schema not found: '{schema}'"),
            hint,
            source: None,
        }
    }

    /// The specified table does not exist in the given schema.
    pub fn table_not_found(schema: impl Into<String>, table: impl Into<String>) -> Self {
        let schema = schema.into();
        let table = table.into();
        let hint = format!(
            "Table '{table}' does not exist in schema '{schema}', or is not visible \
             to the connected role. Use list_tables to see available tables."
        );
        Self {
            code: "table_not_found",
            message: format!("table not found: '{schema}.{table}'"),
            hint,
            source: None,
        }
    }

    /// Unexpected error with no more specific code.
    ///
    /// Presence of this error in production logs indicates a bug that should
    /// be filed and fixed.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal",
            message: message.into(),
            hint: "This is an unexpected internal error. Please report it as a bug \
                   with the full error message and the request that triggered it."
                .to_string(),
            source: None,
        }
    }

    // ── Builder for attaching a source error ─────────────────────────────

    /// Attach a source error for logging purposes.
    ///
    /// The source is stored internally and written to the tracing span when
    /// the error is logged. It is NEVER included in `to_json()` output sent
    /// to agents — raw database errors may contain sensitive data.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// tokio_postgres::connect(&url, NoTls).await.map_err(|e| {
    ///     McpError::pg_connect_failed(format!("could not connect to {url}"))
    ///         .with_source(e)
    /// })?;
    /// ```
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    /// Returns the machine-readable error code.
    ///
    /// This is the stable, lowercase_snake_case identifier for the error kind.
    /// Agents should use this field for programmatic error handling.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the agent-actionable hint.
    pub fn hint(&self) -> &str {
        &self.hint
    }

    /// Serialize the error to a `serde_json::Value` for inclusion in MCP responses.
    ///
    /// The output JSON has exactly three fields:
    /// - `code`: machine-readable error code (string)
    /// - `message`: human-readable message for the agent (string)
    /// - `hint`: actionable suggestion for the agent (string)
    ///
    /// The `source` field is intentionally excluded. Raw database errors may
    /// contain sensitive information (query text, schema names, constraint names)
    /// and must not be forwarded to agents.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "message": self.message,
            "hint": self.hint,
        })
    }
}

// ── std::fmt::Display ─────────────────────────────────────────────────────────

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{code}] {message} (hint: {hint})",
            code = self.code,
            message = self.message,
            hint = self.hint,
        )
    }
}

// ── std::error::Error ─────────────────────────────────────────────────────────

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

// ── From conversions ──────────────────────────────────────────────────────────

impl From<tokio_postgres::Error> for McpError {
    /// Convert a raw tokio-postgres error into an `McpError`.
    ///
    /// The conversion inspects the error kind to choose the most specific
    /// error code. The raw error is attached as `source` for logging but is
    /// not forwarded to agents.
    fn from(err: tokio_postgres::Error) -> Self {
        use tokio_postgres::error::SqlState;

        // tokio_postgres::Error::db_error() returns Some if this is a
        // server-reported SQL error (SQLSTATE). Connectivity errors (IO,
        // TLS, protocol) return None.
        if let Some(db_err) = err.as_db_error() {
            let code = db_err.code();
            if code == &SqlState::CONNECTION_FAILURE
                || code == &SqlState::CONNECTION_EXCEPTION
                || code == &SqlState::SQLCLIENT_UNABLE_TO_ESTABLISH_SQLCONNECTION
            {
                return McpError::pg_connect_failed(db_err.message().to_string()).with_source(err);
            }
            // All other DB errors (permission denied, constraint violation,
            // syntax error, etc.) map to pg_query_failed.
            return McpError::pg_query_failed(db_err.message().to_string()).with_source(err);
        }

        // Non-DB errors (IO failure, TLS, unexpected close) indicate a
        // connectivity problem rather than a query failure.
        McpError::pg_connect_failed(format!("postgres connection error: {err}")).with_source(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All 12 error code variants must exist and be constructible.
    #[test]
    fn test_all_error_codes_exist() {
        let _config = McpError::config_invalid("bad toml");
        let _pg_connect = McpError::pg_connect_failed("connection refused");
        let _pg_version = McpError::pg_version_unsupported("version 13 detected");
        let _pg_query = McpError::pg_query_failed("column does not exist");
        let _pg_pool = McpError::pg_pool_timeout("pool exhausted after 5s");
        let _not_found = McpError::tool_not_found("unknown_tool");
        let _param = McpError::param_invalid("sql", "must not be empty");
        let _guardrail = McpError::guardrail_violation("DDL is not permitted in query tool");
        let _sql_parse = McpError::sql_parse_error("unexpected token");
        let _schema = McpError::schema_not_found("nonexistent_schema");
        let _table = McpError::table_not_found("public", "nonexistent_table");
        let _internal = McpError::internal("unexpected None in cache");
    }

    // Display must include the error code for agent consumption.
    #[test]
    fn test_display_includes_error_code() {
        let err = McpError::config_invalid("missing database_url");
        let msg = err.to_string();
        assert!(
            msg.contains("config_invalid"),
            "Display must contain 'config_invalid': got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_message() {
        let err = McpError::pg_connect_failed("connection refused to localhost:5432");
        let msg = err.to_string();
        assert!(
            msg.contains("connection refused to localhost:5432"),
            "Display must include the message: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_pg_connect_failed() {
        let err = McpError::pg_connect_failed("connection refused");
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("hint"),
            "Display must include a hint: got '{msg}'"
        );
    }

    #[test]
    fn test_display_includes_hint_for_config_invalid() {
        let err = McpError::config_invalid("database_url is required");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_guardrail_violation() {
        let err = McpError::guardrail_violation("DDL is not permitted");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_tool_not_found() {
        let err = McpError::tool_not_found("badtool");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_param_invalid() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_schema_not_found() {
        let err = McpError::schema_not_found("ghost_schema");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    #[test]
    fn test_display_includes_hint_for_table_not_found() {
        let err = McpError::table_not_found("public", "ghost_table");
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("hint"), "got '{msg}'");
    }

    // code() accessor must return the machine-readable string.
    #[test]
    fn test_code_returns_correct_string_for_each_variant() {
        assert_eq!(McpError::config_invalid("x").code(), "config_invalid");
        assert_eq!(McpError::pg_connect_failed("x").code(), "pg_connect_failed");
        assert_eq!(
            McpError::pg_version_unsupported("x").code(),
            "pg_version_unsupported"
        );
        assert_eq!(McpError::pg_query_failed("x").code(), "pg_query_failed");
        assert_eq!(McpError::pg_pool_timeout("x").code(), "pg_pool_timeout");
        assert_eq!(McpError::tool_not_found("x").code(), "tool_not_found");
        assert_eq!(McpError::param_invalid("f", "x").code(), "param_invalid");
        assert_eq!(
            McpError::guardrail_violation("x").code(),
            "guardrail_violation"
        );
        assert_eq!(McpError::sql_parse_error("x").code(), "sql_parse_error");
        assert_eq!(McpError::schema_not_found("x").code(), "schema_not_found");
        assert_eq!(
            McpError::table_not_found("s", "x").code(),
            "table_not_found"
        );
        assert_eq!(McpError::internal("x").code(), "internal");
    }

    // McpError must implement std::error::Error.
    #[test]
    fn test_implements_std_error() {
        fn requires_error<E: std::error::Error>(_: &E) {}
        let err = McpError::internal("test");
        requires_error(&err);
    }

    // McpError must be Send + Sync so it can be returned from async handlers.
    #[test]
    fn test_is_send_and_sync() {
        fn requires_send_sync<T: Send + Sync>() {}
        requires_send_sync::<McpError>();
    }

    // McpError must implement Debug.
    #[test]
    fn test_implements_debug() {
        let err = McpError::internal("debug test");
        let dbg = format!("{err:?}");
        assert!(!dbg.is_empty());
    }

    // to_json() must produce a valid JSON object with code, message, and hint fields.
    #[test]
    fn test_to_json_has_required_fields() {
        let err = McpError::param_invalid("sql", "must not be empty");
        let json = err.to_json();
        assert_eq!(json["code"], "param_invalid");
        assert!(json["message"].is_string());
        assert!(json["hint"].is_string());
        // The source field must NOT be present in the JSON (internal errors are logged,
        // not forwarded to agents).
        assert!(json.get("source").is_none());
    }

    #[test]
    fn test_to_json_message_contains_user_facing_message() {
        let err = McpError::table_not_found("myschema", "mytable");
        let json = err.to_json();
        let msg = json["message"].as_str().unwrap();
        assert!(
            msg.contains("mytable"),
            "message must reference the table name: got '{msg}'"
        );
    }

    #[test]
    fn test_to_json_hint_is_non_empty_for_all_variants() {
        let errors = [
            McpError::config_invalid("x"),
            McpError::pg_connect_failed("x"),
            McpError::pg_version_unsupported("x"),
            McpError::pg_query_failed("x"),
            McpError::pg_pool_timeout("x"),
            McpError::tool_not_found("x"),
            McpError::param_invalid("f", "x"),
            McpError::guardrail_violation("x"),
            McpError::sql_parse_error("x"),
            McpError::schema_not_found("x"),
            McpError::table_not_found("s", "x"),
            McpError::internal("x"),
        ];
        for err in &errors {
            let json = err.to_json();
            let hint = json["hint"].as_str().unwrap_or("");
            assert!(
                !hint.is_empty(),
                "hint must be non-empty for {}: got empty string",
                err.code()
            );
        }
    }

    #[test]
    fn test_with_source_does_not_affect_to_json() {
        // Attach a source error and verify it does not appear in to_json output.
        use std::io;
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "connection refused");
        let err = McpError::pg_connect_failed("could not connect").with_source(io_err);
        let json = err.to_json();
        // source must not leak into agent-visible JSON
        assert!(json.get("source").is_none());
        assert_eq!(json["code"], "pg_connect_failed");
    }

    #[test]
    fn test_error_source_chain() {
        use std::error::Error as _;
        use std::io;
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let err = McpError::pg_connect_failed("could not connect").with_source(io_err);
        // std::error::Error::source() must expose the attached error for logging.
        assert!(err.source().is_some());
    }

    #[test]
    fn test_tool_not_found_message_includes_tool_name() {
        let err = McpError::tool_not_found("badtool");
        assert!(err.message().contains("badtool"));
        assert!(err.hint().contains("badtool"));
    }

    #[test]
    fn test_param_invalid_message_includes_field_and_reason() {
        let err = McpError::param_invalid("limit", "must be positive");
        assert!(err.message().contains("limit"));
        assert!(err.message().contains("must be positive"));
    }

    #[test]
    fn test_schema_not_found_hint_includes_schema_name() {
        let err = McpError::schema_not_found("analytics");
        assert!(err.hint().contains("analytics"));
    }

    #[test]
    fn test_table_not_found_message_includes_schema_and_table() {
        let err = McpError::table_not_found("public", "orders");
        assert!(err.message().contains("public"));
        assert!(err.message().contains("orders"));
    }

    #[test]
    fn test_internal_error_hint_mentions_bug() {
        let err = McpError::internal("unexpected None");
        let hint = err.hint();
        assert!(
            hint.to_lowercase().contains("bug") || hint.to_lowercase().contains("report"),
            "internal error hint should mention reporting a bug: got '{hint}'"
        );
    }

    // From<String> conversion for convenience in tests and simple callsites.
    #[test]
    fn test_internal_from_string() {
        let err = McpError::internal("some unexpected state");
        assert_eq!(err.code(), "internal");
    }
}
