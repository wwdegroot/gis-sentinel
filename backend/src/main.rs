pub mod config;
pub mod handlers;
pub mod schema;
pub mod workers;

use crate::config::Config;
use crate::handlers::{
    generic::static_handler, sentinel_ws::ws_sentinel_handler, websockets::ws_handler,
};
use crate::workers::alert_workers;
use anyhow::Result;
use axum::extract::ws::Message;
use axum::routing::{get, Router};
use schema::SentinelAlert;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{
    broadcast::{self, Receiver, Sender},
    RwLock,
};
use tokio_util::sync::CancellationToken;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone)]
pub struct AppState {
    pub broadcast_tx: Sender<Message>,
    pub broadcast_rx: Arc<Receiver<Message>>,
    pub active_alerts: Arc<RwLock<Vec<SentinelAlert>>>,
    pub shutdown_token: CancellationToken,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::load();

    // Create broadcast channel for WebSocket messages
    let (broadcast_tx, broadcast_rx) = broadcast::channel(32);

    // Cancellation token for graceful shutdown
    let shutdown_token = CancellationToken::new();
    let app_state = AppState {
        broadcast_tx: broadcast_tx,
        broadcast_rx: Arc::new(broadcast_rx),
        active_alerts: Arc::new(RwLock::new(vec![])),
        shutdown_token: shutdown_token.clone(),
    };

    tokio::spawn(alert_workers::start_alert_generator(app_state.clone()));

    // Build the application router
    let app = Router::new()
        .fallback(static_handler)
        .route("/ws", get(ws_handler))
        .route("/ws/sentinel", get(ws_sentinel_handler))
        .with_state(app_state.clone())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // Start the server with graceful shutdown
    let addr = config.bind_addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    // Create a closure that handles the shutdown signal
    let shutdown_signal = async move {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl+c event");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to listen for terminate signal")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                shutdown_token.cancel();
            },
            _ = terminate => {
                shutdown_token.cancel();
            },
        }

        tracing::info!("Shutdown signal received, notifying WebSocket handlers...");
    };

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal)
    .await?;

    tracing::info!("Server shut down complete.");
    Ok(())
}
