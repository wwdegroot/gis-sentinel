//! Probe job scheduler — the queue producer (task 2.2).
//!
//! On every tick the scheduler queries enabled alert points whose
//! `check_interval_seconds` has elapsed ([`repo::alert_points::list_due`]),
//! acquires the per-target in-flight gate (`SET NX EX` so a target never has
//! more than one pending/in-flight job), and pushes a [`ProbeJob`] onto the
//! Valkey list consumed by the probe worker (task 2.3).
//!
//! Error policy: failures for individual targets are logged and skipped so a
//! single bad row cannot kill the loop; connection-level failures to Postgres
//! or Valkey abort the tick early and are retried on the next tick.

use std::collections::HashMap;

use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::db::models::AlertPoint;
use crate::queue::{self, ProbeJob};
use crate::AppState;

/// Entry point spawned from `main`; exits when the shutdown token is fired.
pub async fn run_scheduler(app: AppState) {
    let tick = app.config.scheduler_tick();
    let mut ticker = interval(tick);
    // `interval` fires immediately on the first tick; skip it so the very
    // first scheduling pass happens one tick after startup (gives migrations
    // and other startup work time to settle).
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    info!(every_secs = tick.as_secs(), "scheduler started");

    loop {
        tokio::select! {
            _ = app.shutdown_token.cancelled() => {
                info!("shutdown signal received. Stopping scheduler.");
                break;
            }
            _ = ticker.tick() => {
                if let Err(e) = schedule_due(&app).await {
                    warn!(error = %e, "scheduler tick failed; will retry next tick");
                }
            }
        }
    }
}

/// One scheduling pass: find due targets and enqueue probe jobs.
async fn schedule_due(app: &AppState) -> anyhow::Result<()> {
    let due = crate::db::repo::alert_points::list_due(&app.db).await?;
    if due.is_empty() {
        debug!("scheduler: no due targets");
        return Ok(());
    }

    let pending = queue::queue_len(&app.redis).await.unwrap_or(0);
    info!(due = due.len(), pending_jobs = pending, "scheduler tick");

    for point in due {
        if let Err(e) = enqueue_for_target(app, &point).await {
            warn!(
                target_id = %point.id,
                name = %point.name,
                error = %e,
                "failed to enqueue probe job"
            );
        }
    }
    Ok(())
}

/// Convert the configured JSONB headers object into the flat map carried by
/// [`ProbeJob`]. Non-string values are dropped (the REST API validates that
/// all values are strings; this is belt-and-braces).
fn headers_map(custom_headers: &serde_json::Value) -> HashMap<String, String> {
    custom_headers
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Gate + enqueue a single target. Fails soft: any error is returned to the
/// caller for logging and retried on the next tick.
async fn enqueue_for_target(app: &AppState, point: &AlertPoint) -> anyhow::Result<()> {
    // In-flight gate: at most one pending/in-flight job per target. The TTL is
    // the check interval plus slack — it is only a crash-safety net; the
    // worker releases the key explicitly when the probe finishes (task 2.3).
    let ttl_secs = point.check_interval_seconds.max(1) as u64 + 30;
    if !queue::try_acquire_inflight(&app.redis, point.id, ttl_secs).await? {
        debug!(target_id = %point.id, "skipped: probe already in flight");
        return Ok(());
    }

    let job = ProbeJob {
        target_id: point.id,
        target_name: point.name.clone(),
        url: point.url.clone(),
        service_type: point.service_type,
        expected_time_ms: point.expected_response_time_ms,
        timeout_ms: app.config.probe_timeout_ms_default as i32,
        http_method: point.http_method,
        custom_headers: headers_map(&point.custom_headers),
        auth: crate::probe::client::parse_auth_spec(point.auth_config.as_ref()),
    };

    match queue::enqueue_probe_job(&app.redis, &job).await {
        Ok(_) => {
            crate::db::repo::alert_points::touch_last_checked(&app.db, point.id).await?;
            debug!(target_id = %point.id, url = %point.url, "enqueued probe job");
            Ok(())
        }
        Err(e) => {
            // Release the gate so the next tick can retry immediately.
            queue::release_inflight(&app.redis, point.id).await?;
            Err(e.into())
        }
    }
}
