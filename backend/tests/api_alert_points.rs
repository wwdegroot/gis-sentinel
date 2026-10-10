//! REST API integration tests over the in-memory fakes (see `docs/tests.md`).
//!
//! Builds the real `/api/v1` router (`api::router()`) with an `ApiState`
//! wired to the in-memory repository/queue fakes — no PostgreSQL/Valkey
//! required. Run with `cargo test --features test-utils`.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use backend::api::{router, ApiState};
use backend::queue::FakeProbeQueue;

// ---------------------------------------------------------------------------
// test harness
// ---------------------------------------------------------------------------

/// Bundle of fakes so assertions can inspect the stores directly. The three
/// repo fakes share one backing state (mirroring a single PostgreSQL DB).
struct Harness {
    stores: std::sync::Arc<backend::db::repo::FakeStores>,
    queue: FakeProbeQueue,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            stores: std::sync::Arc::new(backend::db::repo::FakeStores::new()),
            queue: FakeProbeQueue::new(),
        }
    }
}

impl Harness {
    fn api_state(&self) -> ApiState {
        let stores = self.stores.clone();
        ApiState {
            alert_points: std::sync::Arc::new(stores.alert_points.clone()),
            probe_results: std::sync::Arc::new(stores.probe_results.clone()),
            active_alerts: std::sync::Arc::new(stores.active_alerts.clone()),
            queue: std::sync::Arc::new(self.queue.clone()),
        }
    }

    fn stores(&self) -> &backend::db::repo::FakeStores {
        &self.stores
    }

    fn app(&self) -> Router {
        router().with_state(self.api_state())
    }

    /// Send a JSON request; returns `(status, parsed body)`.
    async fn send(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        let req = match body {
            Some(v) => req.body(Body::from(v.to_string())).unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self
            .app()
            .oneshot(req)
            .await
            .expect("router service should not error");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("response body must be JSON")
        };
        (status, value)
    }

    /// Valid `POST /alert-points` payload.
    fn valid_create() -> Value {
        json!({
            "name": "GeoServer Demo",
            "url": "https://example.com/geoserver/wms?SERVICE=WMS&REQUEST=GetCapabilities",
            "service_type": "WMS",
            "check_interval_seconds": 30,
            "expected_response_time_ms": 500,
            "http_method": "GET",
            "custom_headers": {"X-Api-Key": "secret"},
            "enabled": true
        })
    }
}

// ---------------------------------------------------------------------------
// CRUD happy paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_returns_201_and_persists() {
    let h = Harness::default();

    let (status, body) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");

    assert_eq!(body["name"], "GeoServer Demo");
    assert_eq!(body["service_type"], "WMS");
    assert!(body["id"].is_string());
    assert_eq!(body["enabled"], true);
    assert_eq!(body["custom_headers"]["X-Api-Key"], "secret");

    // persisted in the store behind the API
    assert_eq!(h.stores().alert_points.len(), 1);
}

#[tokio::test]
async fn get_returns_detail_with_empty_history() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, detail) = h
        .send(Method::GET, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["alert_point"]["id"], created["id"]);
    assert_eq!(detail["recent_results"], json!([]));
    assert_eq!(detail["uptime_24h_pct"], Value::Null);
    assert_eq!(detail["open_alert"], Value::Null);
}

