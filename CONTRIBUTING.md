# Contributing to pgmcp

pgmcp is company-led open source. External contributions are welcome for bug
fixes, correctness improvements, and new tool implementations that fit the
project scope. All contributions follow this development workflow.

## Development Setup

### Prerequisites

- **Rust 1.88+** — `rustup update stable`; the `rust-toolchain.toml` file pins the channel
- **Docker** — required for integration tests (testcontainers pulls postgres images)
- **cargo clippy** and **cargo fmt** — included with the Rust toolchain

### Clone and Build

```bash
git clone https://github.com/tideandiron/pgmcp.git
cd pgmcp

# Verify the build
cargo build

# Run unit tests (no Docker needed)
cargo test --lib

# Run all tests (requires Docker for integration tests)
cargo test
```

### Running Integration Tests

Integration tests use [testcontainers](https://docs.rs/testcontainers) to start
real Postgres containers. Docker must be running on your machine.

```bash
# Run all tests against the default Postgres version (16-alpine)
cargo test

# Run against a specific Postgres version
PGMCP_TEST_PG_VERSION=14 cargo test --tests
PGMCP_TEST_PG_VERSION=15 cargo test --tests
PGMCP_TEST_PG_VERSION=16 cargo test --tests
PGMCP_TEST_PG_VERSION=17 cargo test --tests

# Run a specific test binary
cargo test --test query
cargo test --test error_coverage
cargo test --test dispatcher
```

If Docker is not available, integration tests skip automatically (they print
`SKIP: Docker not available` and return). Only unit tests (`cargo test --lib`)
run unconditionally.

### Code Quality Checks

All of the following must pass before committing:

```bash
# Format check
cargo fmt --check

# Auto-format (if check fails)
cargo fmt

# Lint with all warnings as errors
cargo clippy --all-targets --all-features -- -D warnings

# Full check suite (mirrors CI)
cargo fmt --check && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo test --lib
```

## Branch Model

Every change lives in a feature branch named:

```
feat/NNN-short-description
```

Where `NNN` is zero-padded to three digits (`feat/001`, `feat/012`, `feat/029`).

- One feature per branch
- One branch per PR
- Always branched from `main`
- Squash-merged to `main` (no merge commits)

### Commit Format

```
feat(NNN): short description of what was done

Longer explanation if needed. Describe the why, not just the what.

Co-Authored-By: Your Name <email@example.com>
```

## Code Quality Standards

### Error Handling

- **All errors must use `McpError`** — never use `anyhow`, `eyre`, or `Box<dyn Error>` at public API boundaries.
- Every error must have `code`, `message`, and `hint` populated.
- Raw Postgres error messages must never be forwarded to agents — attach them as `source` via `.with_source(e)`.
- Error codes must match an existing variant in `src/error.rs`. Add a new variant with its own test if a new code is needed.

```rust
// Correct:
Err(McpError::pg_query_failed("column 'x' does not exist")
    .with_source(pg_err))

// Wrong:
Err(Box::new(pg_err))  // type erasure
Err(anyhow::Error::from(pg_err))  // anyhow is banned
```

### SQL Safety

- All SQL executed on behalf of a tool call must pass through `sql::guardrails::check()`.
- No raw SQL interpolation — use parameterized queries or verified static SQL files.
- DDL in tool handlers is only allowed in `propose_migration` (which does not execute).
- Every new guardrail rule must have both block and allow tests.

### Performance

- Never collect rows into an intermediate `Vec<Value>` — use the streaming encoder.
- Pre-allocate buffers using `Vec::with_capacity` where the size is known.
- Do not hold a pool connection across uncontrolled `await` points.
- Profile before adding complexity to the hot path.
- New hot-path changes must be covered by a criterion benchmark.

### Testing

- Every new tool handler requires integration tests in `tests/<tool_name>.rs`.
- Every new `McpError` code requires at least one test in `tests/error_coverage.rs`.
- Every guardrail rule requires both a block test and an allow test.
- Tests must pass against Postgres 14, 15, 16, and 17.
- Tests that require Docker must skip gracefully when Docker is unavailable:

```rust
let Some((_container, url)) = common::fixtures::pg_container().await else {
    eprintln!("SKIP: Docker not available");
    return;
};
```

### Naming Conventions

- Tool names: lowercase snake_case matching the spec (`list_tables`, not `ListTables`)
- Error codes: lowercase snake_case (`pg_query_failed`, not `PgQueryFailed`)
- Module names: lowercase snake_case matching the tool name
- Test names: descriptive snake_case (`test_` prefix for integration tests, no prefix for unit tests)

### Documentation

- Public items must have `///` doc comments.
- Doc comments must include a description, relevant `# Errors` section, and `# Examples` where helpful.
- Tool descriptions in `src/server/tool_defs.rs` must specify:
  - What the response contains (exact field names)
  - Valid parameter values (enum values, ranges)
  - Error conditions
  - Usage guidance for agents

## Adding a New Tool

1. Create `src/tools/<tool_name>.rs` with a `handle(ctx, args)` async function.
2. Add the tool to `src/tools/mod.rs`.
3. Add the tool definition to `src/server/tool_defs.rs` with a full LLM-readable description.
4. Add the dispatch case to `src/server/router.rs`.
5. Add integration tests in `tests/<tool_name>.rs`.
6. Update `src/server/tool_defs.rs` test that checks for exactly N tools.

## Review Process

All PRs require:

1. All CI jobs passing (check, test matrix, deny, bench, check-targets)
2. Reviewer approval (one review from a maintainer)
3. No new `unwrap()` or `expect()` in non-test code without a comment explaining the invariant
4. No new `unsafe` blocks without a SAFETY comment and MIRI verification
5. Test count must not decrease

PRs that add new tools must include at least one integration test per guardrail path (block and allow).

## Banned Dependencies

The following crates are banned. `cargo deny check` enforces this in CI.

| Crate | Reason |
|-------|--------|
| `anyhow` | Type-erases errors; `McpError` is the error surface |
| `eyre` | Same reason as anyhow |
| `chrono` | Soundness issues in local timezone handling; use `time 0.3` |
| `lazy_static` | Global state; use `Arc` injection via `ToolContext` |

`once_cell` may appear as a transitive dependency (from `criterion`, `testcontainers`, `tracing-core`) but must not be added as a direct dependency.

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.
