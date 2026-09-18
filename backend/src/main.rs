pub mod config;
pub mod handlers;
pub mod probe;
pub mod schema;
pub mod workers;

use crate::config::Config;
use crate::handlers::{
    alert_points_api, generic::static_handler, sentinel_ws::ws_sentinel_handler,
    websockets::ws_handler,
};
use backend::db;
use backend::queue;

use crate::workers::alert_workers;
use anyhow::Result;
use axum::extract::ws::Message;
use axum::routing::{get, Router};
use schema::{AlertType, SentinelAlert};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use tokio::signal;
use tokio::sync::broadcast::{self, Sender};
use tokio_util::sync::CancellationToken;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone)]
pub struct AppState {
    // broadcast::Sender is Send + Sync and all its methods take &self, so a
    // plain Arc suffices for sharing it across handlers/workers. New receivers
    // are created via `subscribe()` (fan-out); re-derive a Receiver on demand
    // if fan-in is ever needed.
    broadcast_tx: Arc<Sender<Message>>,
    active_alerts: Arc<RwLock<Vec<SentinelAlert>>>,
    /// PostgreSQL connection pool, shared across handlers/services.
    /// (Used by the alert-points REST API; queue consumers follow in Phase 2.2/2.3.)
    db: PgPool,
    /// Valkey/Redis connection pool for the probe job queue.
    /// (Currently unused by routes; consumed by the scheduler/worker in Phase 2.)
    #[allow(dead_code)]
    redis: queue::RedisPool,
    shutdown_token: CancellationToken,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config = Config::load();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| config.log_level.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // database connection + migrations
    let db_pool = db::connect(&config.database_url)
        .await
        .unwrap_or_else(|e| panic!("could not connect to database: {e}"));
    db::run_migrations(&db_pool)
        .await
        .unwrap_or_else(|e| panic!("database migrations failed: {e}"));
    tracing::info!("database connected and migrations up to date");

    // Valkey/Redis queue connection + health check.
    // Phase 1 keeps the server booting without a reachable Valkey — the queue
    // is not consumed yet; the scheduler/worker (Phase 2) will make it mandatory.
    let redis_pool = queue::connect(&config.redis_url)
        .unwrap_or_else(|e| panic!("could not create redis pool: {e}"));
    match queue::health_check(&redis_pool).await {
        Ok(()) => tracing::info!("valkey/redis reachable at {}", config.redis_url),
        Err(e) => tracing::warn!(
            "valkey/redis health check failed ({}): {e} — continuing without queue",
            config.redis_url
        ),
    }

    // share state
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

    // Create broadcast channel for WebSocket messages
    let (tx, _rx) = broadcast::channel(32);
    // Cancellation token for graceful shutdown
    let shutdown_token = CancellationToken::new();
    let app_state = AppState {
        broadcast_tx: Arc::new(tx),
        active_alerts: Arc::new(RwLock::new(active_alerts)),
        db: db_pool,
        redis: redis_pool,
        shutdown_token: shutdown_token.clone(),
    };

    tokio::spawn(alert_workers::start_alert_generator(app_state.clone()));

    // Build the application router
    let app = Router::new()
        .fallback(static_handler)
        .route("/ws", get(ws_handler))
        .route("/ws/sentinel", get(ws_sentinel_handler))
        .nest("/api/v1/alert-points", alert_points_api::router())
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
