pub mod handlers;
pub mod schema;
use crate::handlers::{
    generic::static_handler, sentinel_ws::ws_sentinel_handler, websockets::ws_handler,
};
use anyhow::Context;
use backend::api::{router as api_router, ApiState};
use backend::config::Config;
use backend::queue::ProbeQueue;
use backend::{db, queue};

use axum::extract::ws::Message;
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

/// State of the legacy demo WebSocket routes.
///
/// Database/queue pools live in `backend::api::ApiState` since task 2.1 and
/// are consumed by the `/api/v1` REST routes (and later scheduler/worker).
#[derive(Debug, Clone)]
pub struct AppState {
    broadcast_tx: Arc<Mutex<Sender<Message>>>,
    /// Kept for future fan-in usage; currently receivers subscribe directly.
    #[allow(dead_code)]
    broadcast_rx: Arc<Mutex<Receiver<Message>>>,
    active_alerts: Arc<Mutex<Vec<SentinelAlert>>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env().context("invalid configuration")?;
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
        .context("could not connect to database")?;
    db::run_migrations(&db_pool)
        .await
        .context("database migrations failed")?;
    tracing::info!("database connected and migrations up to date");

    // Valkey/Redis queue connection + health check.
    // Phase 1 keeps the server booting without a reachable Valkey — the queue
    // is not consumed yet; the scheduler/worker (Phase 2) will make it mandatory.
    let redis_pool = queue::connect(&config.redis_url).context("could not create redis pool")?;
    let probe_queue = queue::RedisProbeQueue::new(redis_pool);
    match probe_queue.health_check().await {
        Ok(()) => tracing::info!("valkey/redis reachable at {}", config.redis_url),
        Err(e) => tracing::warn!(
            "valkey/redis health check failed ({}): {e} — continuing without queue",
            config.redis_url
        ),
    }

    // API state: trait-object wiring over PostgreSQL + Valkey pools for the
    // /api/v1 REST routes (see docs/tests.md — tests substitute in-memory fakes).
    let api_state = ApiState {
        alert_points: Arc::new(db::repo::PgAlertPointRepository::new(db_pool.clone())),
        probe_results: Arc::new(db::repo::PgProbeResultStore::new(db_pool.clone())),
        active_alerts: Arc::new(db::repo::PgActiveAlertStore::new(db_pool)),
        queue: Arc::new(probe_queue),
    };

    // share state (demo WebSocket routes; cleanup planned for task 2.4)
    let (tx, rx) = broadcast::channel(32);
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
    let ws_state = AppState {
        broadcast_tx: Arc::new(Mutex::new(tx)),
        broadcast_rx: Arc::new(Mutex::new(rx)),
        active_alerts: Arc::new(Mutex::new(active_alerts)),
    };

    // build our application: REST API + WebSocket demo routes + static files
    let api_router = api_router().with_state(api_state);
    let ws_router = Router::new()
        .route("/ws", get(ws_handler))
        .route("/ws/sentinel", get(ws_sentinel_handler))
        .with_state(ws_state);

    let app = Router::new()
        .merge(api_router)
        .merge(ws_router)
        .fallback(static_handler)
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // bind using SERVER_HOST/SERVER_PORT from the environment
    let listener = tokio::net::TcpListener::bind((config.server_host.as_str(), config.server_port))
        .await
        .context("could not bind server address")?;
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("server error")?;
    Ok(())
}
