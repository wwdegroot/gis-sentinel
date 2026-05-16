use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use axum_extra::TypedHeader;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tokio::sync::broadcast::error::RecvError;
//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use tracing::{debug, error, info};
//allows to split the websocket stream into separate TX and RX branches
use crate::AppState;
use futures::{sink::SinkExt, stream::StreamExt};

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

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_sentinel_socket(socket: WebSocket, who: SocketAddr, State(app): State<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = app.broadcast_tx.subscribe();
    let shutdown_token = app.shutdown_token.clone();

    // -- Write message to channel
    // This will temporarily block readers until the write is finished
    // let mut alerts = app.active_alerts.write().await;
    // alerts.push(new_alert);
    //
    // sent active alerts to client
    for alert in app.active_alerts.read().await.iter() {
        let data = serde_json::to_string(alert).expect("Valid SentinelAlert data");
        debug!("Data to be sent= {}", data);
        if let Err(e) = sender.send(Message::Text(data.into())).await {
            error!("Error sending message: {}", e);
        }
    }

    // Spawn a task that will push notifications to the client (does not matter what client does)
    let shutdown_token_send = shutdown_token.clone();
    let mut send_task = tokio::spawn(async move {
        // check for new notifications and sent them
        loop {
            tokio::select! {
                // Exit on shutdown token
                _ = shutdown_token_send.cancelled() => {
                    debug!("Shutdown signal received in send task for {who}");
                    break;
                }

                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            if let Err(e) = sender.send(msg).await {
                                error!("Error sending message: {}", e);
                                break;
                            }
                        }
                        Err(RecvError::Closed) => {
                            debug!("Channel closed, exiting send task");
                            break;
                        }
                        Err(RecvError::Lagged(n)) => {
                            error!("Lagged behind by {} messages", n);
                            // Handle the lag, perhaps by requesting a full state update.
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
    // Clone shutdown_flag for use in the select! loops
    let shutdown_token_recv = shutdown_token.clone();
    // Spawn a task to handle incoming messages from the client and echo them back.
    let mut recv_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                // Exit on shutdown token
                _ = shutdown_token_recv.cancelled() => {
                    debug!("Shutdown signal received in send task for {who}");
                    break;
                }
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(t))) => {
                            info!("Client sent: {}", t);
                            if let Err(e) = app
                                .broadcast_tx
                                .send(format!("Echo: {}", t).into())
                            {
                                error!("Could not send message to broadcast channel: {}", e);
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            debug!("Client closed connection");
                            break;
                        }
                        _ => {}
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
