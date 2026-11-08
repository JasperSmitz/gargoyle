//! Pure logic for Gargoyle: configuration, the health model and the HTTP API DTOs.
//!
//! This crate performs no IO and does not depend on tokio, so everything in it is cheap to test.

pub mod api;
pub mod config;
pub mod health;

pub use health::Health;
