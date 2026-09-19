use crate::db::models::{NewProbeResult, ProbeResult};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// One point of the graph series returned by `GET /{id}/history`.
/// Deliberately minimal — only what the latency chart needs.
#[derive(Debug, Serialize)]
pub struct ProbeHistoryPoint {
    /// When the probe ran (ISO 8601).
    pub timestamp: DateTime<Utc>,
    /// Round-trip time; `None` when the probe never got a response.
    pub response_time_ms: Option<i32>,
    /// Whether the probe counted as up.
    pub is_up: bool,
}

/// A failed probe with enough detail for the history-page incident table
/// (`GET /{id}/incidents`) and the CSV/JSON export.
#[derive(Debug, Serialize)]
pub struct ProbeIncident {
    /// When the probe ran (ISO 8601).
    pub timestamp: DateTime<Utc>,
    pub response_time_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub is_up: bool,
    pub error_message: Option<String>,
}

/// Response of `GET /api/v1/alert-points/{id}/history` (task 3.5):
/// uptime over the requested window plus the (capped) probe series.
#[derive(Debug, Serialize)]
pub struct ProbeHistory {
    /// Requested window in hours (echoed back to the client).
    pub hours: i32,
    /// Uptime percentage over the window; `None` when no probes exist.
    pub uptime_pct: Option<f64>,
    /// Probe results, oldest first, capped at [`HISTORY_ROW_CAP`].
    pub probes: Vec<ProbeHistoryPoint>,
}

/// Maximum rows returned for one history request.
pub const HISTORY_ROW_CAP: i64 = 5_000;

/// Persist a probe result. `raw_response_snippet` is truncated to 2000 chars.
pub async fn insert(pool: &PgPool, result: NewProbeResult) -> Result<ProbeResult, sqlx::Error> {
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
    .fetch_one(pool)
    .await
}

/// Most recent probe results for an alert point, newest first.
pub async fn list_recent(
    pool: &PgPool,
    alert_point_id: Uuid,
    limit: i64,
) -> Result<Vec<ProbeResult>, sqlx::Error> {
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
    .fetch_all(pool)
    .await
}

/// Graph points (timestamp/latency/is_up only) for an alert point within the
/// last `hours`, oldest first. Capped at `limit` rows (oldest kept) to bound
/// payload size for long windows.
pub async fn list_since(
    pool: &PgPool,
    alert_point_id: Uuid,
    hours: i32,
    limit: i64,
) -> Result<Vec<ProbeHistoryPoint>, sqlx::Error> {
    sqlx::query_as!(
        ProbeHistoryPoint,
        r#"
        SELECT
            timestamp,
            response_time_ms,
            is_up
        FROM (
            SELECT id, timestamp, response_time_ms, is_up
            FROM probe_results
            WHERE alert_point_id = $1
              AND timestamp > now() - make_interval(hours => $2)
            ORDER BY timestamp DESC, id DESC
            LIMIT $3
        ) recent
        ORDER BY timestamp ASC, id ASC
        "#,
        alert_point_id,
        hours,
        limit
    )
    .fetch_all(pool)
    .await
}

/// Failed probes (`is_up = false`) for an alert point within the last `hours`,
/// newest first, capped at `limit` rows. Feeds `GET /{id}/incidents` and the
/// CSV/JSON export.
pub async fn list_failed_since(
    pool: &PgPool,
    alert_point_id: Uuid,
    hours: i32,
    limit: i64,
) -> Result<Vec<ProbeIncident>, sqlx::Error> {
    sqlx::query_as!(
        ProbeIncident,
        r#"
        SELECT
            timestamp,
            response_time_ms,
            status_code,
            is_up,
            error_message
        FROM probe_results
        WHERE alert_point_id = $1
          AND is_up = false
          AND timestamp > now() - make_interval(hours => $2)
        ORDER BY timestamp DESC, id DESC
        LIMIT $3
        "#,
        alert_point_id,
        hours,
        limit
    )
    .fetch_all(pool)
    .await
}

/// Uptime percentage (0.0–100.0) for an alert point within the last `hours`.
/// Returns `NULL` if there are no probe results in that window.
pub async fn uptime_percentage(
    pool: &PgPool,
    alert_point_id: Uuid,
    hours: i32,
) -> Result<Option<f64>, sqlx::Error> {
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
    .fetch_one(pool)
    .await
}
