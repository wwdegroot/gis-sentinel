//! Probe worker — the queue consumer (task 2.3).
//!
//! Each worker blocks on `BRPOP` for [`ProbeJob`]s, executes the probe
//! ([`probe::client::run_job_probe`]), applies GIS service-level checks
//! ([`probe::gis::check_service_response`]), evaluates the per-target state
//! machine ([`probe::evaluate::Evaluator`]), persists the probe result,
//! performs the alert DB action (raise/update/resolve), fans the resulting
//! [`AlertEvent`] out to the WebSocket broadcast channel, and finally
//! releases the scheduler's in-flight gate.
//!
//! Alert state (failure streaks) is in-memory per worker; with the current
//! single-process deployment this is correct. Multi-instance deployments will
//! move the streaks to Redis (noted tech debt, Phase 4).

use std::time::Duration;

use axum::extract::ws::Message;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::db::models::{AlertType, NewProbeResult};
use crate::db::repo;
use crate::probe::client::{build_probe_client, run_job_probe};
use crate::probe::evaluate::{AlertAction, Evaluation};
use crate::probe::gis;
use crate::queue::{self, ProbeJob};
use crate::ws_protocol::AlertEvent;
use crate::AppState;

/// Entry point; spawn one instance per configured worker concurrency.
pub async fn run_worker(app: AppState, worker_id: usize) {
    let client = build_probe_client();

    info!(worker_id, "probe worker started");

    loop {
        tokio::select! {
            _ = app.shutdown_token.cancelled() => {
                info!(worker_id, "shutdown signal received. Stopping probe worker.");
                break;
            }
            job = queue::dequeue_probe_job(&app.redis, Duration::from_secs(5)) => {
                match job {
                    Ok(Some(job)) => process_job(&app, &client, worker_id, job).await,
                    Ok(None) => {
                        // BRPOP timeout with an empty queue; re-arm quietly.
                        continue;
                    }
                    Err(e) => {
                        error!(worker_id, error = %e, "queue dequeue failed; backing off");
                        sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

/// Execute one probe job end-to-end. Errors inside are logged; the job is
/// always acknowledged (in-flight gate released) so a poisoned target cannot
/// wedge the pipeline.
async fn process_job(app: &AppState, client: &reqwest::Client, worker_id: usize, job: ProbeJob) {
    debug!(worker_id, target_id = %job.target_id, "processing probe job");

    // 0. Skip jobs whose target was deleted or disabled after being enqueued
    //    (e.g. a job still sitting in the queue when its point is removed).
    //    Probing would either insert an orphaned probe result (FK violation)
    //    or raise alerts for a point the user explicitly turned off.
    match repo::alert_points::get(&app.db, job.target_id).await {
        Ok(point) if point.enabled => {}
        Ok(_) => {
            debug!(target_id = %job.target_id, "skipping job: point disabled");
            app.evaluator.lock().await.reset(job.target_id);
            release_inflight(app, &job).await;
            return;
        }
        Err(sqlx::Error::RowNotFound) => {
            debug!(target_id = %job.target_id, "skipping job: point deleted");
            app.evaluator.lock().await.reset(job.target_id);
            release_inflight(app, &job).await;
            return;
        }
        Err(e) => {
            error!(target_id = %job.target_id, error = %e, "failed to load point for job; skipping");
            release_inflight(app, &job).await;
            return;
        }
    }

    // 1. Transport probe.
    let observation = run_job_probe(client, &job).await;

    // 2. Service-level check (only meaningful when the transport succeeded
    //    and we have a body to inspect).
    let service_error = if observation.is_up {
        gis::check_service_response(
            job.service_type,
            observation.raw_response_snippet.as_deref().unwrap_or(""),
        )
        .err()
    } else {
        None
    };

    // 3. Evaluate the state machine (shared across workers; the lock is held
    //    only for the synchronous evaluate() call).
    let evaluation = {
        let mut evaluator = app.evaluator.lock().await;
        evaluator.evaluate(&job, &observation, service_error.as_deref())
    };

    // 4. Persist the probe result (every probe, healthy or not).
    if let Err(e) = repo::probe_results::insert(
        &app.db,
        NewProbeResult {
            alert_point_id: job.target_id,
            response_time_ms: observation.response_time_ms,
            status_code: observation.status_code,
            is_up: observation.is_up,
            error_message: observation.error_message.clone(),
            raw_response_snippet: observation.raw_response_snippet.clone(),
        },
    )
    .await
    {
        error!(target_id = %job.target_id, error = %e, "failed to persist probe result");
    }

    // 5. Alert store action + WS fan-out.
    match &evaluation.action {
        AlertAction::None => {}
        AlertAction::RaiseNew => {
            match repo::active_alerts::insert(
                &app.db,
                crate::db::models::NewActiveAlert {
                    alert_point_id: job.target_id,
                    alert_type: AlertType::New,
                    status: evaluation.status,
                    reason: evaluation.reason.clone(),
                },
            )
            .await
            {
                Ok(alert) => broadcast_event(app, &job, &evaluation, &observation, alert.id).await,
                Err(e) if is_unique_violation(&e) => {
                    // Defensive: an open alert already exists (e.g. another
                    // worker raced ahead of the shared state machine). Fall
                    // back to updating it instead of failing the probe.
                    warn!(target_id = %job.target_id, "alert already open; downgrading raise to update");
                    match repo::active_alerts::update_open(
                        &app.db,
                        job.target_id,
                        AlertType::Update,
                        evaluation.status,
                        evaluation.reason.clone(),
                    )
                    .await
                    {
                        Ok(Some(alert)) => {
                            broadcast_event(app, &job, &evaluation, &observation, alert.id).await
                        }
                        _ => {
                            error!(target_id = %job.target_id, "failed to update existing alert after race")
                        }
                    }
                }
                Err(e) => error!(target_id = %job.target_id, error = %e, "failed to raise alert"),
            }
        }
        AlertAction::UpdateOpen => {
            match repo::active_alerts::update_open(
                &app.db,
                job.target_id,
                AlertType::Update,
                evaluation.status,
                evaluation.reason.clone(),
            )
            .await
            {
                Ok(Some(alert)) => {
                    broadcast_event(app, &job, &evaluation, &observation, alert.id).await
                }
                Ok(None) => {
                    // Alert resolved concurrently; nothing to update.
                    debug!(target_id = %job.target_id, "update skipped: no open alert");
                }
                Err(e) => error!(target_id = %job.target_id, error = %e, "failed to update alert"),
            }
        }
        AlertAction::Resolve => {
            // Read the open alert's id first so the Remove event can carry it.
            let alert_id = match repo::active_alerts::latest_open(&app.db, job.target_id).await {
                Ok(Some(alert)) => Some(alert.id),
                Ok(None) => None,
                Err(e) => {
                    error!(target_id = %job.target_id, error = %e, "failed to read open alert");
                    None
                }
            };
            match repo::active_alerts::resolve(&app.db, job.target_id).await {
                Ok(_) => {
                    if let Some(id) = alert_id {
                        broadcast_event(app, &job, &evaluation, &observation, id).await;
                    }
                }
                Err(e) => error!(target_id = %job.target_id, error = %e, "failed to resolve alert"),
            }
        }
    }

    // 6. Always release the in-flight gate so the scheduler can re-enqueue.
    release_inflight(app, &job).await;
}

/// Release the scheduler's in-flight gate for a job's target.
async fn release_inflight(app: &AppState, job: &ProbeJob) {
    if let Err(e) = queue::release_inflight(&app.redis, job.target_id).await {
        warn!(target_id = %job.target_id, error = %e, "failed to release in-flight gate (TTL will recover)");
    }
}

/// True when a sqlx error is a unique-constraint violation.
fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Database(db) if db.kind() == sqlx::error::ErrorKind::UniqueViolation
    )
}

/// Publish an [`AlertEvent`] to the WS broadcast channel. Redis Pub/Sub
/// fan-out is the documented multi-instance upgrade path (task 2.4).
async fn broadcast_event(
    app: &AppState,
    job: &ProbeJob,
    evaluation: &Evaluation,
    observation: &crate::probe::client::ProbeObservation,
    alert_id: i64,
) {
    let event = AlertEvent {
        alert_id,
        alert_point_id: job.target_id,
        alert_type: evaluation.alert_type,
        name: job.target_name.clone(),
        url: job.url.clone(),
        service_type: job.service_type,
        status: evaluation.status,
        reason: evaluation.reason.clone(),
        response_time_ms: observation.response_time_ms,
        expected_response_time_ms: job.expected_time_ms,
        triggered_at: chrono::Utc::now(),
    };

    match serde_json::to_string(&crate::ws_protocol::WsMessage::Alert(event)) {
        Ok(json) => {
            info!(
                target_id = %job.target_id,
                alert_id,
                alert_type = ?evaluation.alert_type,
                status = ?evaluation.status,
                reason = %evaluation.reason,
                "alert event broadcast"
            );
            let _ = app.broadcast_tx.send(Message::Text(json.into()));
        }
        Err(e) => error!(target_id = %job.target_id, error = %e, "failed to serialize alert event"),
    }
}
