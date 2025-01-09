use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
};
use axum_extra::TypedHeader;
use tokio::sync::broadcast::error::RecvError;
use std::net::SocketAddr;
//allows to extract the IP of connecting user
use axum::extract::connect_info::ConnectInfo;
use tracing::{debug, error, info};
//allows to split the websocket stream into separate TX and RX branches
use futures::{sink::SinkExt, stream::StreamExt};
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
    println!("`{user_agent}` at {addr} connected.");
    ws.on_upgrade(move |socket| handle_sentinel_socket(socket, addr, State(app)))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_sentinel_socket(socket: WebSocket, who: SocketAddr, State(app): State<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = app.broadcast_tx.lock().await.subscribe();
    
    // sent active alerts to client
    for alert in app.active_alerts.lock().await.iter() {
        let data = serde_json::to_string(alert).expect("Valid SentinelAlert data");
        debug!("Data to be sent= {}", data);
        if let Err(e) = sender.send(Message::Text(data)).await {
            error!("Error sending message: {}", e);
        }
    }

    // Spawn a task that will push notifications to the client (does not matter what client does)
    let mut send_task = tokio::spawn(async move {
        // check for new notifications and sent them
        loop {
            tokio::select! {
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
                else => break
            }
        }
    });
        
    // Spawn a task to handle incoming messages from the client and echo them back.
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(t) => {
                    info!("Client sent: {}", t);
                    if let Err(e) = app.broadcast_tx.lock().await.send(format!("Echo: {}", t).into()) {
                        error!("Could not send message to broadcast channel: {}", e);
                    }
                }
                Message::Close(_) => {
                    debug!("Client closed connection");
                    break;
                }
                _ => {}
            }
        }
        debug!("Receive task finished");
    });

    // If any one of the tasks exit, abort the other.
    tokio::select! {
        _ = (&mut send_task) => {
            debug!("Send task exited");
            recv_task.abort();
        },
        _ = (&mut recv_task) => {
            debug!("Receive task exited");
            send_task.abort();
        }
    };
}
