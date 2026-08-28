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
    pub reason: String,
    pub triggered_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Payload for raising a new alert.
#[derive(Debug, Clone)]
pub struct NewActiveAlert {
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,
    pub reason: String,
}
