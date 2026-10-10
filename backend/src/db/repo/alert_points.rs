//! PostgreSQL implementation of [`AlertPointRepository`] (task 2.1).
//!
//! The sqlx bodies are unchanged from the original free functions; they were
//! only moved behind the trait abstraction in `repo::traits` (docs/tests.md).

use crate::db::models::{AlertPoint, NewAlertPoint, ServiceType, UpdateAlertPoint};
use crate::db::repo::traits::{AlertPointRepository, RepoError};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// Optional list filters shared by `list_filtered` / `count_filtered`.
#[derive(Debug, Default, Clone)]
pub struct ListFilter {
    /// Exact match on `enabled` when `Some`.
    pub enabled: Option<bool>,
    /// Exact match on `service_type` when `Some`.
    pub service_type: Option<ServiceType>,
    /// Case-insensitive substring match on `name` / `url` (already pattern-escaped).
    pub search: Option<String>,
}

/// sqlx-backed `alert_points` repository wrapping a connection pool.
#[derive(Debug, Clone)]
pub struct PgAlertPointRepository {
    pool: PgPool,
}

impl PgAlertPointRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AlertPointRepository for PgAlertPointRepository {
    /// Insert a new alert point. The client-side generated UUID v7 is stored as-is.
    async fn create(&self, new: NewAlertPoint) -> Result<AlertPoint, RepoError> {
        let id = Uuid::now_v7();
        let custom_headers = if new.custom_headers.is_null() {
            serde_json::json!({})
        } else {
            new.custom_headers
        };

        sqlx::query_as!(
            AlertPoint,
            r#"
        INSERT INTO alert_points (
            id, name, url, service_type,
            check_interval_seconds, expected_response_time_ms,
            http_method, custom_headers, auth_config, enabled
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id,
            name,
            url,
            service_type AS "service_type: ServiceType",
            check_interval_seconds,
            expected_response_time_ms,
            http_method AS "http_method: _",
            custom_headers,
            auth_config,
            enabled,
            created_at,
            updated_at
        "#,
            id,
            new.name,
            new.url,
            new.service_type as ServiceType,
            new.check_interval_seconds,
            new.expected_response_time_ms,
            new.http_method as _,
            custom_headers,
            new.auth_config,
            new.enabled,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// List alert points, optionally only the enabled ones, ordered by creation.
    async fn list(
        &self,
        enabled_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError> {
        sqlx::query_as!(
            AlertPoint,
            r#"
        SELECT
            id,
            name,
            url,
            service_type AS "service_type: ServiceType",
            check_interval_seconds,
            expected_response_time_ms,
            http_method AS "http_method: _",
            custom_headers,
            auth_config,
            enabled,
            created_at,
            updated_at
        FROM alert_points
        WHERE (NOT $1::bool) OR enabled
        ORDER BY created_at, id
        LIMIT $2 OFFSET $3
        "#,
            enabled_only,
            limit,
            offset,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// List alert points with optional filtering (task 2.1 pagination support),
    /// ordered by creation.
    async fn list_filtered(
        &self,
        filter: &ListFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlertPoint>, RepoError> {
        sqlx::query_as!(
            AlertPoint,
            r#"
        SELECT
            id,
            name,
            url,
            service_type AS "service_type: ServiceType",
            check_interval_seconds,
            expected_response_time_ms,
            http_method AS "http_method: _",
            custom_headers,
            auth_config,
            enabled,
            created_at,
            updated_at
        FROM alert_points
        WHERE ($1::bool  IS NULL OR enabled = $1)
          AND ($2::text  IS NULL OR service_type = $2)
          AND ($3::text  IS NULL OR name ILIKE $3 OR url ILIKE $3)
        ORDER BY created_at, id
        LIMIT $4 OFFSET $5
        "#,
            filter.enabled,
            filter.service_type.map(|st| st.as_str().to_string()),
            filter.search,
            limit,
            offset,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Count rows matching [`ListFilter`] (pagination `total`, same WHERE as `list_filtered`).
    async fn count_filtered(&self, filter: &ListFilter) -> Result<i64, RepoError> {
        sqlx::query_scalar!(
            r#"
        SELECT count(*) AS "count!: i64"
        FROM alert_points
        WHERE ($1::bool  IS NULL OR enabled = $1)
          AND ($2::text  IS NULL OR service_type = $2)
          AND ($3::text  IS NULL OR name ILIKE $3 OR url ILIKE $3)
        "#,
            filter.enabled,
            filter.service_type.map(|st| st.as_str().to_string()),
            filter.search,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Fetch a single alert point by id.
    async fn get(&self, id: Uuid) -> Result<AlertPoint, RepoError> {
        sqlx::query_as!(
            AlertPoint,
            r#"
        SELECT
            id,
            name,
            url,
            service_type AS "service_type: ServiceType",
            check_interval_seconds,
            expected_response_time_ms,
            http_method AS "http_method: _",
            custom_headers,
            auth_config,
            enabled,
            created_at,
            updated_at
        FROM alert_points
        WHERE id = $1
        "#,
            id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Partially update an alert point. `None` fields are left unchanged.
    /// `auth_config` distinguishes "unchanged" (`None`) from "set, possibly to NULL".
    async fn update(&self, id: Uuid, input: UpdateAlertPoint) -> Result<AlertPoint, RepoError> {
        // $8 flag marks "auth_config should be set to $9 (which may be NULL)".
        let auth_provided = input.auth_config.is_some();
        let auth_value = input.auth_config.flatten();

        sqlx::query_as!(
            AlertPoint,
            r#"
        UPDATE alert_points SET
            name = COALESCE($2, name),
            url = COALESCE($3, url),
            service_type = COALESCE($4, service_type),
            check_interval_seconds = COALESCE($5, check_interval_seconds),
            expected_response_time_ms = COALESCE($6, expected_response_time_ms),
            http_method = COALESCE($7, http_method),
            custom_headers = COALESCE($10, custom_headers),
            auth_config = CASE WHEN $8::bool THEN $9::jsonb ELSE auth_config END,
            enabled = COALESCE($11, enabled)
        WHERE id = $1
        RETURNING
            id,
            name,
            url,
            service_type AS "service_type: ServiceType",
            check_interval_seconds,
            expected_response_time_ms,
            http_method AS "http_method: _",
            custom_headers,
            auth_config,
            enabled,
            created_at,
            updated_at
        "#,
            id,
            input.name,
            input.url,
            input.service_type as Option<ServiceType>,
            input.check_interval_seconds,
            input.expected_response_time_ms,
            input.http_method as Option<_>,
            auth_provided,
            auth_value,
            input.custom_headers,
            input.enabled,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(RepoError::from_sqlx)
    }

    /// Delete an alert point by id; probe results and alerts cascade.
    async fn delete(&self, id: Uuid) -> Result<bool, RepoError> {
        let result = sqlx::query!("DELETE FROM alert_points WHERE id = $1", id)
            .execute(&self.pool)
            .await
            .map_err(RepoError::from_sqlx)?;
        Ok(result.rows_affected() > 0)
    }
}
