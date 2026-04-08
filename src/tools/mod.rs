// src/tools/mod.rs
pub mod connection_info;
pub mod describe_table;
pub(crate) mod explain;
pub mod health;
pub mod list_databases;
pub mod list_enums;
pub mod list_extensions;
pub mod list_schemas;
pub mod list_tables;
pub(crate) mod my_permissions;
pub(crate) mod propose_migration;
pub(crate) mod query;
pub(crate) mod query_events;
pub mod server_info;
pub(crate) mod suggest_index;
pub mod table_stats;
