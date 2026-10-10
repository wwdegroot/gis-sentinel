//! Repository trait abstractions (see `docs/tests.md`).
//!
//! The traits decouple the API layer from sqlx so unit/integration tests can
//! substitute the in-memory fakes from [`crate::db::repo::fake`]. The
//! production implementations live next door (`pg_` structs in the sibling
//! modules) and keep the original `sqlx::query!` macro bodies unchanged.
//!
//! Async trait methods use the `async-trait` crate so the traits are
//! object-safe and can be held as `Arc<dyn …>` in [`crate::api::ApiState`].

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::models::{
    ActiveAlert, AlertPoint, NewActiveAlert, NewAlertPoint, NewProbeResult, ProbeResult,
    UpdateAlertPoint,
};
use crate::db::repo::alert_points::ListFilter;

/// Error type returned by all repository traits.
///
/// Fakes and Pg implementations must be able to construct every variant
/// without depending on sqlx internals, hence the catch-all [`RepoError::Other`].
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// Requested row does not exist (mapped to HTTP 404).
    #[error("resource not found")]
    NotFound,
    /// Operation violates a uniqueness constraint (mapped to HTTP 409).
    /// Currently: a second open alert per alert point (`uq_active_alerts_point`).
    #[error("conflicting resource state")]
    Conflict,
    /// Underlying database error; `RowNotFound` is normalized to
    /// [`RepoError::NotFound`], everything else maps to HTTP 500.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    /// Failure raised by a test fake via its failure-injection hook
    /// (or any backend error that does not fit the other variants).
    #[error("repository failure: {0}")]
    Other(String),
}

impl RepoError {
    /// Normalize a raw sqlx error into a `RepoError`.
    pub fn from_sqlx(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => RepoError::NotFound,
            other => RepoError::Db(other),
        }
    }
}

/// CRUD + filtered listing for monitoring targets (`alert_points`).
#[async_trait]
pub trait AlertPointRepository: Send + Sync + std::fmt::Debug {
    /// Insert a new alert point. The client-side generated UUID v7 is stored as-is.
    async fn create(&self, new: NewAlertPoint) -> Result<AlertPoint, RepoError>;

    /// Fetch a single alert point by id; `RepoError::NotFound` when missing.
    async fn get(&self, id: Uuid) -> Result<AlertPoint, RepoError>;

    /// List alert points, optionally only the enabled ones, ordered by creation.
    async fn list(
        &self,
        enabled_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError>;

    /// List alert points with optional filtering, ordered by creation.
    async fn list_filtered(
        &self,
        filter: &ListFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError>;

    /// Count rows matching `filter` (pagination `total`).
    async fn count_filtered(&self, filter: &ListFilter) -> Result<i64, RepoError>;

    /// Partially update an alert point; `None` fields are left unchanged.
    async fn update(&self, id: Uuid, input: UpdateAlertPoint) -> Result<AlertPoint, RepoError>;

    /// Delete an alert point by id; probe results and alerts cascade.
    /// Returns `false` when the id did not exist.
    async fn delete(&self, id: Uuid) -> Result<bool, RepoError>;
}

/// Persistence of probe execution results.
#[async_trait]
pub trait ProbeResultStore: Send + Sync + std::fmt::Debug {
    /// Persist a probe result. Implementations must truncate
    /// `raw_response_snippet` to 2000 characters before storing.
    async fn insert(&self, result: NewProbeResult) -> Result<ProbeResult, RepoError>;

    /// Most recent probe results for an alert point, newest first.
    async fn list_recent(
        &self,
        alert_point_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ProbeResult>, RepoError>;

    /// Uptime percentage (0.0–100.0) within the last `hours`;
    /// `None` when there are no probe results in that window.
    async fn uptime_percentage(
        &self,
        alert_point_id: Uuid,
        hours: i32,
    ) -> Result<Option<f64>, RepoError>;
}

/// Alert lifecycle storage (at most one open alert per alert point).
#[async_trait]
pub trait ActiveAlertStore: Send + Sync + std::fmt::Debug {
    /// Raise a new alert. `RepoError::Conflict` when an open alert already
    /// exists for the alert point (unique index `uq_active_alerts_point`).
    async fn insert(&self, alert: NewActiveAlert) -> Result<ActiveAlert, RepoError>;

    /// Resolve all open alerts of an alert point; returns resolved row count.
    async fn resolve(&self, alert_point_id: Uuid) -> Result<u64, RepoError>;

    /// All currently open alerts, newest first (initial WS state sync source).
    async fn list_open(&self) -> Result<Vec<ActiveAlert>, RepoError>;

    /// The open alert of a single alert point, if any.
    async fn latest_open(&self, alert_point_id: Uuid) -> Result<Option<ActiveAlert>, RepoError>;
}
