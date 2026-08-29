//! Structured configuration (task 1.3).
//!
//! Values are read from the process environment after `dotenvy` has loaded
//! `.env`; required variables without a default (currently only
//! `DATABASE_URL`) cause startup to fail fast with a clear error.

use serde::Deserialize;

/// Default Valkey/Redis endpoint (matches `docker/docker-compose.yml`).
fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

/// Default bind address: all interfaces.
fn default_server_host() -> String {
    "0.0.0.0".to_string()
}

/// Default HTTP port (matches the frontend dev proxy and docker-compose).
fn default_server_port() -> u16 {
    3000
}

/// Default tracing filter applied when `RUST_LOG` is not set.
fn default_log_level() -> String {
    "info,tower_http=debug".to_string()
}

/// Application configuration, deserialized from the environment.
///
/// Field names map case-insensitively to environment variables
/// (`database_url` → `DATABASE_URL`, ...).
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// PostgreSQL connection string. Required.
    pub database_url: String,
    /// Valkey/Redis connection string. Defaults to `redis://127.0.0.1:6379`.
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
    /// HTTP listener host. Defaults to `0.0.0.0`.
    #[serde(default = "default_server_host")]
    pub server_host: String,
    /// HTTP listener port. Defaults to `3000`.
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    /// `tracing` filter used when `RUST_LOG` is unset.
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

impl Config {
    /// Build the configuration from the current process environment.
    pub fn from_env() -> Result<Self, envy::Error> {
        envy::from_env()
    }
}
