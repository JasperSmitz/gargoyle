//! Pure logic for Gargoyle: configuration, the raw collector data model, the health model, the
//! HTTP API DTOs and the Liner report parser.
//!
//! This crate performs no IO and does not depend on tokio, so everything in it is cheap to test.

pub mod api;
pub mod config;
pub mod health;
pub mod liner;
pub mod model;
pub mod rates;

pub use health::Health;
