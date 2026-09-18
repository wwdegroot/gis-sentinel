//! Valkey/Redis queue-layer smoke test, requires a live Valkey/Redis pointed
//! at by `REDIS_URL` and is run explicitly:
//! `cargo test -- --ignored queue_smoke`

use backend::db::models::ServiceType;
use backend::queue::{self, ProbeJob};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires REDIS_URL pointing at a reachable Valkey/Redis instance"]
async fn queue_round_trip() {
    dotenvy::dotenv().ok();
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    let pool = queue::connect(&redis_url).expect("could not create redis pool");

    // health check
    queue::health_check(&pool).await.expect("PING failed");

    // clean slate, then enqueue/dequeue a serde round trip
    queue::clear_queue(&pool).await.unwrap();
    assert_eq!(queue::queue_len(&pool).await.unwrap(), 0);

    let job = ProbeJob {
        target_id: Uuid::now_v7(),
        url: "https://example.com/wms?SERVICE=WMS&REQUEST=GetCapabilities".to_string(),
        service_type: ServiceType::Wms,
        expected_time_ms: 500,
        timeout_ms: 2000,
    };
    queue::enqueue_probe_job(&pool, &job).await.unwrap();
    assert_eq!(queue::queue_len(&pool).await.unwrap(), 1);

    let got = queue::dequeue_probe_job(&pool, std::time::Duration::from_secs(1))
        .await
        .unwrap()
        .expect("expected one pending job");
    assert_eq!(got, job);

    // drained: blocking dequeue must time out with None
    let empty = queue::dequeue_probe_job(&pool, std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert!(empty.is_none());

    queue::clear_queue(&pool).await.unwrap();
}
