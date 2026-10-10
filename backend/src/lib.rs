//! Library facade so integration tests (and future modules) can reach the
//! DB layer, queue layer, configuration, and the REST API without depending
//! on the binary crate internals.
pub mod api;
pub mod config;
pub mod db;
pub mod queue;
