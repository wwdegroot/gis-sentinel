pub mod auth;
pub mod config;
pub mod handlers;
pub mod probe;
pub mod workers;
pub mod ws_protocol;

use crate::config::Config;
use crate::handlers::{
    alert_points_api, auth_api, generic::static_handler, sentinel_ws::ws_sentinel_handler,
};
use backend::db;
use backend::queue;
use backend::queue::ProbeQueue as _;

use crate::workers::{probe_worker, scheduler};
use anyhow::Context;
use axum::extract::ws::Message;
use axum::routing::{get, Router};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::broadcast::{self, Sender};
use tokio_util::sync::CancellationToken;
use tower_http::request_id::MakeRequestUuid;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// State of the legacy demo WebSocket routes.
///
/// Database/queue pools live in `backend::api::ApiState` since task 2.1 and
/// are consumed by the `/api/v1` REST routes (and later scheduler/worker).
#[derive(Debug, Clone)]
pub struct AppState {
    // broadcast::Sender is Send + Sync and all its methods take &self, so a
    // plain Arc suffices for sharing it across handlers/workers. New receivers
    // are created via `subscribe()` (fan-out); re-derive a Receiver on demand
    // if fan-in is ever needed.
    broadcast_tx: Arc<Sender<Message>>,
    /// PostgreSQL connection pool, shared across handlers/services.
    /// (Used by the REST API, the WS snapshot, and the probe worker.)
    db: PgPool,
    /// Valkey/Redis connection pool for the probe job queue.
    /// (Consumed by the scheduler and the probe worker.)
    redis: queue::RedisPool,
    /// Shared per-target alert state machine (failure streaks, last status).
    /// One instance across all workers so `New` alerts are raised exactly once
    /// regardless of which worker processes a job.
    evaluator: Arc<tokio::sync::Mutex<probe::evaluate::Evaluator>>,
    /// Typed application configuration (scheduler knobs etc.).
    config: Config,
    shutdown_token: CancellationToken,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        .context("could not connect to database")?;
    db::run_migrations(&db_pool)
        .await
        .context("database migrations failed")?;
    tracing::info!("database connected and migrations up to date");

    // Bootstrap admin (Q5): if BOOTSTRAP_ADMIN_EMAIL/BOOTSTRAP_ADMIN_PASSWORD are
    // set, upsert the admin row idempotently on every boot (rotating the env
    // value rotates the password). Without these, no local admin exists until
    // one is provisioned (future CLI / Option D provisioning).
    if let (Some(email), Some(password)) =
        (&config.bootstrap_admin_email, &config.bootstrap_admin_password)
    {
        match auth::password::hash_password(password) {
            Ok(hash) => match db::repo::users::upsert_bootstrap_admin(&db_pool, email, &hash).await {
                Ok(u) => tracing::info!(
                    user_id = u.id,
                    email = %u.email,
                    "bootstrap admin ready (session-cookie auth enabled)"
                ),
                Err(e) => tracing::error!(error = %e, "failed to upsert bootstrap admin"),
            },
            Err(e) => tracing::error!(error = %e, "failed to hash bootstrap admin password"),
        }
    } else {
        tracing::info!("no BOOTSTRAP_ADMIN_* env set; session-cookie auth has no local admin");
    }

    // Valkey/Redis queue connection + health check.
    // Phase 1 keeps the server booting without a reachable Valkey — the queue
    // is not consumed yet; the scheduler/worker (Phase 2) will make it mandatory.
    let redis_pool = queue::connect(&config.redis_url).context("could not create redis pool")?;
    let probe_queue = queue::RedisProbeQueue::new(redis_pool.clone());
    match probe_queue.health_check().await {
        Ok(()) => tracing::info!("valkey/redis reachable at {}", config.redis_url),
        Err(e) => tracing::warn!(
            "valkey/redis health check failed ({}): {e} — continuing without queue",
            config.redis_url
        ),
    }

    // Create broadcast channel for WebSocket messages
    let (tx, _rx) = broadcast::channel(32);
    // Cancellation token for graceful shutdown
    let shutdown_token = CancellationToken::new();
    let app_state = AppState {
        broadcast_tx: Arc::new(tx),
        db: db_pool.clone(),
        redis: redis_pool.clone(),
        evaluator: Arc::new(tokio::sync::Mutex::new(probe::evaluate::Evaluator::new(
            config.probe_failure_threshold,
        ))),
        config: config.clone(),
        shutdown_token: shutdown_token.clone(),
    };

    let mut background_tasks = vec![tokio::spawn(scheduler::run_scheduler(app_state.clone()))];
    for worker_id in 0..app_state.config.worker_concurrency.max(1) {
        background_tasks.push(tokio::spawn(probe_worker::run_worker(
            app_state.clone(),
            worker_id,
        )));
    }

    // Request-ID middleware: every request gets an `x-request-id` (honoring an
    // incoming one) that is echoed on the response and stamped into the trace
    // span, so access logs, handler logs, and client reports can be correlated.
    let request_id_span = |req: &axum::http::Request<_>| {
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        tracing::info_span!(
            "http_request",
            request_id = %request_id,
            method = %req.method(),
            uri = %req.uri().path(),
        )
    };

    // Build the application router.
    //
    // Auth (Option B): auth is enforced per-handler via the `CurrentUser`
    // extractor (401 when absent) — the alert-points handlers, `/auth/me`, and
    // the WS upgrade all declare it. `/auth/{login,logout}` are public. The SPA
    // fallback stays public — the bundle has no secrets, and the frontend gates
    // itself via `/auth/me` (401 → redirect to /login).
    let app = Router::new()
        .fallback(static_handler)
        .route("/ws/sentinel", get(ws_sentinel_handler))
        .nest("/api/v1/alert-points", alert_points_api::router())
        .nest("/api/v1/auth", auth_api::router())
        .with_state(app_state.clone())
        .layer(
            tower::ServiceBuilder::new()
                .layer(tower_http::request_id::SetRequestIdLayer::x_request_id(
                    MakeRequestUuid,
                ))
                .layer(TraceLayer::new_for_http().make_span_with(request_id_span))
                .layer(tower_http::request_id::PropagateRequestIdLayer::x_request_id()),
        );

    // bind using the typed configuration and start the server with graceful
    // shutdown.
    let addr = config.bind_addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    // Create a closure that handles the shutdown signal
    let token_for_signal = shutdown_token.clone();
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
                token_for_signal.cancel();
            },
            _ = terminate => {
                token_for_signal.cancel();
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

    // Ensure background tasks see the token even if the server stopped for a
    // reason other than a signal (idempotent if already cancelled).
    shutdown_token.cancel();

    // Wait for the scheduler/workers to finish their current work (they exit
    // between jobs, never mid-probe). Bound the wait so a stuck probe cannot
    // hang shutdown forever.
    tracing::info!(
        tasks = background_tasks.len(),
        "waiting for background tasks to stop"
    );
    let drain = futures::future::join_all(background_tasks);
    if tokio::time::timeout(std::time::Duration::from_secs(30), drain)
        .await
        .is_err()
    {
        tracing::warn!("background tasks did not stop within 30s; continuing shutdown");
    }

    // Close the connection pools (sqlx waits for idle connections to close).
    db_pool.close().await;
    redis_pool.close();

    tracing::info!("Server shut down complete.");
    Ok(())
}
