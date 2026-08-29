pub mod handlers;
pub mod schema;
use crate::handlers::{
    generic::static_handler, sentinel_ws::ws_sentinel_handler, websockets::ws_handler,
};
use backend::config::Config;
use backend::db;
use backend::queue;

use axum::extract::ws::Message;
use axum::routing::{get, Router};
use schema::{AlertType, SentinelAlert};
use sqlx::PgPool;
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
    /// Kept for future fan-in usage; currently receivers subscribe directly.
    #[allow(dead_code)]
    broadcast_rx: Arc<Mutex<Receiver<Message>>>,
    active_alerts: Arc<Mutex<Vec<SentinelAlert>>>,
    /// PostgreSQL connection pool, shared across handlers/services.
    /// (Currently unused by routes; consumed by services from Phase 2 onwards.)
    #[allow(dead_code)]
    db: PgPool,
    /// Valkey/Redis connection pool for the probe job queue.
    /// (Currently unused by routes; consumed by the scheduler/worker in Phase 2.)
    #[allow(dead_code)]
    redis: queue::RedisPool,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let config = Config::from_env().unwrap_or_else(|e| panic!("invalid configuration: {e}"));
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
    let app = AppState {
        broadcast_tx: Arc::new(Mutex::new(tx)),
        broadcast_rx: Arc::new(Mutex::new(rx)),
        active_alerts: Arc::new(Mutex::new(active_alerts)),
        db: db_pool,
        redis: redis_pool,
    };
    // build our application with some routes and serve static files
    let app = Router::new()
        .fallback(static_handler)
        .route("/ws", get(ws_handler))
        .route("/ws/sentinel", get(ws_sentinel_handler))
        .with_state(app)
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        );

    // bind using SERVER_HOST/SERVER_PORT from the environment
    let listener = tokio::net::TcpListener::bind((config.server_host.as_str(), config.server_port))
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
