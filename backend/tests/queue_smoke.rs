//! Valkey/Redis queue-layer smoke test, requires a live Valkey/Redis pointed
//! at by `REDIS_URL` and is run explicitly:
//! `cargo test --features smoke-tests -- --ignored queue_smoke`
//!
//! This is the source of truth that the redis implementation matches the
//! in-memory fake's semantics (see `docs/tests.md`).

use backend::db::models::{HttpMethod, ServiceType};
use backend::queue::{self, ProbeJob};
use std::collections::HashMap;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires REDIS_URL pointing at a reachable Valkey/Redis instance"]
async fn queue_round_trip() {
    dotenvy::dotenv().ok();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let pool = backend::queue::connect(&redis_url).expect("could not create redis pool");
    let queue = backend::queue::RedisProbeQueue::new(pool);

    // health check
    queue.health_check().await.expect("PING failed");

    // clean slate, then enqueue/dequeue a serde round trip
    queue.clear_queue().await.unwrap();
    assert_eq!(queue.queue_len().await.unwrap(), 0);

    let job = ProbeJob {
        target_id: Uuid::now_v7(),
        target_name: "smoke-test".to_string(),
        url: "https://example.com/wms?SERVICE=WMS&REQUEST=GetCapabilities".to_string(),
        service_type: ServiceType::Wms,
        expected_time_ms: 500,
        timeout_ms: 2000,
        http_method: HttpMethod::Get,
        custom_headers: HashMap::from([("X-Api-Key".to_string(), "k".to_string())]),
        auth: None,
    };
    queue.enqueue_probe_job(&job).await.unwrap();
    assert_eq!(queue.queue_len().await.unwrap(), 1);

    let got = queue
        .dequeue_probe_job(Duration::from_secs(1))
        .await
        .unwrap()
        .expect("expected one pending job");
    assert_eq!(got, job);

    // drained: blocking dequeue must time out with None
    let empty = queue
        .dequeue_probe_job(Duration::from_secs(1))
        .await
        .unwrap();
    assert!(empty.is_none());

    queue.clear_queue().await.unwrap();
}
