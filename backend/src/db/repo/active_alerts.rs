use crate::db::models::{
    ActiveAlert, AlertType, NewActiveAlert, OpenAlertWithPoint, ServiceStatus, ServiceType,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Raise a new alert for an alert point.
pub async fn insert(pool: &PgPool, alert: NewActiveAlert) -> Result<ActiveAlert, sqlx::Error> {
    sqlx::query_as!(
        ActiveAlert,
        r#"
        INSERT INTO active_alerts (alert_point_id, alert_type, status, reason)
        VALUES ($1, $2, $3, $4)
        RETURNING id, alert_point_id, alert_type AS "alert_type: AlertType", status AS "status: ServiceStatus", reason, triggered_at, resolved_at
        "#,
        alert.alert_point_id,
        alert.alert_type as AlertType,
        alert.status as ServiceStatus,
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

/// All currently open alerts with their targets' metadata, newest first.
/// This is the source for the WebSocket snapshot-on-connect (task 2.4).
pub async fn list_open_with_point(pool: &PgPool) -> Result<Vec<OpenAlertWithPoint>, sqlx::Error> {
    sqlx::query_as!(
        OpenAlertWithPoint,
        r#"
        SELECT
            a.id,
            a.alert_point_id,
            a.alert_type AS "alert_type: AlertType",
            a.status AS "status: ServiceStatus",
            a.reason,
            a.triggered_at,
            p.name,
            p.url,
            p.service_type AS "service_type: ServiceType",
            p.expected_response_time_ms
        FROM active_alerts a
        JOIN alert_points p ON p.id = a.alert_point_id
        WHERE a.resolved_at IS NULL
        ORDER BY a.triggered_at DESC, a.id DESC
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Update the open alert of an alert point (`Update` lifecycle event):
/// replaces type, status and reason, keeping `triggered_at`. Returns the
/// updated row or `None` if the alert was resolved concurrently (the worker
/// then simply proceeds without publishing an update).
pub async fn update_open(
    pool: &PgPool,
    alert_point_id: Uuid,
    alert_type: AlertType,
    status: ServiceStatus,
    reason: String,
) -> Result<Option<ActiveAlert>, sqlx::Error> {
    sqlx::query_as!(
        ActiveAlert,
        r#"
        UPDATE active_alerts
        SET alert_type = $2, status = $3, reason = $4
        WHERE alert_point_id = $1 AND resolved_at IS NULL
        RETURNING id, alert_point_id, alert_type AS "alert_type: AlertType", status AS "status: ServiceStatus", reason, triggered_at, resolved_at
        "#,
        alert_point_id,
        alert_type as AlertType,
        status as ServiceStatus,
        reason,
    )
    .fetch_optional(pool)
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
        SELECT id, alert_point_id, alert_type AS "alert_type: AlertType", status AS "status: ServiceStatus", reason, triggered_at, resolved_at
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
