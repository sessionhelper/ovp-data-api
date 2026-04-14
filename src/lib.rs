//! chronicle-data-api — internal application bus for the Chronicle stack.
//!
//! The library crate exists primarily so integration tests can import the
//! router + state directly. The binary in `src/main.rs` is a thin wrapper.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod ids;
pub mod metrics;
pub mod routes;
pub mod state;
pub mod storage;
