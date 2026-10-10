//! Repository layer for the `alert_points`, `probe_results` and
//! `active_alerts` tables (see `docs/tests.md`).
//!
//! - [`traits`]: object-safe repository traits + [`traits::RepoError`]
//! - the `Pg*` structs in the sibling modules: production sqlx implementations
//! - [`fake`] (behind `test-utils`): in-memory fakes for unit/integration tests

pub mod active_alerts;
pub mod alert_points;
#[cfg(any(test, feature = "test-utils"))]
pub mod fake;
pub mod probe_results;
pub mod traits;

#[cfg(any(test, feature = "test-utils"))]
pub use fake::{FakeActiveAlertStore, FakeAlertPointRepository, FakeProbeResultStore, FakeStores};
pub use traits::{ActiveAlertStore, AlertPointRepository, ProbeResultStore, RepoError};

// re-export the sqlx-backed implementations
pub use active_alerts::PgActiveAlertStore;
pub use alert_points::{ListFilter, PgAlertPointRepository};
pub use probe_results::PgProbeResultStore;
