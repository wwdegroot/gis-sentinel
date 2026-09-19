use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use axum_extra::TypedHeader;
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, info};

use crate::db::repo;
use crate::ws_protocol::WsMessage;
use crate::AppState;

pub async fn ws_sentinel_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    State(app): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    debug!("`{user_agent}` at {addr} connected.");
    ws.on_upgrade(move |socket| handle_sentinel_socket(socket, addr, State(app)))
}

/// Actual websocket statemachine (one will be spawned per connection).
///
/// Message flow (task 2.4):
/// 1. On connect, a `Snapshot` of all open alerts (joined from the DB) is
///    sent so the client renders the current state immediately.
/// 2. Afterwards, live `Alert` lifecycle events produced by the probe worker
///    are forwarded from the tokio broadcast channel.
async fn handle_sentinel_socket(socket: WebSocket, who: SocketAddr, State(app): State<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = app.broadcast_tx.subscribe();
    let shutdown_token = app.shutdown_token.clone();

    // -- Initial state synchronization: send the full open-alert list.
    match repo::active_alerts::list_open_with_point(&app.db).await {
        Ok(open) => {
            let alerts = open
                .into_iter()
                .map(|row| crate::ws_protocol::AlertEvent {
                    alert_id: row.id,
                    alert_point_id: row.alert_point_id,
                    alert_type: row.alert_type,
                    name: row.name,
                    url: row.url,
                    service_type: row.service_type,
                    status: row.status,
                    reason: row.reason,
                    response_time_ms: None,
                    expected_response_time_ms: row.expected_response_time_ms,
                    triggered_at: row.triggered_at,
                })
                .collect::<Vec<_>>();
            debug!(client = %who, alerts = alerts.len(), "sending snapshot");
            match serde_json::to_string(&WsMessage::Snapshot { alerts }) {
                Ok(json) => {
                    if let Err(e) = sender.send(Message::Text(json.into())).await {
                        error!("Error sending snapshot to {who}: {e}");
                        return;
                    }
                }
                Err(e) => error!("Failed to serialize snapshot for {who}: {e}"),
            }
        }
        Err(e) => {
            // Snapshot is a nice-to-have; live events still flow. Log and continue.
            error!("Failed to build snapshot for {who}: {e}");
        }
    }

    // -- Write task: forward broadcast events to the client.
    let shutdown_token_send = shutdown_token.clone();
    let mut send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_token_send.cancelled() => {
                    debug!("Shutdown signal received in send task for {who}");
                    break;
                }

                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if let Err(e) = sender.send(msg).await {
                                error!("Error sending message: {e}");
                                break;
                            }
                        }
                        Err(RecvError::Closed) => {
                            debug!("Channel closed, exiting send task");
                            break;
                        }
                        Err(RecvError::Lagged(n)) => {
                            error!("Lagged behind by {n} messages");
                            // The client missed events; request a full state
                            // update via a fresh snapshot.
                            continue;
                        }
                    }
                }
            }
        }

        // Send graceful close before exiting
        if sender
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::NORMAL,
                reason: "Server shutting down".into(),
            })))
            .await
            .is_err()
        {
            debug!("Could not send close frame to {who} during shutdown");
        }
    });

    // -- Read task: track client disconnects. No server-side handling of
    //    client messages is needed yet (an envelope can be added when the
    //    frontend requires it).
    let shutdown_token_recv = shutdown_token.clone();
    let mut recv_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_token_recv.cancelled() => {
                    debug!("Shutdown signal received in recv task for {who}");
                    break;
                }
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(t))) => {
                            debug!("Client {who} sent: {t}");
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            debug!("Client closed connection");
                            break;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            debug!("Websocket error from {who}: {e}");
                            break;
                        }
                    }
                }
            }
        }
        debug!("Receive task finished for {who}");
    });

    // If any one of the tasks exit, abort the other.
    tokio::select! {
        _ = (&mut send_task) => {
            debug!("Send task exited for {who}");
            recv_task.abort();
        },
        _ = (&mut recv_task) => {
            debug!("Receive task exited for {who}");
            send_task.abort();
        }
    };

    info!("Sentinel WebSocket connection closed for {who}");
}
