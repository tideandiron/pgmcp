// tests/common/fixtures.rs
//
// PostgreSQL container fixture for integration tests.
//
// Uses testcontainers GenericImage with the official postgres:16-alpine image.
// The container is started once per call; testcontainers handles cleanup on drop.
//
// When Docker is not available, `pg_container` returns `None`. Integration
// tests that require a live container should call `require_pg_container()` and
// return early if Docker is unavailable, marking the test as skipped.

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

/// Docker image name used for the PostgreSQL container in integration tests.
pub const PG_IMAGE: &str = "postgres";

/// Docker image tag — `16-alpine` is a small, well-tested official image.
pub const PG_TAG: &str = "16-alpine";

/// Port Postgres listens on inside the container.
pub const PG_PORT: u16 = 5432;

/// Attempt to start a fresh PostgreSQL container.
///
/// Returns `Some((container, url))` when Docker is available, or `None` when
/// Docker cannot be reached. Hold the `ContainerAsync` alive for the full
/// duration of the test — dropping it stops the container.
///
/// # Panics
///
/// Panics if Docker is reachable but the container fails to start for any
/// other reason (e.g., image pull failure, out-of-memory).
pub async fn pg_container() -> Option<(ContainerAsync<GenericImage>, String)> {
    // Attempt to start the container. A `SocketNotFoundError` means Docker is
    // not running on this host; treat that as a skip condition, not a failure.
    let result = GenericImage::new(PG_IMAGE, PG_TAG)
        .with_exposed_port(PG_PORT.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_USER", "pgmcp_test")
        .with_env_var("POSTGRES_PASSWORD", "pgmcp_test")
        .with_env_var("POSTGRES_DB", "pgmcp_test")
        .start()
        .await;

    let container = match result {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("SocketNotFoundError")
                || msg.contains("No such file")
                || msg.contains("docker.sock")
                || msg.contains("Connection refused")
            {
                // Docker is not available on this host — skip.
                return None;
            }
            panic!("PostgreSQL container failed to start: {e}");
        }
    };

    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(PG_PORT)
        .await
        .expect("container port");

    let url = format!("postgresql://pgmcp_test:pgmcp_test@{host}:{port}/pgmcp_test");
    Some((container, url))
}