#[tokio::test]
async fn get_unknown_id_is_404() {
    let h = Harness::default();
    let (status, body) = h
        .send(
            Method::GET,
            "/api/v1/alert-points/00000000-0000-0000-0000-000000000000",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn update_applies_partial_changes() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, updated) = h
        .send(
            Method::PUT,
            &format!("/api/v1/alert-points/{id}"),
            Some(json!({
                "name": "Renamed",
                "check_interval_seconds": 60,
                "auth_config": null
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body: {updated}");
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["check_interval_seconds"], 60);
    assert_eq!(updated["auth_config"], Value::Null);
    // untouched fields keep their values
    assert_eq!(updated["url"], created["url"]);
    assert_eq!(updated["expected_response_time_ms"], 500);
}

#[tokio::test]
async fn delete_returns_204_then_404() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .send(Method::DELETE, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::Null);
    assert_eq!(h.stores().alert_points.len(), 0);

    // second delete: gone
    let (status, body) = h
        .send(Method::DELETE, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn full_lifecycle_round_trip() {
    let h = Harness::default();

    // create
    let (status, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();

    // update
    let (status, _) = h
        .send(
            Method::PUT,
            &format!("/api/v1/alert-points/{id}"),
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // list: disabled point hidden by enabled=true filter
    let (status, listed) = h
        .send(Method::GET, "/api/v1/alert-points?enabled=true", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["pagination"]["total"], 0);

    // re-enable, then list shows it
    let _ = h
        .send(
            Method::PUT,
            &format!("/api/v1/alert-points/{id}"),
            Some(json!({"enabled": true})),
        )
        .await;
    let (status, listed) = h.send(Method::GET, "/api/v1/alert-points", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["pagination"]["total"], 1);

    // delete
    let (status, _) = h
        .send(Method::DELETE, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = h
        .send(Method::GET, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// listing: pagination, filtering, search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_reports_pagination_metadata_and_filtering() {
    let h = Harness::default();
    for (name, st) in [("alpha", "WMS"), ("beta", "WMS"), ("gamma", "WFS")] {
        let mut body = Harness::valid_create();
        body["name"] = json!(name);
        body["service_type"] = json!(st);
        let (status, _) = h.send(Method::POST, "/api/v1/alert-points", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // page 1 with page_size 2
    let (status, page1) = h
        .send(Method::GET, "/api/v1/alert-points?page=1&page_size=2", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page1["pagination"]["page"], 1);
    assert_eq!(page1["pagination"]["page_size"], 2);
    assert_eq!(page1["pagination"]["total"], 3);
    assert_eq!(page1["pagination"]["total_pages"], 2);
    assert_eq!(page1["data"].as_array().unwrap().len(), 2);

    // page 2
    let (_, page2) = h
        .send(Method::GET, "/api/v1/alert-points?page=2&page_size=2", None)
        .await;
    assert_eq!(page2["data"].as_array().unwrap().len(), 1);

    // filter by service type
    let (_, wfs) = h
        .send(Method::GET, "/api/v1/alert-points?service_type=WFS", None)
        .await;
    assert_eq!(wfs["pagination"]["total"], 1);
    assert_eq!(wfs["data"][0]["name"], "gamma");

    // search
    let (_, found) = h.send(Method::GET, "/api/v1/alert-points?search=alp", None).await;
    assert_eq!(found["pagination"]["total"], 1);
    assert_eq!(found["data"][0]["name"], "alpha");

    // search that matches nothing
    let (_, none) = h.send(Method::GET, "/api/v1/alert-points?search=zzz", None).await;
    assert_eq!(none["pagination"]["total"], 0);
    assert_eq!(none["data"], json!([]));
}

#[tokio::test]
async fn list_clamps_out_of_range_pagination_params() {
    let h = Harness::default();
    // page 0 and oversized page_size must not crash (clamped to 1 / 100)
    let (status, listed) = h
        .send(Method::GET, "/api/v1/alert-points?page=0&page_size=999", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["pagination"]["page"], 1);
    assert_eq!(listed["pagination"]["page_size"], 100);
}

// ---------------------------------------------------------------------------
// validation & error envelope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_with_validation_errors_returns_400_with_details() {
    let h = Harness::default();

    let mut body = Harness::valid_create();
    body["name"] = json!("   ");
    body["url"] = json!("ftp://example.com/wms");
    body["check_interval_seconds"] = json!(0);

    let (status, body) = h.send(Method::POST, "/api/v1/alert-points", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "validation");
    let details = body["error"]["details"]
        .as_object()
        .expect("details object");
    assert!(details.contains_key("name"), "details: {:?}", details);
    assert!(details.contains_key("url"));
    assert!(details.contains_key("check_interval_seconds"));
    // nothing persisted
    assert_eq!(h.stores().alert_points.len(), 0);
}

#[tokio::test]
async fn create_with_non_object_custom_headers_is_rejected() {
    let h = Harness::default();

    let mut body = Harness::valid_create();
    body["custom_headers"] = json!({"X-Token": 42});

    let (status, body) = h.send(Method::POST, "/api/v1/alert-points", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body["error"]["details"]["custom_headers"].is_null());
}

#[tokio::test]
async fn invalid_json_body_is_400_bad_request() {
    let h = Harness::default();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/alert-points")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let res = h.app().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "bad_request");
}

#[tokio::test]
async fn update_with_invalid_partial_fields_is_400() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .send(
            Method::PUT,
            &format!("/api/v1/alert-points/{id}"),
            Some(json!({"expected_response_time_ms": 0})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body["error"]["details"]["expected_response_time_ms"].is_null());
}

#[tokio::test]
async fn update_unknown_id_is_404() {
    let h = Harness::default();
    let (status, body) = h
        .send(
            Method::PUT,
            "/api/v1/alert-points/00000000-0000-0000-0000-000000000000",
            Some(json!({"name": "nope"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

// ---------------------------------------------------------------------------
// on-demand probe job (`POST /alert-points/{id}/test`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_endpoint_enqueues_probe_job() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .send(Method::POST, &format!("/api/v1/alert-points/{id}/test"), None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["queued"], true);

    let job = &body["job"];
    assert_eq!(job["target_id"], created["id"]);
    assert_eq!(job["url"], created["url"]);
    assert_eq!(job["service_type"], "WMS");
    assert_eq!(job["expected_time_ms"], 500);

    // the job really sits in the queue, serialized as ProbeJob
    let jobs = h.queue.jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_id.to_string(), id);
    assert_eq!(jobs[0].timeout_ms, 10_000); // DEFAULT_PROBE_TIMEOUT_MS
}

#[tokio::test]
async fn test_endpoint_unknown_id_is_404() {
    let h = Harness::default();
    let (status, _) = h
        .send(
            Method::POST,
            "/api/v1/alert-points/00000000-0000-0000-0000-000000000000/test",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(h.queue.jobs().len(), 0);
}

#[tokio::test]
async fn test_endpoint_returns_503_when_queue_is_down() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap().to_string();

    h.queue.fail_next(backend::queue::QueueError::Other(
        "connection refused".into(),
    ));

    let (status, body) = h
        .send(Method::POST, &format!("/api/v1/alert-points/{id}/test"), None)
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "queue_unavailable");
}

// ---------------------------------------------------------------------------
// repository failure injection → 500 path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repository_failure_returns_500_database_error() {
    let h = Harness::default();
    h.stores()
        .alert_points
        .fail_next(backend::db::repo::RepoError::Other("injected".into()));

    let (status, body) = h.send(Method::GET, "/api/v1/alert-points", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"]["code"], "database_error");
}

// ---------------------------------------------------------------------------
// history: probe results + alerts render into the detail endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn detail_renders_probe_history_and_open_alert() {
    let h = Harness::default();
    let (_, created) = h
        .send(Method::POST, "/api/v1/alert-points", Some(Harness::valid_create()))
        .await;
    let id = created["id"].as_str().unwrap();
    let point_id = uuid::Uuid::parse_str(id).unwrap();

    // seed history through the fakes directly (worker does this in production)
    use backend::db::models::{AlertType, NewActiveAlert, NewProbeResult};
    use backend::db::repo::traits::{ActiveAlertStore, ProbeResultStore};
    h.stores()
        .probe_results
        .insert(NewProbeResult {
            alert_point_id: point_id,
            response_time_ms: Some(120),
            status_code: Some(200),
            is_up: true,
            error_message: None,
            raw_response_snippet: Some("ok".to_string()),
        })
        .await
        .unwrap();
    h.stores()
        .active_alerts
        .insert(NewActiveAlert {
            alert_point_id: point_id,
            alert_type: AlertType::Update,
            reason: "latency exceeded".to_string(),
        })
        .await
        .unwrap();

    let (status, detail) = h
        .send(Method::GET, &format!("/api/v1/alert-points/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["recent_results"].as_array().unwrap().len(), 1);
    assert_eq!(detail["uptime_24h_pct"], 100.0);
    assert_eq!(detail["open_alert"]["alert_type"], "Update");
    assert_eq!(detail["open_alert"]["reason"], "latency exceeded");
}
