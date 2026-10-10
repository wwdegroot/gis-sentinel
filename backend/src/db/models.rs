use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// GIS service type of a monitored endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum ServiceType {
    Wms,
    Wfs,
    Wmts,
    Oaf,
    #[sqlx(rename = "ArcGIS_REST")]
    #[serde(rename = "ArcGIS_REST")]
    ArcGisRest,
    Http,
}

impl ServiceType {
    /// The `TEXT` representation stored in PostgreSQL / used in query filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wms => "WMS",
            Self::Wfs => "WFS",
            Self::Wmts => "WMTS",
            Self::Oaf => "OAF",
            Self::ArcGisRest => "ArcGIS_REST",
            Self::Http => "HTTP",
        }
    }
}

/// HTTP method used when probing a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
}

/// Lifecycle state of an alert event.
/// Stored in PostgreSQL as `New`, `Update`, `Remove` (see migration 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum AlertType {
    #[sqlx(rename = "New")]
    New,
    #[sqlx(rename = "Update")]
    Update,
    #[sqlx(rename = "Remove")]
    Remove,
}

/// Evaluated health status of a monitoring target.
/// Stored in PostgreSQL as `HEALTHY`/`DEGRADED`/`DOWN` (migration 0003);
/// serialized to clients as `healthy`/`degraded`/`down`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "UPPERCASE")]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Healthy,
    Degraded,
    Down,
}

/// User role (migration 0004). Stored as `admin`/`viewer`; serialized to clients
/// as `admin`/`viewer`. `viewer` is read-only on the API (GET routes only);
/// `admin` may also use POST sub-actions (create/update/delete/test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    /// True when `self` satisfies a route requiring `required`.
    /// `admin` satisfies both; `viewer` satisfies only `viewer`.
    pub fn at_least(self, required: Role) -> bool {
        match (self, required) {
            (Role::Admin, _) => true,
            (Role::Viewer, Role::Viewer) => true,
            (Role::Viewer, Role::Admin) => false,
        }
    }
}

/// A local user account (session-cookie auth, Option B).
///
/// `password_hash` holds an argon2 PHC string; never serialize it to clients.
/// The reverse-proxy/SSO path (Option D) reuses this table by upserting rows
/// keyed on `email` from a trusted `X-Authenticated-User` header.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: i64,
    pub email: String,
    /// argon2 PHC string — never sent to clients (excluded from API responses
    /// via the `UserPublic` projection in `handlers::auth_api`).
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A configured monitoring target.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AlertPoint {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub service_type: ServiceType,
    pub check_interval_seconds: i32,
    pub expected_response_time_ms: i32,
    pub http_method: HttpMethod,
    pub custom_headers: Value,
    pub auth_config: Option<Value>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When this target was last enqueued for probing (scheduler bookkeeping,
    /// task 2.2). `NULL` = never probed.
    pub last_checked_at: Option<DateTime<Utc>>,
}

/// Payload for creating a new `AlertPoint`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewAlertPoint {
    pub name: String,
    pub url: String,
    pub service_type: ServiceType,
    pub check_interval_seconds: i32,
    pub expected_response_time_ms: i32,
    #[serde(default)]
    pub http_method: HttpMethod,
    #[serde(default)]
    pub custom_headers: Value,
    #[serde(default)]
    pub auth_config: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Payload for partially updating an `AlertPoint` (`None` = leave unchanged).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateAlertPoint {
    pub name: Option<String>,
    pub url: Option<String>,
    pub service_type: Option<ServiceType>,
    pub check_interval_seconds: Option<i32>,
    pub expected_response_time_ms: Option<i32>,
    pub http_method: Option<HttpMethod>,
    pub custom_headers: Option<Value>,
    pub auth_config: Option<Option<Value>>,
    pub enabled: Option<bool>,
}

/// A single probe execution result.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProbeResult {
    pub id: i64,
    pub alert_point_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub response_time_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub is_up: bool,
    pub error_message: Option<String>,
    pub raw_response_snippet: Option<String>,
}

/// Payload for persisting a new `ProbeResult`.
#[derive(Debug, Clone)]
pub struct NewProbeResult {
    pub alert_point_id: Uuid,
    pub response_time_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub is_up: bool,
    pub error_message: Option<String>,
    /// Truncated to 2000 characters before insert to bound row size.
    pub raw_response_snippet: Option<String>,
}

/// An alert currently raised against a monitoring target.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ActiveAlert {
    pub id: i64,
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,
    pub status: ServiceStatus,
    pub reason: String,
    pub triggered_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Payload for raising a new alert.
#[derive(Debug, Clone)]
pub struct NewActiveAlert {
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,
    pub status: ServiceStatus,
    pub reason: String,
}

/// An open alert joined with its monitoring target's metadata — the shape
/// WebSocket snapshot-on-connect is built from (task 2.4).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OpenAlertWithPoint {
    // active_alerts columns
    pub id: i64,
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,
    pub status: ServiceStatus,
    pub reason: String,
    pub triggered_at: DateTime<Utc>,
    // joined alert_points columns
    pub name: String,
    pub url: String,
    pub service_type: ServiceType,
    pub expected_response_time_ms: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_at_least() {
        assert!(Role::Admin.at_least(Role::Admin));
        assert!(Role::Admin.at_least(Role::Viewer));
        assert!(Role::Viewer.at_least(Role::Viewer));
        assert!(!Role::Viewer.at_least(Role::Admin));
    }

    #[test]
    fn role_serde_is_lowercase() {
        assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), "\"admin\"");
        assert_eq!(serde_json::to_string(&Role::Viewer).unwrap(), "\"viewer\"");
        let r: Role = serde_json::from_str("\"viewer\"").unwrap();
        assert_eq!(r, Role::Viewer);
    }
}
