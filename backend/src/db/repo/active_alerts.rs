//! PostgreSQL implementation of [`ActiveAlertStore`] (task 2.1).
//!
//! The sqlx bodies are unchanged from the original free functions; they were
//! only moved behind the trait abstraction in `repo::traits` (docs/tests.md).

use crate::db::models::{ActiveAlert, AlertType, NewActiveAlert};
use crate::db::repo::traits::{ActiveAlertStore, RepoError};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// sqlx-backed `active_alerts` repository wrapping a connection pool.
#[derive(Debug, Clone)]
pub struct PgActiveAlertStore {
    pool: PgPool,
}

impl PgActiveAlertStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Map the `uq_active_alerts_point` unique-index violation to `Conflict`.
fn map_unique_violation(e: sqlx::Error) -> RepoError {
    match &e {
        sqlx::Error::Database(db_err) => {
            if db_err.constraint() == Some("uq_active_alerts_point") {
                RepoError::Conflict
            } else {
                RepoError::from_sqlx(e)
            }
        }
        _ => RepoError::from_sqlx(e),
    }
}

#[async_trait]
impl ActiveAlertStore for PgActiveAlertStore {
    /// Raise a new alert for an alert point.
    /// `RepoError::Conflict` when an open alert already exists for the point.
    async fn insert(&self, alert: NewActiveAlert) -> Result<ActiveAlert, RepoError> {
        sqlx::query_as!(
            ActiveAlert,
            r#"
        INSERT INTO active_alerts (alert_point_id, alert_type, reason)
        VALUES ($1, $2, $3)
        RETURNING
            id,
            alert_point_id,
            alert_type AS "alert_type: AlertType",
            reason,
            triggered_at,
            resolved_at
        "#,
            alert.alert_point_id,
            alert.alert_type as AlertType,
            alert.reason,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_unique_violation)
    }

    /// Resolve all open alerts of an alert point (`Remove` lifecycle event).
    /// Returns the number of resolved rows.
    async fn resolve(&self, alert_point_id: Uuid) -> Result<u64, RepoError> {
        let result = sqlx::query!(
            r#"
        UPDATE active_alerts
        SET resolved_at = now()
        WHERE alert_point_id = $1 AND resolved_at IS NULL
        "#,
            alert_point_id,
        )
        .execute(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)?;
        Ok(result.rows_affected())
    }

    /// All currently open alerts, newest first (initial WS state sync source).
    async fn list_open(&self) -> Result<Vec<ActiveAlert>, RepoError> {
        sqlx::query_as!(
            ActiveAlert,
            r#"
        SELECT
            id,
            alert_point_id,
            alert_type AS "alert_type: AlertType",
            reason,
            triggered_at,
            resolved_at
        FROM active_alerts
        WHERE resolved_at IS NULL
        ORDER BY triggered_at DESC, id DESC
        "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// The open alert of a single alert point, if any.
    async fn latest_open(&self, alert_point_id: Uuid) -> Result<Option<ActiveAlert>, RepoError> {
        sqlx::query_as!(
            ActiveAlert,
            r#"
        SELECT
            id,
            alert_point_id,
            alert_type AS "alert_type: AlertType",
            reason,
            triggered_at,
            resolved_at
        FROM active_alerts
        WHERE alert_point_id = $1 AND resolved_at IS NULL
        ORDER BY triggered_at DESC, id DESC
        LIMIT 1
        "#,
            alert_point_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }
}
