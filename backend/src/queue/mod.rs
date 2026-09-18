//! Valkey/Redis queue infrastructure (task 1.2).
//!
//! Probe jobs produced by the scheduler (task 2.2) are pushed onto a list
//! (`LPUSH`) and consumed by workers (task 2.3) via `BRPOP`, which also acts
//! as the "no duplicate in-flight checks" gate once combined with per-target
//! bookkeeping. Alert lifecycle events will be fanned out on a Pub/Sub channel.
//!
//! Connection pooling uses `deadpool-redis`; a `Pool` hands out connections
//! on demand and transparently re-creates broken ones.

use crate::db::models::ServiceType;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

/// Redis list key holding serialized [`ProbeJob`] payloads awaiting a worker.
pub const PROBE_QUEUE_KEY: &str = "sentinel:probe_jobs";

/// Redis Pub/Sub channel for alert lifecycle events (`New`/`Update`/`Remove`).
pub const ALERTS_PUBSUB_CHANNEL: &str = "sentinel:alerts";

/// Default maximum number of pooled connections.
const POOL_MAX_SIZE: usize = 10;

/// A pooled Valkey/Redis connection pool.
pub type RedisPool = deadpool_redis::Pool;

/// Errors surfaced by the queue layer.
#[derive(Debug)]
pub enum QueueError {
    /// Connection acquisition or command execution failed.
    Pool(deadpool_redis::PoolError),
    /// A dequeued payload could not be deserialized.
    Serde(serde_json::Error),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::Pool(e) => write!(f, "redis queue error: {e}"),
            QueueError::Serde(e) => write!(f, "invalid job payload: {e}"),
        }
    }
}

impl std::error::Error for QueueError {}

impl From<deadpool_redis::PoolError> for QueueError {
    fn from(e: deadpool_redis::PoolError) -> Self {
        QueueError::Pool(e)
    }
}

impl From<redis::RedisError> for QueueError {
    fn from(e: redis::RedisError) -> Self {
        QueueError::Pool(e.into())
    }
}

impl From<serde_json::Error> for QueueError {
    fn from(e: serde_json::Error) -> Self {
        QueueError::Serde(e)
    }
}

/// A single probe job as serialized onto the queue.
///
/// This is the wire contract between the scheduler (producer, task 2.2) and
/// the worker (consumer, task 2.3); keep field names stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeJob {
    /// `alert_points.id` of the monitoring target this job belongs to.
    pub target_id: Uuid,
    /// Endpoint URL to probe.
    pub url: String,
    /// GIS service type, drives probe strategy in the worker.
    pub service_type: ServiceType,
    /// Configured latency budget in milliseconds.
    pub expected_time_ms: i32,
    /// Probe request timeout in milliseconds.
    pub timeout_ms: i32,
}

/// Create a pooled Valkey/Redis connection pool.
///
/// Connections are established lazily; use [`health_check`] to verify
/// reachability.
pub fn connect(redis_url: &str) -> Result<RedisPool, deadpool_redis::CreatePoolError> {
    let mut cfg = deadpool_redis::Config::from_url(redis_url);
    cfg.pool = Some(deadpool_redis::PoolConfig::new(POOL_MAX_SIZE));
    cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
}

/// Verify the server is reachable (`PING` → `PONG`).
pub async fn health_check(pool: &RedisPool) -> Result<(), QueueError> {
    let mut conn = pool.get().await?;
    let pong: String = redis::cmd("PING").query_async(&mut conn).await?;
    if pong != "PONG" {
        return Err(redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "unexpected PING reply",
        ))
        .into());
    }
    Ok(())
}

/// Push a probe job onto the queue (left end, so `BRPOP` yields FIFO order).
pub async fn enqueue_probe_job(pool: &RedisPool, job: &ProbeJob) -> Result<u64, QueueError> {
    let payload = serde_json::to_string(job)?;
    let mut conn = pool.get().await?;
    Ok(conn.lpush(PROBE_QUEUE_KEY, payload).await?)
}

/// Block up to `timeout` waiting for the next probe job.
/// Returns `None` on timeout without a pending job.
pub async fn dequeue_probe_job(
    pool: &RedisPool,
    timeout: Duration,
) -> Result<Option<ProbeJob>, QueueError> {
    let mut conn = pool.get().await?;
    // BRPOP answers with a (queue_key, payload) pair
    let raw: Option<(String, String)> = conn.brpop(PROBE_QUEUE_KEY, timeout.as_secs_f64()).await?;
    match raw {
        Some((_, payload)) => Ok(Some(serde_json::from_str(&payload)?)),
        None => Ok(None),
    }
}

/// Number of probe jobs currently waiting in the queue.
pub async fn queue_len(pool: &RedisPool) -> Result<u64, QueueError> {
    let mut conn = pool.get().await?;
    Ok(conn.llen(PROBE_QUEUE_KEY).await?)
}

/// Remove all pending probe jobs (used by tests / maintenance).
pub async fn clear_queue(pool: &RedisPool) -> Result<(), QueueError> {
    let mut conn = pool.get().await?;
    conn.del::<_, ()>(PROBE_QUEUE_KEY).await?;
    Ok(())
}
