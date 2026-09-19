//! Alert state evaluation (task 2.3): per-target state machine with flap
//! protection.
//!
//! Status semantics:
//! - **Down** — transport failure, HTTP 4xx/5xx, or GIS service-level error.
//!   Only declared after [`PROBE_FAILURE_THRESHOLD`] consecutive failures
//!   (hysteresis against flapping).
//! - **Degraded** — service reachable but latency exceeds the expected SLA.
//! - **Healthy** — reachable within SLA.
//!
//! Recovery from Down/Degraded is immediate on the first successful probe.
//! The evaluator tracks per-target state in memory; with the current
//! single-process worker this is sufficient (multi-instance deployments would
//! move the streak counters to Redis — noted tech debt for Phase 4).

use std::collections::HashMap;

use tracing::debug;
use uuid::Uuid;

use crate::db::models::AlertType;
use crate::probe::client::ProbeObservation;
use crate::queue::ProbeJob;
use crate::ws_protocol::ServiceStatus;

/// What the worker should do with the alert store / WS hub after a probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertAction {
    /// No open alert needed (healthy, or no-op update).
    None,
    /// Raise a new alert (`alert_type` = `New`).
    RaiseNew,
    /// Update the open alert's reason/type (`alert_type` = `Update`).
    UpdateOpen,
    /// Resolve the open alert (`alert_type` = `Remove`).
    Resolve,
}

/// Outcome of evaluating one probe observation against the target's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub status: ServiceStatus,
    pub reason: String,
    pub action: AlertAction,
    /// The `alert_type` to store/publish for this event (`New`/`Update`/`Remove`).
    pub alert_type: AlertType,
}

#[derive(Debug, Default, Clone)]
struct TargetState {
    consecutive_failures: u32,
    last_status: Option<ServiceStatus>,
    last_reason: Option<String>,
}

/// Per-target probe evaluation state machine.
#[derive(Debug)]
pub struct Evaluator {
    failure_threshold: u32,
    states: HashMap<Uuid, TargetState>,
}

