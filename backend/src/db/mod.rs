pub mod models;
pub mod repo;

pub use models::{ActiveAlert, AlertPoint, AlertType, HttpMethod, ProbeResult, ServiceType};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Embedded migrations from `./migrations`, applied at startup.
static MIGRATOR: Migrator = sqlx::migrate!();

/// Create a PostgreSQL connection pool.
///
/// The pool keeps a small number of connections alive and fails fast if the
/// database cannot be reached within `acquire_timeout`.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
}

/// Apply all pending embedded migrations to the database.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}
