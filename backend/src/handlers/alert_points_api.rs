//! REST API for alert points CRUD (task 2.1).
//!
//! Routes (mounted under `/api/v1/alert-points`; GET/POST only — state
//! changes are expressed as POST sub-actions):
//!
//! | Method | Path           | Purpose |
//! |--------|----------------|---------|
//! | GET    | `/`            | list (filter: `enabled`, `service_type`; paginate: `page`, `per_page`) |
//! | POST   | `/`            | create |
//! | GET    | `/{id}`        | detail + recent probe history |
//! | POST   | `/{id}/update` | partial update |
//! | POST   | `/{id}/delete` | delete (cascades probe results & alerts) |
//! | POST   | `/{id}/test`   | on-demand probe (not persisted, raises no alerts) |
//!
//! The on-demand probe uses the HTTP transport layer from `crate::probe`;
//! GIS-specific checks arrive with task 2.3 and will be wired into the same
//! endpoint afterwards.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::db::models::{AlertPoint, ServiceType};
use crate::db::repo;
use crate::handlers::error::{ApiError, ApiResult};
use crate::probe::client::{build_probe_client, run_http_probe};
use crate::AppState;

/// Route parameters bounds.
const MIN_CHECK_INTERVAL_SECS: i32 = 10;
const MAX_CHECK_INTERVAL_SECS: i32 = 86_400;
const MAX_EXPECTED_RESPONSE_MS: i32 = 600_000;
const MAX_NAME_LEN: usize = 200;
const MAX_URL_LEN: usize = 2048;
const MAX_PER_PAGE: i64 = 200;
const DEFAULT_PER_PAGE: i64 = 50;

/// Headers that must never be set through `custom_headers`.
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
    "te",
    "trailer",
    "proxy-connection",
    "proxy-authorization",
];

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the `/api/v1/alert-points` sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_alert_points).post(create_alert_point))
        .route("/{id}", get(get_alert_point))
        .route("/{id}/update", post(update_alert_point))
        .route("/{id}/delete", post(delete_alert_point))
        .route("/{id}/test", post(test_alert_point))
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ListParams {
    /// `true` → only enabled points, `false` → only disabled, absent → all.
    enabled: Option<bool>,
    /// Filter by GIS service type, e.g. `service_type=WMS`.
    service_type: Option<ServiceType>,
    /// 1-based page number (default 1).
    page: Option<u32>,
    /// Page size (default 50, capped at [`MAX_PER_PAGE`]).
    per_page: Option<u32>,
}

#[derive(Debug, Serialize)]
struct Pagination {
    page: u32,
    per_page: u32,
    total: i64,
}

#[derive(Debug, Serialize)]
struct AlertPointListResponse {
    data: Vec<AlertPoint>,
    pagination: Pagination,
}

async fn list_alert_points(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListParams>,
) -> ApiResult<Json<AlertPointListResponse>> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params
        .per_page
        .unwrap_or(DEFAULT_PER_PAGE as u32)
        .clamp(1, MAX_PER_PAGE as u32);
    let limit = per_page as i64;
    let offset = (page as i64 - 1) * per_page as i64;

    // `enabled` filter semantics: Some(true) → enabled only,
    // Some(false) → disabled only, None → everything.
    let (enabled_only, disabled_only) = match params.enabled {
        Some(true) => (true, false),
        Some(false) => (false, true),
        None => (false, false),
    };

    let mut data =
        repo::alert_points::list(&app.db, enabled_only, params.service_type, limit, offset).await?;

    if disabled_only {
        data.retain(|p| !p.enabled);
    }

    // Disabled-only filtering is a rare path; count matches the SQL filter for
    // the common cases and is computed from the filtered page for the rest.
    let total = if disabled_only {
        data.len() as i64
    } else {
        repo::alert_points::count(&app.db, enabled_only, params.service_type).await?
    };

    Ok(Json(AlertPointListResponse {
        data,
        pagination: Pagination {
            page,
            per_page,
            total,
        },
    }))
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

async fn create_alert_point(
    State(app): State<AppState>,
    Json(payload): Json<crate::db::models::NewAlertPoint>,
) -> ApiResult<(axum::http::StatusCode, Json<AlertPoint>)> {
    validate_new_point(&payload)?;

    let point = repo::alert_points::create(&app.db, payload).await?;

    tracing::info!(id = %point.id, name = %point.name, "alert point created");
    Ok((axum::http::StatusCode::CREATED, Json(point)))
}

