//! DB-layer smoke test, requires a live database pointed at by `DATABASE_URL`
//! and is run explicitly:
//! `cargo test --features smoke-tests -- --ignored db_smoke`
//!
//! This is the source of truth that the sqlx implementations match the
//! in-memory fakes' semantics (see `docs/tests.md`).

use backend::db;
use backend::db::models::{
    HttpMethod, NewActiveAlert, NewAlertPoint, NewProbeResult, ServiceType, UpdateAlertPoint,
};
use backend::db::repo::traits::{ActiveAlertStore, AlertPointRepository, ProbeResultStore};
use backend::db::repo::PgActiveAlertStore;
use backend::db::repo::PgAlertPointRepository;
use backend::db::repo::PgProbeResultStore;

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at a migrated database"]
async fn repository_layer_round_trip() {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = db::connect(&database_url).await.unwrap();
    db::run_migrations(&pool).await.unwrap();

    let alert_points = PgAlertPointRepository::new(pool.clone());
    let probe_results = PgProbeResultStore::new(pool.clone());
    let active_alerts = PgActiveAlertStore::new(pool);

    // create
    let point = alert_points
        .create(NewAlertPoint {
            name: "smoke-test".to_string(),
            url: "https://example.com/wms?SERVICE=WMS&REQUEST=GetCapabilities".to_string(),
            service_type: ServiceType::Wms,
            check_interval_seconds: 30,
            expected_response_time_ms: 500,
            http_method: HttpMethod::Get,
            custom_headers: serde_json::json!({"X-Api-Key": "secret"}),
            auth_config: None,
            enabled: true,
        })
        .await
        .unwrap();

    // read back
    let fetched = alert_points.get(point.id).await.unwrap();
    assert_eq!(fetched.name, "smoke-test");
    let list = alert_points.list(true, 10, 0).await.unwrap();
    assert!(list.iter().any(|p| p.id == point.id));

    // partial update incl. setting auth_config from NULL to a value
    let updated = alert_points
        .update(
            point.id,
            UpdateAlertPoint {
                enabled: Some(false),
                auth_config: Some(Some(serde_json::json!({"kind": "basic", "user": "u"}))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!updated.enabled);
    assert!(updated.auth_config.is_some());

    // probe result with snippet truncation
    let probe = probe_results
        .insert(NewProbeResult {
            alert_point_id: point.id,
            response_time_ms: Some(123),
            status_code: Some(200),
            is_up: true,
            error_message: None,
            raw_response_snippet: Some("x".repeat(5000)),
        })
        .await
        .unwrap();
    assert_eq!(probe.raw_response_snippet.as_deref().unwrap().len(), 2000);
    let recent = probe_results.list_recent(point.id, 5).await.unwrap();
    assert_eq!(recent.len(), 1);
    let uptime = probe_results.uptime_percentage(point.id, 24).await.unwrap();
    assert_eq!(uptime, Some(100.0));

    // alert lifecycle: one open alert per point is enforced
    active_alerts
        .insert(NewActiveAlert {
            alert_point_id: point.id,
            alert_type: backend::db::AlertType::New,
            reason: "unreachable".to_string(),
        })
        .await
        .unwrap();
    let dup = active_alerts
        .insert(NewActiveAlert {
            alert_point_id: point.id,
            alert_type: backend::db::AlertType::Update,
            reason: "degraded".to_string(),
        })
        .await;
    assert!(
        matches!(dup, Err(backend::db::repo::RepoError::Conflict)),
        "expected conflict for second open alert"
    );
    let open = active_alerts.list_open().await.unwrap();
    assert_eq!(open.len(), 1);
    let resolved = active_alerts.resolve(point.id).await.unwrap();
    assert_eq!(resolved, 1);

    // cascade cleanup
    assert!(alert_points.delete(point.id).await.unwrap());
    assert!(matches!(
        alert_points.get(point.id).await,
        Err(backend::db::repo::RepoError::NotFound)
    ));
    // probe results and alerts must have cascaded
    let recent = probe_results.list_recent(point.id, 5).await.unwrap();
    assert!(recent.is_empty());
    let open = active_alerts.list_open().await.unwrap();
    assert!(open.iter().all(|a| a.alert_point_id != point.id));
}
