use crate::db::models::{AlertPoint, ServiceType};
use crate::db::models::{NewAlertPoint, UpdateAlertPoint};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
/// Insert a new alert point. The client-side generated UUID v7 is stored as-is.
pub async fn create(pool: &PgPool, new: NewAlertPoint) -> Result<AlertPoint, sqlx::Error> {
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
            updated_at,
            last_checked_at
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
    .fetch_one(pool)
    .await
}

/// List alert points with optional filtering and pagination, ordered by creation.
///
/// `enabled_only` restricts to enabled points, `service_type` (when given)
/// filters on the GIS service type. Pagination is `limit`/`offset` based.
pub async fn list(
    pool: &PgPool,
    enabled_only: bool,
    service_type: Option<ServiceType>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AlertPoint>, sqlx::Error> {
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
            updated_at,
            last_checked_at
        FROM alert_points
        WHERE ((NOT $1::bool) OR enabled)
          AND ($2::text IS NULL OR service_type = $2::text)
        ORDER BY created_at, id
        LIMIT $3 OFFSET $4
        "#,
        enabled_only,
        service_type as Option<ServiceType>,
        limit,
        offset,
    )
    .fetch_all(pool)
    .await
}

/// Count alert points matching the same filter as [`list`] (for pagination totals).
pub async fn count(
    pool: &PgPool,
    enabled_only: bool,
    service_type: Option<ServiceType>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT count(*)::bigint AS "count!"
        FROM alert_points
        WHERE ((NOT $1::bool) OR enabled)
          AND ($2::text IS NULL OR service_type = $2::text)
        "#,
        enabled_only,
        service_type as Option<ServiceType>,
    )
    .fetch_one(pool)
    .await
}

/// Fetch a single alert point by id.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<AlertPoint, sqlx::Error> {
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
            updated_at,
            last_checked_at
        FROM alert_points
        WHERE id = $1
        "#,
        id,
    )
    .fetch_one(pool)
    .await
}

/// Partially update an alert point. `None` fields are left unchanged.
/// `auth_config` distinguishes "unchanged" (`None`) from "set, possibly to NULL".
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    input: UpdateAlertPoint,
) -> Result<AlertPoint, sqlx::Error> {
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
            updated_at,
            last_checked_at
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
    .fetch_one(pool)
    .await
}

/// Delete an alert point by id; probe results and alerts cascade.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM alert_points WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// All enabled alert points whose check interval has elapsed (scheduler
/// producer query, task 2.2). A point is due when it has never been checked
/// or `last_checked_at + check_interval_seconds <= now()`.
pub async fn list_due(pool: &PgPool) -> Result<Vec<AlertPoint>, sqlx::Error> {
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
            updated_at,
            last_checked_at
        FROM alert_points
        WHERE enabled
          AND (
                last_checked_at IS NULL
                OR last_checked_at + make_interval(secs => check_interval_seconds) <= now()
              )
        ORDER BY created_at, id
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Mark an alert point as just enqueued for probing (scheduler bookkeeping).
/// Returns the recorded timestamp.
pub async fn touch_last_checked(pool: &PgPool, id: Uuid) -> Result<DateTime<Utc>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        UPDATE alert_points
        SET last_checked_at = now()
        WHERE id = $1
        RETURNING last_checked_at AS "last_checked_at!"
        "#,
        id,
    )
    .fetch_one(pool)
    .await
}
