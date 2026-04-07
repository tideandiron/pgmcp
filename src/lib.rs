// src/lib.rs
//
// Library target for pgmcp.
//
// Exposes internal modules for integration tests. Production code enters
// through src/main.rs; this file makes the same modules available to
// the test harness under the `pgmcp` crate name.

#![allow(dead_code)]

pub mod config;
pub mod error;
pub mod pg;
pub mod server;
pub mod sql;
pub mod streaming;
pub mod telemetry;
pub mod tools;
pub mod transport;
