pub mod config;
pub mod handlers;
pub mod schema;

use crate::config::Config;
use crate::handlers::{
    generic::static_handler, sentinel_ws::ws_sentinel_handler, websockets::ws_handler,
};
use anyhow::Result;
use axum::extract::ws::Message;
use axum::routing::{get, Router};
use schema::{AlertType, SentinelAlert};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{
    broadcast::{self, Receiver, Sender},
    Mutex,
};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone)]
pub struct AppState {
    pub broadcast_tx: Arc<Mutex<Sender<Message>>>,
    pub broadcast_rx: Arc<Mutex<Receiver<Message>>>,
    pub active_alerts: Arc<Mutex<Vec<SentinelAlert>>>,
    pub shutdown_flag: Arc<AtomicBool>,
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

    // Initialize active alerts with demo data
    let active_alerts: Vec<SentinelAlert> = vec![
        SentinelAlert {
            id: "1".to_string(),
            atype: AlertType::New,
            name: "test1".to_string(),
            performance: 300,
            expected: 150,
            up: true,
            reason: "slow performance".to_string(),
            error: None,
        },
        SentinelAlert {
            id: "2".to_string(),
            atype: AlertType::New,
            name: "test2".to_string(),
            performance: 150,
            expected: 150,
            up: false,
            reason: "service down".to_string(),
            error: None,
        },
    ];

    // Create atomic boolean flag for graceful shutdown signaling
    let shutdown_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    let app_state = AppState {
        broadcast_tx: Arc::new(Mutex::new(broadcast_tx)),
        broadcast_rx: Arc::new(Mutex::new(broadcast_rx)),
        active_alerts: Arc::new(Mutex::new(active_alerts)),
        shutdown_flag,
    };

    // Clone shutdown_flag for use in the closure (before moving app_state)
    let shutdown_notifier = app_state.shutdown_flag.clone();

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
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        tracing::info!("Shutdown signal received, notifying WebSocket handlers...");
        // Set the atomic flag so all WebSocket handlers can detect it
        shutdown_notifier.store(true, Ordering::SeqCst);
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
