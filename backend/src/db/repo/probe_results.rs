//! PostgreSQL implementation of [`ProbeResultStore`] (task 2.1).
//!
//! The sqlx bodies are unchanged from the original free functions; they were
//! only moved behind the trait abstraction in `repo::traits` (docs/tests.md).

use crate::db::models::{NewProbeResult, ProbeResult};
use crate::db::repo::traits::{ProbeResultStore, RepoError};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// sqlx-backed `probe_results` repository wrapping a connection pool.
#[derive(Debug, Clone)]
pub struct PgProbeResultStore {
    pool: PgPool,
}

impl PgProbeResultStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProbeResultStore for PgProbeResultStore {
    /// Persist a probe result. `raw_response_snippet` is truncated to 2000 chars.
    async fn insert(&self, result: NewProbeResult) -> Result<ProbeResult, RepoError> {
        let snippet = result
            .raw_response_snippet
            .map(|s| s.chars().take(2000).collect::<String>());

        sqlx::query_as!(
            ProbeResult,
            r#"
        INSERT INTO probe_results (
            alert_point_id, response_time_ms, status_code,
            is_up, error_message, raw_response_snippet
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING
            id,
            alert_point_id,
            timestamp,
            response_time_ms,
            status_code,
            is_up,
            error_message,
            raw_response_snippet
        "#,
            result.alert_point_id,
            result.response_time_ms,
            result.status_code,
            result.is_up,
            result.error_message,
            snippet,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Most recent probe results for an alert point, newest first.
    async fn list_recent(
        &self,
        alert_point_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ProbeResult>, RepoError> {
        sqlx::query_as!(
            ProbeResult,
            r#"
        SELECT
            id,
            alert_point_id,
            timestamp,
            response_time_ms,
            status_code,
            is_up,
            error_message,
            raw_response_snippet
        FROM probe_results
        WHERE alert_point_id = $1
        ORDER BY timestamp DESC, id DESC
        LIMIT $2
        "#,
            alert_point_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Uptime percentage (0.0–100.0) for an alert point within the last `hours`.
    /// Returns `NULL` if there are no probe results in that window.
    async fn uptime_percentage(
        &self,
        alert_point_id: Uuid,
        hours: i32,
    ) -> Result<Option<f64>, RepoError> {
        sqlx::query_scalar!(
            r#"
        SELECT 100.0 * AVG((is_up)::int)::float8 AS "uptime_pct"
        FROM probe_results
        WHERE alert_point_id = $1
          AND timestamp > now() - make_interval(hours => $2)
        "#,
            alert_point_id,
            hours,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }
}