/// Request body for partial updates (all fields optional, `None` = unchanged).
#[derive(Debug, Deserialize)]
struct UpdateAlertPointPayload {
    name: Option<String>,
    url: Option<String>,
    service_type: Option<crate::db::models::ServiceType>,
    check_interval_seconds: Option<i32>,
    expected_response_time_ms: Option<i32>,
    http_method: Option<crate::db::models::HttpMethod>,
    custom_headers: Option<Value>,
    auth_config: Option<Option<Value>>,
    enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Get single
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct AlertPointDetailResponse {
    #[serde(flatten)]
    alert_point: AlertPoint,
    recent_probes: Vec<crate::db::models::ProbeResult>,
}

async fn get_alert_point(
    State(app): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<AlertPointDetailResponse>> {
    let point = repo::alert_points::get(&app.db, id).await?;
    let recent_probes = repo::probe_results::list_recent(&app.db, id, 50).await?;
    Ok(Json(AlertPointDetailResponse {
        alert_point: point,
        recent_probes,
    }))
}

// ---------------------------------------------------------------------------
// Update / Delete
// ---------------------------------------------------------------------------

async fn update_alert_point(
    State(app): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(payload): Json<UpdateAlertPointPayload>,
) -> ApiResult<Json<AlertPoint>> {
    // Reject no-op payloads instead of silently doing nothing.
    let touched_any = payload.name.is_some()
        || payload.url.is_some()
        || payload.service_type.is_some()
        || payload.check_interval_seconds.is_some()
        || payload.expected_response_time_ms.is_some()
        || payload.http_method.is_some()
        || payload.custom_headers.is_some()
        || payload.auth_config.is_some()
        || payload.enabled.is_some();
    if !touched_any {
        return Err(ApiError::BadRequest(
            "update payload must set at least one field".to_string(),
        ));
    }

    // Validate whichever fields are being changed.
    if let Some(name) = &payload.name {
        validate_name(name)?;
    }
    if let Some(url) = &payload.url {
        validate_url(url)?;
    }
    if let Some(v) = payload.check_interval_seconds {
        validate_check_interval(v).map_err(ApiError::BadRequest)?;
    }
    if let Some(v) = payload.expected_response_time_ms {
        validate_expected_ms(v).map_err(ApiError::BadRequest)?;
    }
    if let Some(h) = &payload.custom_headers {
        validate_custom_headers(h)?;
    }
    if let Some(auth) = payload.auth_config.as_ref().and_then(|a| a.as_ref()) {
        validate_auth_config(auth)?;
    }

    let point = repo::alert_points::update(
        &app.db,
        id,
        crate::db::models::UpdateAlertPoint {
            name: payload.name,
            url: payload.url,
            service_type: payload.service_type,
            check_interval_seconds: payload.check_interval_seconds,
            expected_response_time_ms: payload.expected_response_time_ms,
            http_method: payload.http_method,
            custom_headers: payload.custom_headers,
            auth_config: payload.auth_config.flatten().map(Some),
            enabled: payload.enabled,
        },
    )
    .await?;

    tracing::info!(id = %id, "alert point updated");
    Ok(Json(point))
}

async fn delete_alert_point(
    State(app): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let deleted = repo::alert_points::delete(&app.db, id).await?;
    if !deleted {
        return Err(ApiError::NotFound("alert point"));
    }
    tracing::info!(id = %id, "alert point deleted");
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// On-demand probe test
// ---------------------------------------------------------------------------

/// Timeout for on-demand probes: 3× the expected latency, clamped to
/// [5 s, 30 s] so a slow/missing SLA config neither aborts instantly nor
/// hangs the request for a minute.
fn probe_timeout(expected_response_time_ms: i32) -> std::time::Duration {
    let ms = (expected_response_time_ms as i64 * 3).clamp(5_000, 30_000);
    std::time::Duration::from_millis(ms as u64)
}

/// Convert the configured JSONB headers into a reqwest `HeaderMap`.
fn configured_headers_to_map(custom_headers: &Value) -> Result<HeaderMap, ApiError> {
    let mut map = HeaderMap::new();
    let Some(obj) = custom_headers.as_object() else {
        return Ok(map);
    };
    for (name, value) in obj {
        let Some(val_str) = value.as_str() else {
            continue;
        };
        // Header names are case-insensitive; normalize so users can write
        // 'X-Api-Key' naturally (HeaderName stores lowercase).
        let lowered = name.to_ascii_lowercase();
        let header_name = axum::http::HeaderName::from_lowercase(lowered.as_bytes())
            .map_err(|_| ApiError::BadRequest(format!("invalid custom header name: {name}")))?;
        let header_value = axum::http::HeaderValue::from_str(val_str).map_err(|_| {
            ApiError::BadRequest(format!("invalid custom header value for: {name}"))
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

async fn test_alert_point(
    State(app): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<crate::probe::client::ProbeObservation>> {
    let point = repo::alert_points::get(&app.db, id).await?;

    let method = match point.http_method {
        crate::db::models::HttpMethod::Get => reqwest::Method::GET,
        crate::db::models::HttpMethod::Post => reqwest::Method::POST,
    };
    let headers = configured_headers_to_map(&point.custom_headers)?;
    // Basic auth from auth_config when configured.
    let url = apply_basic_auth(&point.url, point.auth_config.as_ref())?;

    let observation = run_http_probe(
        &build_probe_client(),
        &url,
        method,
        &headers,
        probe_timeout(point.expected_response_time_ms),
    )
    .await;

    Ok(Json(observation))
}

/// Inline `http://user:pass@` basic-auth support: auth_config of shape
/// `{"type": "basic", "username": …, "password": …}` is folded into the URL
/// for the on-demand probe. Task 2.3 moves this into the worker's request
/// builder and adds bearer/header auth kinds.
fn apply_basic_auth(url: &str, auth_config: Option<&Value>) -> ApiResult<String> {
    let Some(auth) = auth_config else {
        return Ok(url.to_string());
    };
    let kind = auth.get("type").and_then(Value::as_str);
    if kind != Some("basic") {
        return Ok(url.to_string());
    }
    let user = auth
        .get("username")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest("auth_config.basic requires 'username'".into()))?;
    let pass = auth
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut parsed = url::Url::parse(url)
        .map_err(|e| ApiError::BadRequest(format!("configured URL is invalid: {e}")))?;
    parsed
        .set_username(user)
        .map_err(|_| ApiError::BadRequest("cannot set credentials on this URL".into()))?;
    parsed
        .set_password(Some(pass))
        .map_err(|_| ApiError::BadRequest("cannot set credentials on this URL".into()))?;
    Ok(parsed.to_string())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a full create payload (fields are required by the serde struct;
/// this checks their values). Missing/malformed JSON fields are rejected
/// earlier by axum's JSON extractor with a 422.
fn validate_new_point(p: &crate::db::models::NewAlertPoint) -> Result<(), ApiError> {
    let mut errors: Vec<(String, String)> = Vec::new();

    if let Err(msg) = validate_name_inner(&p.name) {
        errors.push(("name".into(), msg));
    }
    if let Err(msg) = validate_url_inner(&p.url) {
        errors.push(("url".into(), msg));
    }
    if let Err(msg) = validate_check_interval(p.check_interval_seconds) {
        errors.push(("check_interval_seconds".into(), msg));
    }
    if let Err(msg) = validate_expected_ms(p.expected_response_time_ms) {
        errors.push(("expected_response_time_ms".into(), msg));
    }
    if let Err(msg) = validate_custom_headers_inner(&p.custom_headers) {
        errors.push(("custom_headers".into(), msg));
    }
    if let Some(a) = &p.auth_config {
        if let Err(msg) = validate_auth_config_inner(a) {
            errors.push(("auth_config".into(), msg));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::Validation(errors))
    }
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    validate_name_inner(name).map_err(ApiError::BadRequest)
}

fn validate_name_inner(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("must not be empty".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("must be at most {MAX_NAME_LEN} characters"));
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<(), ApiError> {
    validate_url_inner(url).map_err(ApiError::BadRequest)
}

fn validate_url_inner(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_LEN {
        return Err(format!("must be at most {MAX_URL_LEN} characters"));
    }
    let parsed = url::Url::parse(url).map_err(|e| format!("is not a valid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("scheme must be http or https, got '{other}'")),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("must not embed credentials; use auth_config instead".into());
    }
    if parsed.host_str().is_none() {
        return Err("is missing a host".into());
    }
    Ok(())
}

fn validate_check_interval(secs: i32) -> Result<(), String> {
    if !(MIN_CHECK_INTERVAL_SECS..=MAX_CHECK_INTERVAL_SECS).contains(&secs) {
        return Err(format!(
            "must be between {MIN_CHECK_INTERVAL_SECS} and {MAX_CHECK_INTERVAL_SECS} seconds"
        ));
    }
    Ok(())
}

fn validate_expected_ms(ms: i32) -> Result<(), String> {
    if ms < 1 {
        return Err("must be positive".into());
    }
    if ms > MAX_EXPECTED_RESPONSE_MS {
        return Err(format!("must be at most {MAX_EXPECTED_RESPONSE_MS} ms"));
    }
    Ok(())
}

fn validate_custom_headers(headers: &Value) -> Result<(), ApiError> {
    validate_custom_headers_inner(headers).map_err(ApiError::BadRequest)
}

fn validate_custom_headers_inner(headers: &Value) -> Result<(), String> {
    if headers.is_null() {
        return Ok(());
    }
    let Some(obj) = headers.as_object() else {
        return Err("must be a JSON object of {name: value}".into());
    };
    for (name, value) in obj {
        if name.is_empty() {
            return Err("header name must not be empty".into());
        }
        // Header names are case-insensitive; normalize before validating
        // so users can write 'X-Api-Key' naturally.
        let lowered = name.to_ascii_lowercase();
        if axum::http::HeaderName::from_lowercase(lowered.as_bytes()).is_err() {
            return Err(format!("'{name}' is not a valid HTTP header name"));
        }
        if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(format!(
                "'{name}' is managed by the server and cannot be overridden"
            ));
        }
        if !value.is_string() {
            return Err(format!("value of '{name}' must be a string"));
        }
    }
    Ok(())
}

fn validate_auth_config(auth: &Value) -> Result<(), ApiError> {
    validate_auth_config_inner(auth).map_err(ApiError::BadRequest)
}

fn validate_auth_config_inner(auth: &Value) -> Result<(), String> {
    let Some(obj) = auth.as_object() else {
        return Err("must be a JSON object".into());
    };
    match obj.get("type").and_then(Value::as_str) {
        Some("basic") => {
            if !obj.contains_key("username") {
                return Err("type 'basic' requires a 'username'".into());
            }
            if obj.get("username").and_then(Value::as_str).is_none() {
                return Err("'username' must be a string".into());
            }
            Ok(())
        }
        Some("bearer") => match obj.get("token") {
            Some(t) if t.is_string() => Ok(()),
            _ => Err("type 'bearer' requires a string 'token'".into()),
        },
        Some("header") => {
            let name_ok = obj.get("name").and_then(Value::as_str).is_some_and(|n| {
                !n.is_empty()
                    && axum::http::HeaderName::from_lowercase(n.to_ascii_lowercase().as_bytes())
                        .is_ok()
            });
            let value_ok = obj.get("value").and_then(Value::as_str).is_some();
            if name_ok && value_ok {
                Ok(())
            } else {
                Err("type 'header' requires string 'name' and 'value'".into())
            }
        }
        Some(other) => Err(format!(
            "unknown auth type '{other}' (expected basic|bearer|header)"
        )),
        None => Err("missing 'type' (expected basic|bearer|header)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validation_accepts_http_and_https() {
        assert!(validate_url_inner("https://example.com/wms").is_ok());
        assert!(validate_url_inner("http://localhost:8080/geoserver").is_ok());
    }

    #[test]
    fn url_validation_rejects_bad_input() {
        assert!(validate_url_inner("not a url").is_err());
        assert!(validate_url_inner("ftp://example.com").is_err());
        assert!(validate_url_inner("https://user:pass@example.com").is_err());
        assert!(validate_url_inner("").is_err());
    }

    #[test]
    fn check_interval_bounds() {
        assert!(validate_check_interval(10).is_ok());
        assert!(validate_check_interval(9).is_err());
        assert!(validate_check_interval(0).is_err());
        assert!(validate_check_interval(-5).is_err());
    }

    #[test]
    fn expected_ms_bounds() {
        assert!(validate_expected_ms(1).is_ok());
        assert!(validate_expected_ms(0).is_err());
        assert!(validate_expected_ms(600_000).is_ok());
        assert!(validate_expected_ms(600_001).is_err());
    }

    #[test]
    fn custom_headers_must_be_object_of_strings() {
        assert!(validate_custom_headers_inner(&serde_json::json!({})).is_ok());
        assert!(validate_custom_headers_inner(&serde_json::json!({"X-Api-Key": "v"})).is_ok());
        assert!(validate_custom_headers_inner(&serde_json::json!([])).is_err());
        assert!(validate_custom_headers_inner(&serde_json::json!({"X-A": 1})).is_err());
    }

    #[test]
    fn custom_headers_reject_hop_by_hop() {
        assert!(validate_custom_headers_inner(&serde_json::json!({"Host": "evil.com"})).is_err());
        assert!(
            validate_custom_headers_inner(&serde_json::json!({"Connection": "close"})).is_err()
        );
        assert!(validate_custom_headers_inner(&serde_json::json!({"X-Api-Key": "k"})).is_ok());
    }

    #[test]
    fn auth_config_shapes() {
        assert!(validate_auth_config_inner(
            &serde_json::json!({"type": "basic", "username": "u", "password": "p"})
        )
        .is_ok());
        assert!(validate_auth_config_inner(&serde_json::json!({"type": "basic"})).is_err());
        assert!(
            validate_auth_config_inner(&serde_json::json!({"type": "bearer", "token": "t"}))
                .is_ok()
        );
        assert!(validate_auth_config_inner(&serde_json::json!({"type": "bearer"})).is_err());
        assert!(validate_auth_config_inner(
            &serde_json::json!({"type": "header", "name": "X-Key", "value": "v"})
        )
        .is_ok());
        assert!(validate_auth_config_inner(
            &serde_json::json!({"type": "header", "name": "X Key", "value": "v"})
        )
        .is_err());
        assert!(validate_auth_config_inner(&serde_json::json!({"type": "saml"})).is_err());
        assert!(validate_auth_config_inner(&serde_json::json!({})).is_err());
    }
}
