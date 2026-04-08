# Dockerfile for pgmcp
#
# Multi-stage build:
#   Stage 1 (builder) — Rust + musl toolchain, compiles a fully static binary.
#   Stage 2 (final)   — FROM scratch, contains only the binary and CA certs.
#
# Target image size: ~8–10 MB (binary) + ~200 KB (ca-certificates).
#
# Build:
#   docker build -t pgmcp:latest .
#
# Run:
#   docker run --rm \
#     -e PGMCP_DATABASE_URL="postgres://user:pass@host:5432/db" \
#     pgmcp:latest
#
# Or with a config file:
#   docker run --rm \
#     -v /path/to/pgmcp.toml:/etc/pgmcp/pgmcp.toml:ro \
#     pgmcp:latest --config /etc/pgmcp/pgmcp.toml
#
# Health check:
#   docker run --rm pgmcp:latest --help

# ── Stage 1: builder ──────────────────────────────────────────────────────────
FROM rust:1.85-alpine AS builder

# Install musl-tools for static linking and OpenSSL for any TLS dependencies.
# The pgmcp binary uses NoTls by default (tokio-postgres NoTls), so OpenSSL
# is not strictly required, but we install it for completeness.
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    git

# Set up the static musl target.
RUN rustup target add x86_64-unknown-linux-musl

# Set the working directory.
WORKDIR /build

# Copy dependency manifests first for layer caching.
# This allows Docker to cache the dependency compilation layer separately
# from the source code compilation layer.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# Create a minimal dummy src to compile dependencies without application code.
# This is a standard Docker layer-caching technique for Rust projects.
RUN mkdir src && \
    echo 'fn main() {}' > src/main.rs && \
    echo 'pub fn dummy() {}' > src/lib.rs && \
    mkdir -p benches && \
    echo 'fn main() {}' > benches/serialization.rs && \
    echo 'fn main() {}' > benches/streaming.rs && \
    echo 'fn main() {}' > benches/connection.rs

# Compile dependencies only (the dummy src will be replaced by real source).
RUN RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --target x86_64-unknown-linux-musl 2>&1 || true

# Remove the dummy artifacts so the real source triggers a rebuild.
RUN rm -f target/x86_64-unknown-linux-musl/release/pgmcp* \
          target/x86_64-unknown-linux-musl/release/deps/pgmcp*

# Copy the real source code.
COPY src/ src/
COPY benches/ benches/

# Compile the real binary with full optimizations.
# LTO + single codegen unit + strip are set in Cargo.toml [profile.release].
RUN RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --target x86_64-unknown-linux-musl --bin pgmcp

# Verify the binary exists and reports a version/help string.
# The musl target produces a self-contained binary with no glibc dependency.
RUN ls -lh target/x86_64-unknown-linux-musl/release/pgmcp && \
    echo "Binary size: $(du -sh target/x86_64-unknown-linux-musl/release/pgmcp | cut -f1)"

# ── Stage 2: final (scratch) ──────────────────────────────────────────────────
FROM scratch

# Import CA certificates from the builder stage.
# Required for TLS connections to Postgres (e.g., RDS, Supabase, Neon).
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Copy the statically linked binary.
COPY --from=builder \
    /build/target/x86_64-unknown-linux-musl/release/pgmcp \
    /usr/local/bin/pgmcp

# pgmcp listens on this port when transport.mode = "sse".
# When transport.mode = "stdio" (default), no port is needed.
EXPOSE 3000

# Default entrypoint.
# The connection string or --config flag must be provided at runtime.
ENTRYPOINT ["/usr/local/bin/pgmcp"]
