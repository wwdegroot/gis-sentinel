//! In-memory fake of the [`ProbeQueue`] trait (see `docs/tests.md` §1.3).
//!
//! Emulates the Redis list semantics used by the API and smoke tests:
//! `LPUSH` + `BRPOP` (FIFO), `LLEN`, `DEL` (clear) and blocking dequeue with
//! timeout returning `None`. Gated behind `test-utils` (or `test`) so
//! production builds carry no test code.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::queue::{ProbeJob, ProbeQueue, QueueError};
/// In-memory [`ProbeQueue`]. Cloneable; clones share state.
///
/// Test hooks:
/// - [`FakeProbeQueue::fail_next`] — next operation returns the given error.
/// - [`FakeProbeQueue::jobs`] — snapshot of pending jobs for assertions.
#[derive(Debug, Clone, Default)]
pub struct FakeProbeQueue {
    state: Arc<Mutex<VecDeque<ProbeJob>>>,
    fail_next: Arc<Mutex<Option<QueueError>>>,
}

impl FakeProbeQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm the failure-injection hook: the next operation fails with `error`.
    pub fn fail_next(&self, error: QueueError) {
        *self.fail_next.lock().expect("fail_next poisoned") = Some(error);
    }

    fn take_failure(&self) -> Result<(), QueueError> {
        match self.fail_next.lock().expect("fail_next poisoned").take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Snapshot of the currently pending jobs in dequeue (BRPOP) order
    /// (FIFO from the producer's perspective), for test assertions.
    pub fn jobs(&self) -> Vec<ProbeJob> {
        self.state
            .lock()
            .expect("queue state poisoned")
            .iter()
            .rev()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl ProbeQueue for FakeProbeQueue {
    async fn health_check(&self) -> Result<(), QueueError> {
        self.take_failure()
    }

    async fn enqueue_probe_job(&self, job: &ProbeJob) -> Result<u64, QueueError> {
        self.take_failure()?;
        let mut state = self.state.lock().expect("queue state poisoned");
        state.push_front(job.clone()); // LPUSH = left end
        Ok(state.len() as u64)
    }

    async fn dequeue_probe_job(&self, timeout: Duration) -> Result<Option<ProbeJob>, QueueError> {
        self.take_failure()?;
        // BRPOP = right end, blocking with timeout
        let job = tokio::time::timeout(timeout, async {
            loop {
                {
                    let mut state = self.state.lock().expect("queue state poisoned");
                    if let Some(job) = state.pop_back() {
                        return Some(job);
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or(None);
        Ok(job)
    }

    async fn queue_len(&self) -> Result<u64, QueueError> {
        self.take_failure()?;
        Ok(self.state.lock().expect("queue state poisoned").len() as u64)
    }

    async fn clear_queue(&self) -> Result<(), QueueError> {
        self.take_failure()?;
        self.state.lock().expect("queue state poisoned").clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::ServiceType;
    use std::collections::HashMap;
    use crate::queue::PROBE_QUEUE_KEY;
    use uuid::Uuid;

    fn job(target: u8) -> ProbeJob {
        ProbeJob {
            target_id: Uuid::now_v7(),
            target_name: format!("target-{target}"),
            url: format!("https://target-{target}.example.com/wms"),
            service_type: ServiceType::Wms,
            expected_time_ms: 500,
            timeout_ms: 2000,
            http_method: crate::db::models::HttpMethod::Get,
            custom_headers: HashMap::new(),
            auth: None,
        }
    }

    #[tokio::test]
    async fn serde_round_trip_of_probe_job() {
        let j = job(1);
        let json = serde_json::to_string(&j).unwrap();
        assert_eq!(serde_json::from_str::<ProbeJob>(&json).unwrap(), j);
    }

    #[test]
    fn queue_key_is_stable() {
        // wire contract: workers/scheduler depend on this key
        assert_eq!(PROBE_QUEUE_KEY, "sentinel:probe_jobs");
    }

    #[tokio::test]
    async fn enqueue_dequeue_is_fifo() {
        let queue = FakeProbeQueue::new();
        let a = job(1);
        let b = job(2);

        queue.enqueue_probe_job(&a).await.unwrap();
        queue.enqueue_probe_job(&b).await.unwrap();
        assert_eq!(queue.queue_len().await.unwrap(), 2);
        // jobs() reports pending jobs in dequeue (BRPOP) order → FIFO
        assert_eq!(queue.jobs(), vec![a.clone(), b.clone()]);

        // BRPOP takes from the right → FIFO order
        let first = queue
            .dequeue_probe_job(Duration::from_millis(100))
            .await
            .unwrap()
            .expect("first job");
        assert_eq!(first, a);
        let second = queue
            .dequeue_probe_job(Duration::from_millis(100))
            .await
            .unwrap()
            .expect("second job");
        assert_eq!(second, b);
    }

    #[tokio::test]
    async fn dequeue_times_out_with_none_on_empty_queue() {
        let queue = FakeProbeQueue::new();
        let got = queue
            .dequeue_probe_job(Duration::from_millis(100))
            .await
            .unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn clear_empties_the_queue() {
        let queue = FakeProbeQueue::new();
        queue.enqueue_probe_job(&job(1)).await.unwrap();
        queue.enqueue_probe_job(&job(2)).await.unwrap();
        queue.clear_queue().await.unwrap();
        assert_eq!(queue.queue_len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn failure_injection_is_one_shot() {
        let queue = FakeProbeQueue::new();
        queue.fail_next(QueueError::Other("injected".to_string()));

        assert!(matches!(
            queue.health_check().await,
            Err(QueueError::Other(_))
        ));
        // next call works again
        assert!(queue.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn health_check_ok_by_default() {
        let queue = FakeProbeQueue::new();
        assert!(queue.health_check().await.is_ok());
    }
}
