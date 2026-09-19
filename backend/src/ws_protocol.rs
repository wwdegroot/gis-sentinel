//! Wire contract for real-time alert events (task 2.3 publishes, task 2.4
//! adds the WebSocket `Snapshot`/`Alert` envelope).
//!
//! This replaces the demo-era `schema::SentinelAlert` shape; the frontend
//! (`sentinelSocket`) will be updated in Phase 3 task 3.1.

use crate::db::models::AlertType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::db::models::ServiceStatus;

/// A single alert lifecycle event pushed to WebSocket clients.
///
/// `alert_type` semantics:
/// - `New` — newly triggered outage or degradation
/// - `Update` — status changed within an ongoing incident
/// - `Remove` — incident resolved; clients drop the card (`alert_id` +
///   `alert_point_id` identify what to remove)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub alert_id: i64,
    pub alert_point_id: Uuid,
    pub alert_type: AlertType,
    pub name: String,
    pub url: String,
    pub service_type: crate::db::models::ServiceType,
    pub status: ServiceStatus,
    pub reason: String,
    pub response_time_ms: Option<i32>,
    pub expected_response_time_ms: i32,
    pub triggered_at: DateTime<Utc>,
}

/// Full message envelope for the WebSocket channel (task 2.4 will switch the
/// `sentinel_ws` handler to send these).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    /// Initial state sync sent to every client on connect.
    Snapshot { alerts: Vec<AlertEvent> },
    /// Incremental lifecycle event.
    Alert(AlertEvent),
}
