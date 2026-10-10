//! Valkey/Redis queue infrastructure (task 1.2).
//!
//! Probe jobs produced by the scheduler (task 2.2) are pushed onto a list
//! (`LPUSH`) and consumed by workers (task 2.3) via `BRPOP`, which also acts
//! as the "no duplicate in-flight checks" gate once combined with per-target
//! bookkeeping. Alert lifecycle events will be fanned out on a Pub/Sub channel.
//!
//! Connection pooling uses `deadpool-redis`; a `Pool` hands out connections
//! on demand and transparently re-creates broken ones.
//!
//! The [`ProbeQueue`] trait decouples consumers (API handlers, future
//! scheduler/worker) from Redis so tests can substitute the in-memory fake
//! from [`queue::fake`] (see `docs/tests.md`).

use crate::db::models::ServiceType;
use async_trait::async_trait;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[cfg(any(test, feature = "test-utils"))]
pub mod fake;
#[cfg(any(test, feature = "test-utils"))]
pub use fake::FakeProbeQueue;

/// Redis list key holding serialized [`ProbeJob`] payloads awaiting a worker.
pub const PROBE_QUEUE_KEY: &str = "sentinel:probe_jobs";

/// Redis Pub/Sub channel for alert lifecycle events (`New`/`Update`/`Remove`).
pub const ALERTS_PUBSUB_CHANNEL: &str = "sentinel:alerts";

/// Default maximum number of pooled connections.
const POOL_MAX_SIZE: usize = 10;

/// A pooled Valkey/Redis connection pool.
pub type RedisPool = deadpool_redis::Pool;

/// Errors surfaced by the queue layer.
///
/// `#[from]` wires proper `source()` chaining so downstream consumers
/// (e.g. `ApiError::QueueUnavailable`) can log the inner cause.
#[derive(Debug, Error)]
pub enum QueueError {
    /// Connection acquisition or command execution failed.
    #[error("redis queue error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    /// A dequeued payload could not be deserialized.
    #[error("invalid job payload: {0}")]
    Serde(#[from] serde_json::Error),
    /// Failure raised by a test fake via its failure-injection hook
    /// (or any backend error that does not fit the other variants).
    #[error("queue failure: {0}")]
    Other(String),
}

// Manual: thiserror cannot do transitive From (RedisError -> PoolError -> QueueError).
impl From<redis::RedisError> for QueueError {
    fn from(e: redis::RedisError) -> Self {
        QueueError::Pool(e.into())
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

/// Queue operations needed by producers (API on-demand checks, scheduler)
/// and consumers (workers).
#[async_trait]
pub trait ProbeQueue: Send + Sync + std::fmt::Debug {
    /// Verify the server is reachable (`PING` → `PONG`).
    async fn health_check(&self) -> Result<(), QueueError>;

    /// Push a probe job onto the queue (left end, so `BRPOP` yields FIFO order).
    async fn enqueue_probe_job(&self, job: &ProbeJob) -> Result<u64, QueueError>;

    /// Block up to `timeout` waiting for the next probe job.
    /// Returns `None` on timeout without a pending job.
    async fn dequeue_probe_job(&self, timeout: Duration) -> Result<Option<ProbeJob>, QueueError>;

    /// Number of probe jobs currently waiting in the queue.
    async fn queue_len(&self) -> Result<u64, QueueError>;

    /// Remove all pending probe jobs (used by tests / maintenance).
    async fn clear_queue(&self) -> Result<(), QueueError>;
}

/// Create a pooled Valkey/Redis connection pool.
///
/// Connections are established lazily; use `health_check` to verify
/// reachability.
pub fn connect(redis_url: &str) -> Result<RedisPool, deadpool_redis::CreatePoolError> {
    let mut cfg = deadpool_redis::Config::from_url(redis_url);
    cfg.pool = Some(deadpool_redis::PoolConfig::new(POOL_MAX_SIZE));
    cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
}

/// Redis-backed [`ProbeQueue`] implementation.
#[derive(Debug, Clone)]
pub struct RedisProbeQueue {
    pool: RedisPool,
}

impl RedisProbeQueue {
    pub fn new(pool: RedisPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProbeQueue for RedisProbeQueue {
    async fn health_check(&self) -> Result<(), QueueError> {
        let mut conn = self.pool.get().await?;
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

    async fn enqueue_probe_job(&self, job: &ProbeJob) -> Result<u64, QueueError> {
        let payload = serde_json::to_string(job)?;
        let mut conn = self.pool.get().await?;
        Ok(conn.lpush(PROBE_QUEUE_KEY, payload).await?)
    }

    async fn dequeue_probe_job(&self, timeout: Duration) -> Result<Option<ProbeJob>, QueueError> {
        let mut conn = self.pool.get().await?;
        // BRPOP answers with a (queue_key, payload) pair
        let raw: Option<(String, String)> =
            conn.brpop(PROBE_QUEUE_KEY, timeout.as_secs_f64()).await?;
        match raw {
            Some((_, payload)) => Ok(Some(serde_json::from_str(&payload)?)),
            None => Ok(None),
        }
    }

    async fn queue_len(&self) -> Result<u64, QueueError> {
        let mut conn = self.pool.get().await?;
        Ok(conn.llen(PROBE_QUEUE_KEY).await?)
    }

    async fn clear_queue(&self) -> Result<(), QueueError> {
        let mut conn = self.pool.get().await?;
        conn.del::<_, ()>(PROBE_QUEUE_KEY).await?;
        Ok(())
    }
}