impl Evaluator {
    pub fn new(failure_threshold: u32) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            states: HashMap::new(),
        }
    }

    /// Forget a target's streak/history. Used when a service point is deleted
    /// or disabled so stale state cannot leak into a future point with the
    /// same id (ids are UUIDs, but the entry would still leak memory).
    pub fn reset(&mut self, target_id: Uuid) {
        self.states.remove(&target_id);
    }

    /// Evaluate one probe result for `job` and produce the next [`Evaluation`].
    pub fn evaluate(
        &mut self,
        job: &ProbeJob,
        observation: &ProbeObservation,
        service_error: Option<&str>,
    ) -> Evaluation {
        let state = self.states.entry(job.target_id).or_default();

        let failed = !observation.is_up || service_error.is_some();
        if failed {
            state.consecutive_failures += 1;
        } else {
            state.consecutive_failures = 0;
        }

        // Determine the new status.
        let (status, reason) = if failed && state.consecutive_failures >= self.failure_threshold {
            let reason = service_error
                .map(str::to_string)
                .or_else(|| observation.error_message.clone())
                .unwrap_or_else(|| "probe failed".into());
            (ServiceStatus::Down, reason)
        } else if failed {
            // Failing but below threshold: keep previous status (grace), stay
            // quiet about it.
            (
                state.last_status.unwrap_or(ServiceStatus::Healthy),
                state
                    .last_reason
                    .clone()
                    .unwrap_or_else(|| "pending".into()),
            )
        } else {
            match observation.response_time_ms {
                Some(ms) if ms > job.expected_time_ms => (
                    ServiceStatus::Degraded,
                    format!(
                        "latency {} ms exceeded expected {} ms",
                        ms, job.expected_time_ms
                    ),
                ),
                _ => (ServiceStatus::Healthy, "service healthy".into()),
            }
        };

        // Decide the alert action from the previous → new transition.
        let action = match (state.last_status, status) {
            (None, ServiceStatus::Healthy) => AlertAction::None,
            (None, ServiceStatus::Degraded) | (None, ServiceStatus::Down) => AlertAction::RaiseNew,
            (Some(ServiceStatus::Healthy), ServiceStatus::Healthy) => AlertAction::None,
            (Some(ServiceStatus::Healthy), _) => AlertAction::RaiseNew,
            (Some(ServiceStatus::Down | ServiceStatus::Degraded), ServiceStatus::Healthy) => {
                AlertAction::Resolve
            }
            (Some(_), _) if reason != state.last_reason.clone().unwrap_or_default() => {
                AlertAction::UpdateOpen
            }
            (Some(_), _) => AlertAction::None,
        };

        let alert_type = match action {
            AlertAction::None => {
                // Keep the last stored type for bookkeeping; irrelevant when
                // no alert is touched.
                AlertType::Update
            }
            AlertAction::RaiseNew => AlertType::New,
            AlertAction::UpdateOpen => AlertType::Update,
            AlertAction::Resolve => AlertType::Remove,
        };

        debug!(
            target_id = %job.target_id,
            ?status,
            ?action,
            streak = state.consecutive_failures,
            "probe evaluated"
        );

        state.last_status = Some(status);
        state.last_reason = Some(reason.clone());

        Evaluation {
            status,
            reason,
            action,
            alert_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::HttpMethod;

    fn job() -> ProbeJob {
        ProbeJob {
            target_id: uuid::Uuid::nil(),
            target_name: "t".into(),
            url: "https://example.com".into(),
            service_type: crate::db::models::ServiceType::Http,
            expected_time_ms: 500,
            timeout_ms: 5000,
            http_method: HttpMethod::Get,
            custom_headers: HashMap::new(),
            auth: None,
        }
    }

    fn obs(up: bool, ms: Option<i32>, err: Option<&str>) -> ProbeObservation {
        ProbeObservation {
            status_code: Some(200),
            response_time_ms: ms,
            is_up: up,
            error_message: err.map(str::to_string),
            raw_response_snippet: None,
        }
    }

    #[test]
    fn healthy_target_stays_quiet() {
        let job = job();
        let mut ev = Evaluator::new(2);
        let e = ev.evaluate(&job, &obs(true, Some(100), None), None);
        assert_eq!(e.status, ServiceStatus::Healthy);
        assert_eq!(e.action, AlertAction::None);
    }

    #[test]
    fn latency_breach_raises_degraded_then_resolves() {
        let job = job();
        let mut ev = Evaluator::new(2);
        let e = ev.evaluate(&job, &obs(true, Some(900), None), None);
        assert_eq!(e.status, ServiceStatus::Degraded);
        assert_eq!(e.action, AlertAction::RaiseNew);
        assert_eq!(e.alert_type, AlertType::New);
        assert!(e.reason.contains("900 ms"));

        // same measurement again -> reason unchanged -> no-op update
        let e = ev.evaluate(&job, &obs(true, Some(900), None), None);
        assert_eq!(e.action, AlertAction::None);

        // materially different latency -> Update
        let e = ev.evaluate(&job, &obs(true, Some(1400), None), None);
        assert_eq!(e.action, AlertAction::UpdateOpen);
        assert_eq!(e.alert_type, AlertType::Update);

        // recovery
        let e = ev.evaluate(&job, &obs(true, Some(100), None), None);
        assert_eq!(e.status, ServiceStatus::Healthy);
        assert_eq!(e.action, AlertAction::Resolve);
        assert_eq!(e.alert_type, AlertType::Remove);
    }

    #[test]
    fn down_requires_consecutive_failures() {
        let job = job();
        let mut ev = Evaluator::new(2);
        // first failure: grace period, no alert
        let e = ev.evaluate(&job, &obs(false, None, Some("connection failed")), None);
        assert_eq!(e.status, ServiceStatus::Healthy);
        assert_eq!(e.action, AlertAction::None);
        // second consecutive failure: Down
        let e = ev.evaluate(&job, &obs(false, None, Some("connection failed")), None);
        assert_eq!(e.status, ServiceStatus::Down);
        assert_eq!(e.action, AlertAction::RaiseNew);
        assert_eq!(e.alert_type, AlertType::New);
        // third failure, same reason: no-op
        let e = ev.evaluate(&job, &obs(false, None, Some("connection failed")), None);
        assert_eq!(e.action, AlertAction::None);
        // recovery on first success
        let e = ev.evaluate(&job, &obs(true, Some(50), None), None);
        assert_eq!(e.action, AlertAction::Resolve);
    }

    #[test]
    fn single_blip_does_not_alert() {
        let job = job();
        let mut ev = Evaluator::new(2);
        let _ = ev.evaluate(&job, &obs(false, None, Some("timeout")), None);
        let e = ev.evaluate(&job, &obs(true, Some(50), None), None);
        assert_eq!(e.status, ServiceStatus::Healthy);
        assert_eq!(e.action, AlertAction::None);
    }

    #[test]
    fn service_error_counts_as_failure() {
        let job = job();
        let mut ev = Evaluator::new(1);
        let e = ev.evaluate(
            &job,
            &obs(true, Some(50), None),
            Some("service returned a ServiceException"),
        );
        assert_eq!(e.status, ServiceStatus::Down);
        assert_eq!(e.action, AlertAction::RaiseNew);
        assert!(e.reason.contains("ServiceException"));
    }

    #[test]
    fn reason_change_triggers_update() {
        let job = job();
        let mut ev = Evaluator::new(1);
        let _ = ev.evaluate(&job, &obs(false, None, Some("HTTP 500")), None);
        let e = ev.evaluate(&job, &obs(false, None, Some("HTTP 503")), None);
        assert_eq!(e.status, ServiceStatus::Down);
        assert_eq!(e.action, AlertAction::UpdateOpen);
        assert_eq!(e.alert_type, AlertType::Update);
    }
}
