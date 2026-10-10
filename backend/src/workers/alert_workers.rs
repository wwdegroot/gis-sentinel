use axum::extract::ws::Message;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, info};

use crate::schema::{AlertType, SentinelAlert};
use crate::AppState;

pub async fn start_alert_generator(app: AppState) {
    let mut interval = interval(Duration::from_secs(5));
    let shutdown_token = app.shutdown_token.clone();

    let demo_alerts = vec![
        SentinelAlert {
            id: "1".to_string(),
            name: "test1".to_string(),
            atype: AlertType::New,
            performance: 300,
            expected: 150,
            up: true,
            reason: "slow performance".to_string(),
            error: None,
        },
        SentinelAlert {
            id: "2".to_string(),
            name: "test2".to_string(),
            atype: AlertType::New,
            performance: 150,
            expected: 150,
            up: false,
            reason: "service down".to_string(),
            error: None,
        },
    ];

    // Populate the shared centralized vector state
    {
        let mut active_alerts = app.active_alerts.write().await;
        *active_alerts = demo_alerts.clone();
    }

    // Immediately broadcast these initial 'New' alerts to any currently listening sockets
    for alert in demo_alerts {
        match serde_json::to_string(&alert) {
            Ok(json) => {
                let _ = app.broadcast_tx.send(Message::Text(json.into()));
            }
            Err(e) => error!("Failed to serialize initial alert: {}", e),
        }
    }

    info!("Alert generator background task initialized successfully.");

    // periodic test data
    let mut tick_counter = 0;

    loop {
        tokio::select! {
            _ = shutdown_token.cancelled() => {
                info!("Shutdown signal received. Stopping background alert generator.");
                break;
            }
            _ = interval.tick() => {
                tick_counter += 1;

                // Simulate performance metrics shifting dynamically
                let new_perf_1 = 300 + (tick_counter % 3) * 15;
                let new_reason_1 = format!("slow performance (latency: {}ms)", new_perf_1);

                // Create a clone of the updated alert data to broadcast outside the lock
                let mut updated_alert_to_broadcast = None;

                // 1. Lock the shared state to update the master record
                {
                    let mut active_alerts = app.active_alerts.write().await;
                    if let Some(alert) = active_alerts.iter_mut().find(|a| a.id == "1") {
                        alert.performance = new_perf_1;
                        alert.reason = new_reason_1;

                        // Keep the master record inside State marked as Update or New depending on your requirements.
                        // Here we keep it as Update so future connections or systems know it's a running delta.
                        alert.atype = AlertType::Update;

                        updated_alert_to_broadcast = Some(alert.clone());
                        // Active Alerts are always New, so new clients automatically display them
                        alert.atype = AlertType::New;
                    }
                }

                // 2. Broadcast the update out to the cluster
                if let Some(alert) = updated_alert_to_broadcast {
                    match serde_json::to_string(&alert) {
                        Ok(json) => {
                            debug!("Broadcasting periodic alert update to clients...");
                            let _ = app.broadcast_tx.send(Message::Text(json.into()));
                        }
                        Err(e) => error!("Failed to serialize periodic update alert: {}", e),
                    }
                }
            }
        }
    }
}
