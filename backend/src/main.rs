//! Example websocket server.
//!
//! Run the server with
//! ```not_rust
//! cargo run -p example-websockets --bin example-websockets
//! ```
//!
//! Run a browser client with
//! ```not_rust
//! firefox http://localhost:3000
//! ```
//!
//! Alternatively you can run the rust client (showing two
//! concurrent websocket connections being established) with
//! ```not_rust
//! cargo run -p example-websockets --bin example-client
//! ```
pub mod handlers;
pub mod schema;
use crate::handlers::{
    generic::static_handler,
    websockets::ws_handler,
    sentinel_ws::ws_sentinel_handler,
};

use axum::extract::ws::{Message};
use axum::routing::{get, Router};
use schema::{AlertType, SentinelAlert};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{self, Receiver, Sender},
    Mutex,
};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone)]
pub struct AppState {
    broadcast_tx: Arc<Mutex<Sender<Message>>>,
    broadcast_rx: Arc<Mutex<Receiver<Message>>>,
    active_alerts: Arc<Mutex<Vec<SentinelAlert>>>,

}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    // share state
    let (tx, rx) = broadcast::channel(32);
    let active_alerts: Vec<SentinelAlert> = vec![
        SentinelAlert{ id: "1".to_string(), atype: AlertType::New, name: "test1".to_string(), performance: 300, expected: 150, up: true, reason: "slow performance".to_string(), error: None},
        SentinelAlert{ id: "2".to_string(), atype: AlertType::New, name: "test2".to_string(), performance: 150, expected: 150, up: false, reason: "service down".to_string(), error: None},
        ];
    let app = AppState {
        broadcast_tx: Arc::new(Mutex::new(tx)),
        broadcast_rx: Arc::new(Mutex::new(rx)),
        active_alerts: Arc::new(Mutex::new(active_alerts)),
    };
    // build our application with some routes and serve static files
    let app = Router::new()
        .fallback(static_handler)
        .route("/ws", get(ws_handler))
        .route("/ws/sentinel", get(ws_sentinel_handler)).with_state(app)
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // run it with hyper
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
