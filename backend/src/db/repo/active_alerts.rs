use crate::db::models::{ActiveAlert, AlertType, NewActiveAlert};
use sqlx::PgPool;
use uuid::Uuid;

/// Raise a new alert for an alert point.
pub async fn insert(pool: &PgPool, alert: NewActiveAlert) -> Result<ActiveAlert, sqlx::Error> {
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
    .fetch_one(pool)
    .await
}

/// Resolve all open alerts of an alert point (`Remove` lifecycle event).
/// Returns the number of resolved rows.
pub async fn resolve(pool: &PgPool, alert_point_id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE active_alerts
        SET resolved_at = now()
        WHERE alert_point_id = $1 AND resolved_at IS NULL
        "#,
        alert_point_id,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// All currently open alerts, newest first (initial WS state sync source).
pub async fn list_open(pool: &PgPool) -> Result<Vec<ActiveAlert>, sqlx::Error> {
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
    .fetch_all(pool)
    .await
}

/// The open alert of a single alert point, if any.
pub async fn latest_open(
    pool: &PgPool,
    alert_point_id: Uuid,
) -> Result<Option<ActiveAlert>, sqlx::Error> {
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
    .fetch_optional(pool)
    .await
}
