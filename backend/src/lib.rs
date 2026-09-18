//! Library facade so integration tests (and future modules) can reach the
//! DB layer, queue layer, and configuration without depending on the binary
//! crate internals.
pub mod config;
pub mod db;
pub mod queue;
