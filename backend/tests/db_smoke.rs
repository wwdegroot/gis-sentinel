//! DB-layer smoke test, requires a live database pointed at by `DATABASE_URL`
//! and is run explicitly: `cargo test -- --ignored db_smoke`

use backend::db;
use backend::db::models::AlertType as DbAlertType;
use backend::db::models::{
    HttpMethod, NewActiveAlert, NewAlertPoint, NewProbeResult, ServiceType, UpdateAlertPoint,
};
use backend::db::repo;

#[tokio::test]
#[ignore = "requires DATABASE_URL pointing at a migrated database"]
async fn repository_layer_round_trip() {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = db::connect(&database_url).await.unwrap();
    db::run_migrations(&pool).await.unwrap();

    // remove leftovers from aborted earlier runs
    sqlx::query!("DELETE FROM alert_points WHERE name = 'smoke-test'")
        .execute(&pool)
        .await
        .unwrap();

    // create
    let point = repo::alert_points::create(
        &pool,
        NewAlertPoint {
            name: "smoke-test".to_string(),
            url: "https://example.com/wms?SERVICE=WMS&REQUEST=GetCapabilities".to_string(),
            service_type: ServiceType::Wms,
            check_interval_seconds: 30,
            expected_response_time_ms: 500,
            http_method: HttpMethod::Get,
            custom_headers: serde_json::json!({"X-Api-Key": "secret"}),
            auth_config: None,
            enabled: true,
        },
    )
    .await
    .unwrap();

    // read back
    let fetched = repo::alert_points::get(&pool, point.id).await.unwrap();
    assert_eq!(fetched.name, "smoke-test");
    let list = repo::alert_points::list(&pool, true, None, 10, 0)
        .await
        .unwrap();
    assert!(list.iter().any(|p| p.id == point.id));

    // partial update incl. setting auth_config from NULL to a value
    let updated = repo::alert_points::update(
        &pool,
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
    let probe = repo::probe_results::insert(
        &pool,
        NewProbeResult {
            alert_point_id: point.id,
            response_time_ms: Some(123),
            status_code: Some(200),
            is_up: true,
            error_message: None,
            raw_response_snippet: Some("x".repeat(5000)),
        },
    )
    .await
    .unwrap();
    assert_eq!(probe.raw_response_snippet.as_deref().unwrap().len(), 2000);
    let recent = repo::probe_results::list_recent(&pool, point.id, 5)
        .await
        .unwrap();
    assert_eq!(recent.len(), 1);
    let uptime = repo::probe_results::uptime_percentage(&pool, point.id, 24)
        .await
        .unwrap();
    assert_eq!(uptime, Some(100.0));

    // alert lifecycle: one open alert per point is enforced
    repo::active_alerts::insert(
        &pool,
        NewActiveAlert {
            alert_point_id: point.id,
            alert_type: DbAlertType::New,
            reason: "unreachable".to_string(),
        },
    )
    .await
    .unwrap();
    let dup = repo::active_alerts::insert(
        &pool,
        NewActiveAlert {
            alert_point_id: point.id,
            alert_type: DbAlertType::Update,
            reason: "degraded".to_string(),
        },
    )
    .await;
    assert!(
        dup.is_err(),
        "expected unique violation for second open alert"
    );
    let open = repo::active_alerts::list_open(&pool).await.unwrap();
    assert_eq!(open.len(), 1);
    let resolved = repo::active_alerts::resolve(&pool, point.id).await.unwrap();
    assert_eq!(resolved, 1);

    // cascade cleanup
    assert!(repo::alert_points::delete(&pool, point.id).await.unwrap());
    assert!(repo::alert_points::get(&pool, point.id).await.is_err());
    // probe results and alerts must have cascaded
    let recent = repo::probe_results::list_recent(&pool, point.id, 5)
        .await
        .unwrap();
    assert!(recent.is_empty());
    let open = repo::active_alerts::list_open(&pool).await.unwrap();
    assert!(open.iter().all(|a| a.alert_point_id != point.id));
}
